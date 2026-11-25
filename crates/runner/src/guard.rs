//! Guardián de un proceso supervisado (HU-024): que ningún agente siga trabajando si la app
//! muere de golpe (fallo, `kill -9`).
//!
//! En Unix el runner no lanza el programa directamente, sino el ejecutable de la app con
//! [`GUARD_FLAG`]:
//!
//! ```text
//! <app> --kanbania-guard <pid de la app> <gracia en ms> -- <programa> <argumentos…>
//! ```
//!
//! El guardián encabeza el grupo de procesos del intento, lanza el programa (que hereda el
//! grupo, el entorno, la carpeta y la entrada/salida) y espera. Si el programa termina, el
//! guardián termina con el mismo código (o la misma señal). Si la app deja de ser su padre (al
//! morir, el sistema le asigna otro), envía `SIGTERM` al grupo, espera la gracia y envía
//! `SIGKILL` al grupo, que lo incluye a él.
//!
//! Nunca se mata un proceso ajeno: el guardián solo envía señales a su propio grupo, cuyo
//! identificador es su propio pid, y lo hace estando vivo; un pid no se reutiliza mientras su
//! proceso exista, así que no hay ningún pid guardado que pueda haber pasado a otro proceso. Si
//! no consigue encabezar su grupo, no lanza nada.
//!
//! Sin cubrir: un descendiente que cree su propia sesión o grupo (`setsid`, doble `fork` con
//! `setpgid`) escapa, igual que de la cancelación normal; si el guardián muere a la vez que la
//! app (p. ej. `kill -9` a todo el árbol de la app), o si un proceso del grupo lo detiene
//! (`SIGSTOP`), nadie termina el grupo; si el programa termina y deja descendientes, el
//! guardián sale con él y, hasta que el runner termina el grupo (enseguida), una caída de la
//! app los dejaría vivos. El git de la app y «Comprobar adaptador» no usan guardián.
//! La bandera permite a cualquier proceso del usuario lanzar un programa con el ejecutable de
//! la app (no da privilegios que ese usuario no tenga; con TCC de macOS el hijo podría heredar
//! permisos concedidos a la app, como ya les ocurre a los adaptadores). Windows no usa el
//! guardián: el Job Object con «kill on close» termina el árbol cuando la app muere.
//!
//! Lo atienden la app (`main.rs`) y el binario `kanbania-test-process` antes de cualquier otra
//! cosa; no usa tokio ni abre datos.

use std::ffi::OsString;
use std::time::Duration;

/// Primer argumento que reserva la app para ejecutar el guardián.
pub const GUARD_FLAG: &str = "--kanbania-guard";
/// Cada cuánto comprueba el guardián que la app sigue siendo su padre.
pub const PARENT_POLL: Duration = Duration::from_millis(100);
/// Tras terminar el programa, cuánto sigue vigilando el guardián a la app antes de salir. Al
/// morir la app se cierran sus tuberías y el programa puede terminar por ello **antes** de que
/// el sistema reasigne el padre del guardián; si la app muere, los descendientes que queden en
/// el grupo se terminan igualmente.
pub const EXIT_LINGER: Duration = Duration::from_millis(300);
#[cfg(unix)]
const LINGER_POLL: Duration = Duration::from_millis(20);
/// Gracia máxima que acepta (la del runner es de segundos).
const MAX_GRACE_MS: u64 = 60_000;

/// Códigos de salida propios (los del shell para «no encontrado» y «no ejecutable»).
pub const EXIT_GUARD_ERROR: i32 = 125;
pub const EXIT_NOT_EXECUTABLE: i32 = 126;
pub const EXIT_NOT_FOUND: i32 = 127;

/// Cómo lanzar el guardián: el ejecutable de la app y los argumentos antes del pid
/// (por defecto, [`GUARD_FLAG`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardLauncher {
    pub program: std::path::PathBuf,
    pub args: Vec<OsString>,
}

impl GuardLauncher {
    pub fn new(program: std::path::PathBuf) -> Self {
        Self {
            program,
            args: vec![GUARD_FLAG.into()],
        }
    }

    /// Argumentos completos para envolver `program args` con este proceso como padre.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn wrap(
        &self,
        grace: Duration,
        program: &std::path::Path,
        args: &[OsString],
    ) -> Vec<OsString> {
        let grace_ms = u64::try_from(grace.as_millis())
            .unwrap_or(MAX_GRACE_MS)
            .min(MAX_GRACE_MS);
        let mut wrapped = self.args.clone();
        wrapped.push(std::process::id().to_string().into());
        wrapped.push(grace_ms.to_string().into());
        wrapped.push("--".into());
        wrapped.push(program.as_os_str().to_owned());
        wrapped.extend(args.iter().cloned());
        wrapped
    }
}

/// Argumentos ya interpretados.
#[derive(Debug, PartialEq, Eq)]
struct Invocation {
    parent: u32,
    grace: Duration,
    program: OsString,
    args: Vec<OsString>,
}

fn parse(args: &[OsString]) -> Option<Invocation> {
    let args = match args.first() {
        Some(first) if first == GUARD_FLAG => &args[1..],
        _ => args,
    };
    let [parent, grace, separator, program, rest @ ..] = args else {
        return None;
    };
    let parent: u32 = parent.to_str()?.parse().ok()?;
    let grace: u64 = grace.to_str()?.parse().ok()?;
    // Programa siempre absoluto: uno relativo se buscaría en `PATH` o en el worktree.
    if parent <= 1
        || separator != "--"
        || grace > MAX_GRACE_MS
        || !std::path::Path::new(program).is_absolute()
    {
        return None;
    }
    Some(Invocation {
        parent,
        grace: Duration::from_millis(grace),
        program: program.clone(),
        args: rest.to_vec(),
    })
}

/// Ejecuta el guardián y devuelve el código de salida (si el programa termina por una señal,
/// el guardián termina por la misma y no vuelve).
pub fn run(args: &[OsString]) -> i32 {
    let Some(invocation) = parse(args) else {
        let _ = std::io::Write::write_all(
            &mut std::io::stderr(),
            "kanbania: argumentos del guardián no válidos\n".as_bytes(),
        );
        return EXIT_GUARD_ERROR;
    };
    imp::run(invocation)
}

#[cfg(unix)]
mod imp {
    use std::io::Write;
    use std::process::{Command, ExitStatus};
    use std::sync::mpsc;
    use std::time::Instant;

    use nix::sys::signal::{SigSet, Signal, killpg, raise};
    use nix::unistd::{Pid, getpgrp, getpid, setpgid};

    use super::{
        EXIT_GUARD_ERROR, EXIT_LINGER, EXIT_NOT_EXECUTABLE, EXIT_NOT_FOUND, Invocation,
        LINGER_POLL, PARENT_POLL,
    };

    pub(super) fn run(invocation: Invocation) -> i32 {
        // La app ya no es el padre: murió antes de que arrancara el guardián. No se lanza nada.
        if std::os::unix::process::parent_id() != invocation.parent {
            return EXIT_GUARD_ERROR;
        }
        // Encabezar el grupo (el runner ya lo hace al lanzar; aquí no depende de él). Si no es
        // posible, no se lanza nada: matar «el grupo» podría alcanzar procesos ajenos.
        let _ = setpgid(Pid::from_raw(0), Pid::from_raw(0));
        let own_group = getpid();
        if getpgrp() != own_group {
            let _ = writeln!(
                std::io::stderr(),
                "kanbania: el guardián no encabeza su grupo de procesos"
            );
            return EXIT_GUARD_ERROR;
        }

        let mut child = match Command::new(&invocation.program)
            .args(&invocation.args)
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                // Mensaje fijo, sin ruta, para el registro (`err`). Sin `eprintln!`: haría
                // pánico si stderr ya no existe.
                let (code, text) = match error.kind() {
                    std::io::ErrorKind::NotFound => (EXIT_NOT_FOUND, "no existe"),
                    std::io::ErrorKind::PermissionDenied => (EXIT_NOT_EXECUTABLE, "sin permiso"),
                    _ => (EXIT_GUARD_ERROR, "error del sistema"),
                };
                let _ = writeln!(
                    std::io::stderr(),
                    "kanbania: no se pudo ejecutar el programa ({text})"
                );
                return code;
            }
        };

        // Después de lanzar (el programa hereda la máscara si fuera antes): el `SIGTERM` que el
        // runner envía al grupo al cancelar es para el programa; el guardián sigue hasta que el
        // programa termine (o hasta el `SIGKILL`) para poder informar de cómo terminó.
        let mut sigterm = SigSet::empty();
        sigterm.add(Signal::SIGTERM);
        let _ = sigterm.thread_block();

        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(child.wait());
        });

        loop {
            match receiver.recv_timeout(PARENT_POLL) {
                Ok(Ok(status)) => {
                    if app_died_within(invocation.parent, EXIT_LINGER) {
                        stop_group(own_group, None, invocation.grace);
                        return EXIT_GUARD_ERROR;
                    }
                    return propagate(status, &sigterm);
                }
                Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    stop_group(own_group, Some(&receiver), invocation.grace);
                    return EXIT_GUARD_ERROR;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if std::os::unix::process::parent_id() != invocation.parent {
                stop_group(own_group, Some(&receiver), invocation.grace);
                return EXIT_GUARD_ERROR;
            }
        }
    }

    /// ¿Deja la app de ser el padre dentro de `limit`?
    fn app_died_within(parent: u32, limit: std::time::Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if std::os::unix::process::parent_id() != parent {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(LINGER_POLL);
        }
    }

    /// `SIGTERM` al grupo (el guardián lo tiene bloqueado), espera como mucho `grace` (a que el
    /// programa termine, si sigue vivo) y después `SIGKILL` al grupo entero, guardián incluido,
    /// para no dejar descendientes.
    fn stop_group(
        group: Pid,
        program: Option<&mpsc::Receiver<std::io::Result<ExitStatus>>>,
        grace: std::time::Duration,
    ) {
        let _ = killpg(group, Signal::SIGTERM);
        match program {
            Some(receiver) => {
                let _ = receiver.recv_timeout(grace);
            }
            None => std::thread::sleep(grace),
        }
        let _ = killpg(group, Signal::SIGKILL);
    }

    /// Termina como el programa: mismo código o misma señal. Solo se reproducen las señales que
    /// terminan sin volcado (las de fallo, como `SIGSEGV`, harían que macOS atribuyera un fallo
    /// a la app); el resto, y `SIGPIPE` (que Rust ignora), salen con 128 + señal.
    fn propagate(status: ExitStatus, blocked: &SigSet) -> i32 {
        use std::os::unix::process::ExitStatusExt;
        if let Some(code) = status.code() {
            return code;
        }
        if let Some(number) = status.signal() {
            if let Ok(
                signal @ (Signal::SIGHUP
                | Signal::SIGINT
                | Signal::SIGKILL
                | Signal::SIGTERM
                | Signal::SIGALRM
                | Signal::SIGUSR1
                | Signal::SIGUSR2),
            ) = Signal::try_from(number)
            {
                let _ = blocked.thread_unblock();
                let _ = raise(signal);
            }
            return 128 + number;
        }
        EXIT_GUARD_ERROR
    }
}

#[cfg(not(unix))]
mod imp {
    use super::{EXIT_GUARD_ERROR, Invocation};

    /// Windows usa el Job Object («kill on close»), no el guardián.
    pub(super) fn run(_invocation: Invocation) -> i32 {
        eprintln!("kanbania: el guardián solo existe en Unix");
        EXIT_GUARD_ERROR
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn arguments_are_parsed_strictly() {
        assert_eq!(
            parse(&os(&[
                GUARD_FLAG,
                "4242",
                "1500",
                "--",
                "/bin/prog",
                "-x",
                "--"
            ])),
            Some(Invocation {
                parent: 4242,
                grace: Duration::from_millis(1500),
                program: "/bin/prog".into(),
                args: os(&["-x", "--"]),
            })
        );
        for bad in [
            &[GUARD_FLAG, "4242", "1500", "/bin/prog"][..],
            &[GUARD_FLAG, "1", "1500", "--", "/bin/prog"],
            &[GUARD_FLAG, "0", "1500", "--", "/bin/prog"],
            &[GUARD_FLAG, "-5", "1500", "--", "/bin/prog"],
            &[GUARD_FLAG, "4242", "999999", "--", "/bin/prog"],
            &[GUARD_FLAG, "4242", "1500", "-", "/bin/prog"],
            &[GUARD_FLAG, "4242", "1500", "--", ""],
            &[GUARD_FLAG, "4242", "1500", "--", "prog"],
            &[GUARD_FLAG, "4242", "1500", "--", "./prog"],
            &[GUARD_FLAG],
        ] {
            assert_eq!(parse(&os(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn the_launcher_passes_this_process_as_the_parent() {
        let launcher = GuardLauncher::new("/app".into());
        let wrapped = launcher.wrap(
            Duration::from_secs(5),
            std::path::Path::new("/bin/prog"),
            &os(&["a"]),
        );
        let expected_pid = std::process::id().to_string();
        assert_eq!(
            wrapped,
            os(&[GUARD_FLAG, &expected_pid, "5000", "--", "/bin/prog", "a"])
        );
        let parsed = parse(&wrapped).unwrap();
        assert_eq!(parsed.parent, std::process::id());
    }
}
