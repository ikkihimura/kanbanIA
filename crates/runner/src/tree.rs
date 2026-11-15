//! Árbol de procesos de un hijo lanzado por el runner.
//!
//! - Unix: el hijo encabeza un grupo de procesos propio (`setpgid(0, 0)` vía `process-wrap`) y
//!   sus descendientes lo heredan. Terminar = `SIGTERM` al grupo, gracia, `SIGKILL` al grupo.
//! - Windows: el hijo nace dentro de un Job Object con «kill on close» (sus descendientes
//!   también). Terminar = `TerminateJobObject` (Windows no tiene un equivalente de `SIGTERM`
//!   para procesos de consola sin ventana: no hay gracia).
//!
//! Limitación conocida: un descendiente que cree su propio grupo o sesión (Unix) o salga del
//! Job (Windows, con `CREATE_BREAKAWAY_FROM_JOB` si el job lo permitiera; aquí no) escapa.

use std::io;
use std::process::ExitStatus;
use std::time::Duration;

use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;
#[cfg(unix)]
use tokio::time::{Instant, sleep};

/// Intervalo con el que se comprueba si quedan procesos en el grupo.
#[cfg(unix)]
const POLL: Duration = Duration::from_millis(50);
/// Límite para recoger al hijo tras `SIGKILL`/`TerminateJobObject`; no debería agotarse nunca.
const REAP_LIMIT: Duration = Duration::from_secs(5);

pub(crate) struct ProcessTree {
    child: Box<dyn ChildWrapper>,
    #[cfg(unix)]
    pgid: nix::unistd::Pid,
    leader_status: Option<ExitStatus>,
    /// `true` cuando ya no queda ningún proceso del árbol que matar.
    settled: bool,
}

impl ProcessTree {
    /// Lanza `command` (ya configurado: programa, argumentos, entorno, cwd y stdio) como raíz de
    /// un árbol de procesos propio.
    pub(crate) fn spawn(command: Command) -> io::Result<Self> {
        let mut wrapped = CommandWrap::from(command);
        // `KillOnDrop` mata al hijo directo si se suelta sin esperar; en Windows, además, hace
        // que cerrar el Job (al soltarlo) termine todo el árbol.
        wrapped.wrap(KillOnDrop);
        #[cfg(unix)]
        wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
        // Windows: sin ventana de consola (CREATE_NO_WINDOW). `CreationFlags` es la capa que
        // conserva estas banderas junto a `JobObject`, que añade CREATE_SUSPENDED, asigna el
        // proceso al Job y después lo reanuda (process-wrap 10.0.1).
        #[cfg(windows)]
        wrapped.wrap(process_wrap::tokio::CreationFlags(
            windows::Win32::System::Threading::CREATE_NO_WINDOW,
        ));
        #[cfg(windows)]
        wrapped.wrap(process_wrap::tokio::JobObject);
        let child = wrapped.spawn()?;
        #[cfg(unix)]
        let pgid = {
            let pid = child
                .id()
                .and_then(|pid| i32::try_from(pid).ok())
                .ok_or_else(|| io::Error::other("sin identificador de proceso"))?;
            nix::unistd::Pid::from_raw(pid)
        };
        Ok(Self {
            child,
            #[cfg(unix)]
            pgid,
            leader_status: None,
            settled: false,
        })
    }

    pub(crate) fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    pub(crate) fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin().take()
    }

    pub(crate) fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout().take()
    }

    pub(crate) fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr().take()
    }

    /// Espera solo al proceso raíz (no a sus descendientes). Cancelable: se puede soltar el
    /// futuro y volver a llamar.
    pub(crate) async fn wait_leader(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.leader_status {
            return Ok(status);
        }
        // La capa interna es el `Child` de tokio: su `wait` solo espera a la raíz. El `wait` del
        // Job Object (Windows) esperaría a todo el árbol, que puede no terminar nunca.
        let status = self.child.inner_mut().wait().await?;
        self.leader_status = Some(status);
        Ok(status)
    }

    /// Termina el árbol: aviso (`SIGTERM` al grupo en Unix), espera hasta `grace` y después
    /// fuerza (`SIGKILL` al grupo). También sirve después de que la raíz terminara por su
    /// cuenta, para no dejar descendientes huérfanos. Devuelve `true` si quedaba algún proceso
    /// del árbol.
    #[cfg(unix)]
    pub(crate) async fn terminate(&mut self, grace: Duration) -> bool {
        use nix::sys::signal::{Signal, killpg};
        if self.settled {
            return false;
        }
        let had_processes = self.leader_status.is_none() || self.group_alive();
        if had_processes {
            let _ = killpg(self.pgid, Signal::SIGTERM);
            self.wait_group_until(Instant::now() + grace).await;
            if self.leader_status.is_none() || self.group_alive() {
                let _ = killpg(self.pgid, Signal::SIGKILL);
                self.wait_group_until(Instant::now() + REAP_LIMIT).await;
            }
        }
        self.settled = self.leader_status.is_some() && !self.group_alive();
        had_processes
    }

    /// Windows: `TerminateJobObject` termina a la vez la raíz y todos sus descendientes (no hay
    /// un equivalente de `SIGTERM` para procesos de consola sin ventana: no hay gracia). Tras
    /// terminar la raíz por su cuenta se termina igualmente el Job; si ya está vacío no hace
    /// nada. No se sabe de forma barata si quedaban descendientes: solo se informa de la raíz.
    #[cfg(windows)]
    pub(crate) async fn terminate(&mut self, grace: Duration) -> bool {
        let _ = grace;
        if self.settled {
            return false;
        }
        let had_processes = self.leader_status.is_none();
        let _ = self.child.start_kill();
        if self.leader_status.is_none() {
            let _ = timeout(REAP_LIMIT, self.wait_leader()).await;
        }
        self.settled = self.leader_status.is_some();
        had_processes
    }

    /// Mata el árbol sin gracia (límite de tiempo de Git, por ejemplo).
    pub(crate) async fn kill(&mut self) {
        self.terminate(Duration::ZERO).await;
    }

    /// ¿Queda algún proceso en el grupo? `killpg(pgid, 0)` no envía nada. `EPERM` significa que
    /// el identificador ya pertenece a procesos de otro usuario: no son nuestros.
    #[cfg(unix)]
    fn group_alive(&self) -> bool {
        nix::sys::signal::killpg(self.pgid, None).is_ok()
    }

    /// Espera hasta `deadline` a que el grupo quede vacío, recogiendo a la raíz (que sigue
    /// contando como miembro, zombi, hasta que se recoge).
    #[cfg(unix)]
    async fn wait_group_until(&mut self, deadline: Instant) {
        loop {
            if self.leader_status.is_none() {
                let _ = timeout(POLL, self.wait_leader()).await;
            }
            if self.leader_status.is_some() && !self.group_alive() {
                return;
            }
            if Instant::now() >= deadline {
                return;
            }
            if self.leader_status.is_some() {
                sleep(POLL).await;
            }
        }
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        // Si nadie terminó el árbol (pánico, futuro cancelado…), no se deja vivo. En Windows lo
        // hace el cierre del Job («kill on close»).
        #[cfg(unix)]
        if !self.settled {
            let _ = nix::sys::signal::killpg(self.pgid, nix::sys::signal::Signal::SIGKILL);
        }
    }
}
