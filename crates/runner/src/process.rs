//! Proceso supervisado: programa + argumentos explícitos (nunca un shell), `cwd` indicado,
//! entorno vacío más una lista permitida, árbol de procesos propio, límite de tiempo y
//! cancelación.
//!
//! Dos modos:
//! - normal ([`RunningProcess::spawn`]): `stdin` nulo; stdout y stderr van línea a línea a un
//!   [`OutputLog`]; [`RunningProcess::wait`] aplica el tiempo máximo y la cancelación;
//! - protocolo ([`RunningProcess::spawn_protocol`], HU-019): stdin y stdout en tubería para
//!   quien habla el protocolo (ACP), que los recibe en [`ProtocolIo`] junto con un
//!   [`OutputSink`] para escribir en el mismo registro; stderr sigue yendo al registro (`err`),
//!   con una redacción mínima de correos y prefijos de secretos (`redact`).
//!   El tiempo máximo y la cancelación los aplica quien habla el protocolo (con
//!   [`RunningProcess::deadline`]), que termina con [`RunningProcess::close`]. El entorno
//!   añade las variables de adaptador; nunca `KANBANIA_*`, `ANTHROPIC_*` ni `CLAUDE_*`.
//!
//! Uso en dos fases, para que quien supervisa pueda registrar «en ejecución» justo después de
//! lanzar.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};

use crate::env::{adapter_extra_env, allowed_env, is_adapter_forbidden, valid_extra_name};
use crate::output::{AgentKind, MAX_LINE_BYTES, OutputLog, OutputStats, Stream};
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
    /// Modo protocolo: la raíz no terminó a tiempo tras cerrar el protocolo y el runner terminó
    /// el árbol ([`RunningProcess::close`]).
    Stopped,
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

/// Registro en vuelo de un lector (o de un [`OutputSink`]) al escritor.
struct Record {
    stream: Stream,
    kind: Option<AgentKind>,
    bytes: Vec<u8>,
    cut_bytes: u64,
}

impl std::fmt::Debug for Record {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Record")
    }
}

/// Escritura en el registro de un proceso en modo protocolo: líneas `agent` (normalizadas) y
/// `sys`. Clonable. No mantiene vivo el registro: cuando el proceso se cierra, lo que se envíe
/// se pierde sin error.
#[derive(Debug, Clone)]
pub struct OutputSink(mpsc::WeakSender<Record>);

impl OutputSink {
    /// Una línea `agent` (se recorta a [`MAX_LINE_BYTES`] y se marca `cut`).
    pub async fn agent(&self, kind: AgentKind, text: &str) {
        self.send(Stream::Agent, Some(kind), text).await;
    }

    /// Mensaje del runner: una línea `sys` por línea de texto.
    pub async fn system(&self, text: &str) {
        for line in text.lines() {
            self.send(Stream::System, None, line).await;
        }
    }

    async fn send(&self, stream: Stream, kind: Option<AgentKind>, text: &str) {
        if let Some(sender) = self.0.upgrade() {
            let _ = sender
                .send(Record {
                    stream,
                    kind,
                    bytes: text.as_bytes().to_vec(),
                    cut_bytes: 0,
                })
                .await;
        }
    }
}

/// Tuberías del protocolo de un proceso lanzado en modo protocolo. Soltar `stdin` cierra la
/// entrada del proceso.
#[derive(Debug)]
pub struct ProtocolIo {
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
    pub sink: OutputSink,
}

/// Proceso lanzado, aún sin esperar.
pub struct RunningProcess {
    tree: ProcessTree,
    readers: Vec<JoinHandle<()>>,
    writer: JoinHandle<Option<OutputLog>>,
    /// Mantiene vivo el escritor mientras haya proceso (los [`OutputSink`] son débiles).
    keepalive: Option<mpsc::Sender<Record>>,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Logged,
    Protocol,
}

impl RunningProcess {
    /// Valida y lanza. Requiere un runtime de tokio. Si falla, devuelve el registro para que
    /// quien llama anote el detalle.
    pub fn spawn(spec: ProcessSpec, mut log: OutputLog) -> Result<Self, Box<SpawnFailure>> {
        match launch(&spec, Mode::Logged) {
            Ok(mut tree) => {
                let stdout = tree.take_stdout();
                Ok(Self::start(tree, stdout, Some(log), &spec, false).0)
            }
            Err(error) => {
                if error.kind != SpawnErrorKind::InvalidSpec {
                    log.write_system("No se pudo iniciar el proceso.");
                    let _ = log.flush();
                }
                Err(Box::new(SpawnFailure { error, log }))
            }
        }
    }

    /// Modo protocolo (HU-019): como [`RunningProcess::spawn`], pero stdin y stdout quedan en
    /// tubería para quien habla el protocolo y el entorno añade las variables de adaptador.
    /// stderr va al registro (`err`). El tiempo máximo y la cancelación los aplica quien llama
    /// (ver [`RunningProcess::deadline`] y [`RunningProcess::close`]).
    pub fn spawn_protocol(
        spec: ProcessSpec,
        mut log: OutputLog,
    ) -> Result<(Self, ProtocolIo), Box<SpawnFailure>> {
        match launch_protocol(&spec) {
            Ok((tree, stdin, stdout)) => {
                let (process, sink) = Self::start(tree, None, Some(log), &spec, true);
                Ok((
                    process,
                    ProtocolIo {
                        stdin,
                        stdout,
                        sink,
                    },
                ))
            }
            Err(error) => {
                if error.kind != SpawnErrorKind::InvalidSpec {
                    log.write_system("No se pudo iniciar el proceso.");
                    let _ = log.flush();
                }
                Err(Box::new(SpawnFailure { error, log }))
            }
        }
    }

    /// Modo protocolo sin registro (p. ej. «Comprobar adaptador»): stderr se lee y se descarta,
    /// y lo que se escriba en el [`OutputSink`] se pierde.
    pub fn spawn_protocol_unlogged(spec: ProcessSpec) -> Result<(Self, ProtocolIo), SpawnError> {
        let (tree, stdin, stdout) = launch_protocol(&spec)?;
        let (process, sink) = Self::start(tree, None, None, &spec, true);
        Ok((
            process,
            ProtocolIo {
                stdin,
                stdout,
                sink,
            },
        ))
    }

    /// Arranca los lectores (stdout si se indica, y stderr) y el escritor de un árbol lanzado.
    fn start(
        mut tree: ProcessTree,
        stdout: Option<ChildStdout>,
        log: Option<OutputLog>,
        spec: &ProcessSpec,
        redact_stderr: bool,
    ) -> (Self, OutputSink) {
        let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
        let mut readers = Vec::with_capacity(2);
        if let Some(stdout) = stdout {
            readers.push(tokio::spawn(pump(
                stdout,
                Stream::Stdout,
                sender.clone(),
                false,
            )));
        }
        if let Some(stderr) = tree.take_stderr() {
            readers.push(tokio::spawn(pump(
                stderr,
                Stream::Stderr,
                sender.clone(),
                redact_stderr,
            )));
        }
        let sink = OutputSink(sender.downgrade());
        let writer = tokio::task::spawn_blocking(move || write_records(log, receiver));
        (
            Self {
                tree,
                readers,
                writer,
                keepalive: Some(sender),
                timeout: spec.timeout,
                grace: spec.grace,
                started: Instant::now(),
            },
            sink,
        )
    }

    pub fn pid(&self) -> Option<u32> {
        self.tree.pid()
    }

    /// Hora límite del tiempo máximo (lanzamiento + `timeout`).
    pub fn deadline(&self) -> Instant {
        self.started + self.timeout
    }

    /// Espera a que termine, se agote el tiempo o se cancele; en los dos últimos casos termina
    /// el árbol (aviso, gracia, fuerza). Tras una salida normal también termina los
    /// descendientes que sigan vivos. Devuelve el informe y el registro (`None` si el hilo de
    /// escritura falló), para anotar el cierre.
    pub async fn wait(mut self, mut cancel: CancelToken) -> (ProcessReport, Option<OutputLog>) {
        let deadline = self.deadline();
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
        self.finish(outcome, grace).await
    }

    /// Modo protocolo: tras cerrar el protocolo (quien llama ya soltó `stdin`), espera como
    /// mucho `wait_exit` a que la raíz termine sola y después termina el árbol (aviso, gracia,
    /// fuerza), también los descendientes que sigan vivos. Con `wait_exit` cero, termina en
    /// seguida.
    pub async fn close(mut self, wait_exit: Duration) -> (ProcessReport, Option<OutputLog>) {
        let (outcome, grace) = match timeout(wait_exit, self.tree.wait_leader()).await {
            Ok(Ok(status)) => (outcome_of(status), self.grace),
            Ok(Err(_)) => (ProcessOutcome::WaitFailed, Duration::ZERO),
            Err(_) => (ProcessOutcome::Stopped, self.grace),
        };
        self.finish(outcome, grace).await
    }

    async fn finish(
        mut self,
        outcome: ProcessOutcome,
        grace: Duration,
    ) -> (ProcessReport, Option<OutputLog>) {
        let tree_terminated = self.tree.terminate(grace).await;

        // Vaciar tuberías: con el árbol muerto llegan al final enseguida.
        for mut reader in std::mem::take(&mut self.readers) {
            if timeout(DRAIN_LIMIT, &mut reader).await.is_err() {
                reader.abort();
            }
        }
        // Sin esta copia, el escritor termina en cuanto acaban los lectores (los `OutputSink`
        // son débiles).
        self.keepalive = None;
        let log = (&mut self.writer).await.ok().flatten();
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

/// Valida y lanza el árbol.
fn launch(spec: &ProcessSpec, mode: Mode) -> Result<ProcessTree, SpawnError> {
    validate(spec, mode).map_err(|detail| SpawnError {
        kind: SpawnErrorKind::InvalidSpec,
        detail,
    })?;
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(allowed_env());
    if mode == Mode::Protocol {
        command.envs(adapter_extra_env());
    }
    command
        .envs(spec.extra_env.iter().map(|(k, v)| (k, v)))
        .stdin(match mode {
            Mode::Logged => Stdio::null(),
            Mode::Protocol => Stdio::piped(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    ProcessTree::spawn(command).map_err(|error| SpawnError {
        kind: match error.kind() {
            std::io::ErrorKind::NotFound => SpawnErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied => SpawnErrorKind::PermissionDenied,
            _ => SpawnErrorKind::Other,
        },
        detail: error.to_string(),
    })
}

/// Lanza en modo protocolo y toma stdin y stdout.
fn launch_protocol(
    spec: &ProcessSpec,
) -> Result<(ProcessTree, ChildStdin, ChildStdout), SpawnError> {
    let mut tree = launch(spec, Mode::Protocol)?;
    match (tree.take_stdin(), tree.take_stdout()) {
        (Some(stdin), Some(stdout)) => Ok((tree, stdin, stdout)),
        // Soltar el árbol mata el proceso.
        _ => Err(SpawnError {
            kind: SpawnErrorKind::Other,
            detail: "sin tuberías del protocolo".into(),
        }),
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

fn validate(spec: &ProcessSpec, mode: Mode) -> Result<(), String> {
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
    if mode == Mode::Protocol
        && let Some((name, _)) = spec
            .extra_env
            .iter()
            .find(|(name, _)| is_adapter_forbidden(name))
    {
        return Err(format!("variable de entorno no permitida: {name:?}"));
    }
    Ok(())
}

/// Lee una tubería y envía líneas de como mucho [`MAX_LINE_BYTES`] bytes; el exceso de cada
/// línea se descarta (contado) sin acumularlo. Un `\r` final se quita.
async fn pump(
    mut stream: impl AsyncRead + Unpin,
    kind: Stream,
    sender: mpsc::Sender<Record>,
    redact: bool,
) {
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
                    kind: None,
                    bytes: maybe_redact(std::mem::take(&mut line), redact),
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
                kind: None,
                bytes: maybe_redact(line, redact),
                cut_bytes,
            })
            .await;
    }
}

fn maybe_redact(line: Vec<u8>, redact: bool) -> Vec<u8> {
    if redact {
        crate::redact::redact_line(&line)
    } else {
        line
    }
}

/// Hilo de escritura: escribe cada registro y vuelca al disco cuando no queda nada en cola. Sin
/// registro, los descarta.
fn write_records(
    mut log: Option<OutputLog>,
    mut receiver: mpsc::Receiver<Record>,
) -> Option<OutputLog> {
    while let Some(record) = receiver.blocking_recv() {
        let Some(log) = log.as_mut() else { continue };
        log.write_record(record.stream, record.kind, &record.bytes, record.cut_bytes);
        if receiver.is_empty() {
            let _ = log.flush();
        }
    }
    if let Some(log) = log.as_mut() {
        let _ = log.flush();
    }
    log
}
