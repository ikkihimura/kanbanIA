//! Proceso supervisado: programa + argumentos explícitos (nunca un shell), `cwd` indicado,
//! `stdin` nulo, entorno vacío más una lista permitida, árbol de procesos propio, límite de
//! tiempo y cancelación. stdout y stderr van línea a línea a un [`OutputLog`].
//!
//! Uso en dos fases, para que quien supervisa pueda registrar «en ejecución» justo después de
//! lanzar: [`RunningProcess::spawn`] y luego [`RunningProcess::wait`].

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};

use crate::env::{allowed_env, valid_extra_name};
use crate::output::{MAX_LINE_BYTES, OutputLog, OutputStats, Stream};
use crate::tree::ProcessTree;

/// Gracia por defecto entre `SIGTERM` y `SIGKILL` (Unix).
pub const DEFAULT_GRACE: Duration = Duration::from_secs(5);
/// Lo que se espera a que los lectores vacíen las tuberías tras morir el árbol. Si un proceso
/// escapó del grupo y conserva la tubería, se deja de leer.
const DRAIN_LIMIT: Duration = Duration::from_secs(2);
/// Registros en vuelo entre los lectores y el escritor (contrapresión: si el disco va lento,
/// el proceso se bloquea al escribir, no crece la memoria).
const CHANNEL_CAPACITY: usize = 256;
const READ_CHUNK: usize = 8 * 1024;

/// Qué lanzar y con qué límites.
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    /// Ruta absoluta del ejecutable (no se busca en `PATH`).
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Carpeta de trabajo: absoluta y existente.
    pub cwd: PathBuf,
    /// Variables además de las permitidas. Nunca `KANBANIA_*`.
    pub extra_env: Vec<(OsString, OsString)>,
    /// Tiempo máximo desde el lanzamiento. Debe ser mayor que cero.
    pub timeout: Duration,
    /// Gracia entre el aviso y la terminación forzosa (Unix; en Windows no hay aviso).
    pub grace: Duration,
}

/// Cómo terminó.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessOutcome {
    /// La raíz terminó por su cuenta con este código (0 = éxito).
    Exited { code: i32 },
    /// La raíz terminó por una señal que no envió el runner (Unix).
    Signaled { signal: i32 },
    /// Se agotó el tiempo y se terminó el árbol.
    TimedOut,
    /// Se canceló y se terminó el árbol.
    Cancelled,
    /// No se pudo esperar al proceso (error del sistema); el árbol se terminó.
    WaitFailed,
}

/// Resultado de un proceso que llegó a lanzarse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessReport {
    pub outcome: ProcessOutcome,
    pub output: OutputStats,
    /// `true` si al terminar quedaban procesos del árbol y se mataron (descendientes huérfanos
    /// tras una salida normal, o el árbol entero al cancelar o agotar el tiempo).
    pub tree_terminated: bool,
    pub elapsed: Duration,
}

/// Por qué no se lanzó.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnErrorKind {
    /// Programa no absoluto, `cwd` inválido, tiempo cero, variable reservada…
    InvalidSpec,
    NotFound,
    PermissionDenied,
    Other,
}

/// Error al lanzar. `Display` es un mensaje seguro (sin rutas); `detail` es para el log.
#[derive(Debug, Clone)]
pub struct SpawnError {
    pub kind: SpawnErrorKind,
    pub detail: String,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.kind {
            SpawnErrorKind::InvalidSpec => "La configuración del proceso no es válida.",
            SpawnErrorKind::NotFound => "No se encontró el programa que había que ejecutar.",
            SpawnErrorKind::PermissionDenied => "No hay permiso para ejecutar el programa.",
            SpawnErrorKind::Other => "No se pudo iniciar el proceso.",
        })
    }
}

impl std::error::Error for SpawnError {}

/// Error al lanzar + el registro de salida, que se devuelve para poder anotar el detalle.
#[derive(Debug)]
pub struct SpawnFailure {
    pub error: SpawnError,
    pub log: OutputLog,
}

/// Lado que cancela. Clonable; cancelar varias veces no hace nada más.
#[derive(Debug, Clone)]
pub struct CancelHandle(watch::Sender<bool>);

/// Lado que escucha. Si se sueltan todos los [`CancelHandle`], ya no se puede cancelar (el
/// límite de tiempo sigue aplicándose).
#[derive(Debug, Clone)]
pub struct CancelToken(watch::Receiver<bool>);

pub fn cancel_pair() -> (CancelHandle, CancelToken) {
    let (sender, receiver) = watch::channel(false);
    (CancelHandle(sender), CancelToken(receiver))
}

impl CancelHandle {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
}

impl CancelToken {
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    pub(crate) async fn cancelled(&mut self) {
        if self.0.wait_for(|cancelled| *cancelled).await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Registro en vuelo de un lector al escritor.
struct Record {
    stream: Stream,
    bytes: Vec<u8>,
    cut_bytes: u64,
}

/// Proceso lanzado, aún sin esperar.
pub struct RunningProcess {
    tree: ProcessTree,
    readers: Vec<JoinHandle<()>>,
    writer: JoinHandle<OutputLog>,
    timeout: Duration,
    grace: Duration,
    started: Instant,
}

impl std::fmt::Debug for RunningProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningProcess")
            .field("pid", &self.tree.pid())
            .finish_non_exhaustive()
    }
}

impl RunningProcess {
    /// Valida y lanza. Requiere un runtime de tokio. Si falla, devuelve el registro para que
    /// quien llama anote el detalle.
    pub fn spawn(spec: ProcessSpec, mut log: OutputLog) -> Result<Self, Box<SpawnFailure>> {
        if let Err(detail) = validate(&spec) {
            return Err(Box::new(SpawnFailure {
                error: SpawnError {
                    kind: SpawnErrorKind::InvalidSpec,
                    detail,
                },
                log,
            }));
        }
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .env_clear()
            .envs(allowed_env())
            .envs(spec.extra_env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut tree = match ProcessTree::spawn(command) {
            Ok(tree) => tree,
            Err(error) => {
                let kind = match error.kind() {
                    std::io::ErrorKind::NotFound => SpawnErrorKind::NotFound,
                    std::io::ErrorKind::PermissionDenied => SpawnErrorKind::PermissionDenied,
                    _ => SpawnErrorKind::Other,
                };
                log.write_system("No se pudo iniciar el proceso.");
                let _ = log.flush();
                return Err(Box::new(SpawnFailure {
                    error: SpawnError {
                        kind,
                        detail: error.to_string(),
                    },
                    log,
                }));
            }
        };
        let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
        let mut readers = Vec::with_capacity(2);
        if let Some(stdout) = tree.take_stdout() {
            readers.push(tokio::spawn(pump(stdout, Stream::Stdout, sender.clone())));
        }
        if let Some(stderr) = tree.take_stderr() {
            readers.push(tokio::spawn(pump(stderr, Stream::Stderr, sender.clone())));
        }
        drop(sender);
        let writer = tokio::task::spawn_blocking(move || write_records(log, receiver));
        Ok(Self {
            tree,
            readers,
            writer,
            timeout: spec.timeout,
            grace: spec.grace,
            started: Instant::now(),
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.tree.pid()
    }

    /// Espera a que termine, se agote el tiempo o se cancele; en los dos últimos casos termina
    /// el árbol (aviso, gracia, fuerza). Tras una salida normal también termina los
    /// descendientes que sigan vivos. Devuelve el informe y el registro (`None` si el hilo de
    /// escritura falló), para anotar el cierre.
    pub async fn wait(mut self, mut cancel: CancelToken) -> (ProcessReport, Option<OutputLog>) {
        let deadline = self.started + self.timeout;
        let ended = tokio::select! {
            status = self.tree.wait_leader() => End::Exited(status),
            () = tokio::time::sleep_until(deadline) => End::TimedOut,
            () = cancel.cancelled() => End::Cancelled,
        };
        let (outcome, grace) = match ended {
            End::Exited(Ok(status)) => (outcome_of(status), self.grace),
            End::Exited(Err(_)) => (ProcessOutcome::WaitFailed, Duration::ZERO),
            End::TimedOut => (ProcessOutcome::TimedOut, self.grace),
            End::Cancelled => (ProcessOutcome::Cancelled, self.grace),
        };
        let tree_terminated = self.tree.terminate(grace).await;

        // Vaciar tuberías: con el árbol muerto llegan al final enseguida.
        for mut reader in self.readers {
            if timeout(DRAIN_LIMIT, &mut reader).await.is_err() {
                reader.abort();
            }
        }
        let log = self.writer.await.ok();
        let output = log.as_ref().map(OutputLog::stats).unwrap_or_default();
        let report = ProcessReport {
            outcome,
            output,
            tree_terminated,
            elapsed: self.started.elapsed(),
        };
        (report, log)
    }
}

enum End {
    Exited(std::io::Result<ExitStatus>),
    TimedOut,
    Cancelled,
}

fn outcome_of(status: ExitStatus) -> ProcessOutcome {
    if let Some(code) = status.code() {
        return ProcessOutcome::Exited { code };
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return ProcessOutcome::Signaled { signal };
        }
    }
    ProcessOutcome::WaitFailed
}

fn validate(spec: &ProcessSpec) -> Result<(), String> {
    if !spec.program.is_absolute() {
        return Err("el programa debe ser una ruta absoluta".into());
    }
    if !spec.cwd.is_absolute() || !spec.cwd.is_dir() {
        return Err("la carpeta de trabajo debe ser absoluta y existir".into());
    }
    if spec.timeout.is_zero() {
        return Err("el tiempo máximo debe ser mayor que cero".into());
    }
    if let Some((name, _)) = spec
        .extra_env
        .iter()
        .find(|(name, _)| !valid_extra_name(name))
    {
        return Err(format!("variable de entorno no permitida: {name:?}"));
    }
    Ok(())
}

/// Lee una tubería y envía líneas de como mucho [`MAX_LINE_BYTES`] bytes; el exceso de cada
/// línea se descarta (contado) sin acumularlo. Un `\r` final se quita.
async fn pump(mut stream: impl AsyncRead + Unpin, kind: Stream, sender: mpsc::Sender<Record>) {
    let mut chunk = vec![0u8; READ_CHUNK];
    let mut line: Vec<u8> = Vec::with_capacity(256);
    let mut cut_bytes: u64 = 0;
    let mut pending = false;
    loop {
        let read = match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let mut rest = &chunk[..read];
        while !rest.is_empty() {
            pending = true;
            let (segment, newline) = match rest.iter().position(|b| *b == b'\n') {
                Some(at) => (&rest[..at], true),
                None => (rest, false),
            };
            rest = if newline {
                &rest[segment.len() + 1..]
            } else {
                &[]
            };
            let room = MAX_LINE_BYTES.saturating_sub(line.len());
            let keep = segment.len().min(room);
            line.extend_from_slice(&segment[..keep]);
            cut_bytes += (segment.len() - keep) as u64;
            if newline {
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                let record = Record {
                    stream: kind,
                    bytes: std::mem::take(&mut line),
                    cut_bytes: std::mem::take(&mut cut_bytes),
                };
                pending = false;
                if sender.send(record).await.is_err() {
                    return;
                }
            }
        }
    }
    if pending {
        let _ = sender
            .send(Record {
                stream: kind,
                bytes: line,
                cut_bytes,
            })
            .await;
    }
}

/// Hilo de escritura: escribe cada registro y vuelca al disco cuando no queda nada en cola.
fn write_records(mut log: OutputLog, mut receiver: mpsc::Receiver<Record>) -> OutputLog {
    while let Some(record) = receiver.blocking_recv() {
        log.write_line(record.stream, &record.bytes, record.cut_bytes);
        if receiver.is_empty() {
            let _ = log.flush();
        }
    }
    let _ = log.flush();
    log
}
