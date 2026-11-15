//! Supervisor de ejecuciones (HU-018): orquesta el núcleo (`runs::supervisor`, protegido por
//! `SupervisorToken`) y los procesos de este crate.
//!
//! - Arranque ([`Supervisor::start`]): marca `interrupted` los intentos activos de una sesión
//!   anterior y anula las peticiones de inicio pendientes. Nada se relanza solo.
//! - Bucle: se despierta con [`SupervisorWaker::wake`] o cada `poll_interval`. En cada vuelta
//!   aplica las peticiones de cancelación y despacha (FIFO) mientras haya hueco según
//!   `max_concurrency`, leído en cada vuelta (bajarlo no detiene los que corren).
//! - Por intento: `begin_dispatch` → `record_worktree` (antes de tocar el disco: si la app muere
//!   en medio, el worktree nunca queda sin registrar) → `create_worktree` → lanzar el proceso de
//!   prueba (sin IA) → `mark_running` → esperar → `finish_run`.
//! - Ante un error del núcleo no se reintenta a ciegas: se registra y la siguiente vuelta relee
//!   el estado. Un error no detiene el bucle.
//! - Apagado ([`Supervisor::shutdown`]): cancela todos los árboles, espera, marca `interrupted`
//!   los intentos activos («Se cerró la app.») y suelta el token.
//!
//! Los motivos que ve la persona los genera el núcleo; los detalles (rutas, salida de Git) van
//! solo al registro JSONL del intento (`sys`). En stderr nunca hay rutas ni salida de procesos.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kanbania_core::error::CoreError;
use kanbania_core::features::runs::{
    self, Dispatch, DispatchOutcome, InterruptCause, RUN_OUTPUT_DIR, RunChange, RunFailure,
    RunOutcome, RunOutputLine, RunOutputStream, RunOutputView, RunSummary, SupervisorToken,
    WORKTREES_DIR,
};
use kanbania_core::platform::storage::Store;
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;

use crate::output::{MAX_TAIL_LINES, OutputLog, Stream, read_tail};
use crate::process::{
    CancelHandle, CancelToken, DEFAULT_GRACE, ProcessOutcome, ProcessSpec, RunningProcess,
    cancel_pair,
};
use crate::test_process::RESERVED_FLAG;
use crate::worktree::{
    CreateWorktree, DEFAULT_GIT_TIMEOUT, GitConfig, WorktreeError, WorktreeErrorKind,
    create_worktree_cancellable,
};

/// Carpeta vacía que se pasa a Git como `core.hooksPath` (un nivel bajo la raíz de worktrees:
/// nunca coincide con `<raíz>/<proyecto>/<intento>`).
pub const NO_HOOKS_DIR: &str = ".no-hooks";
/// Sondeo por defecto cuando nadie despierta al supervisor.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Margen sobre la gracia para esperar a los árboles al apagar; después se abortan (y soltar el
/// proceso mata su grupo).
const SHUTDOWN_MARGIN: Duration = Duration::from_secs(10);

/// Aviso de cambios para la app (el host emite `run-change`).
pub type ChangeSink = Arc<dyn Fn(RunChange) + Send + Sync>;

/// Configuración del supervisor.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// Directorio de datos del perfil (el mismo con el que se abrió el `Store`).
    pub data_dir: PathBuf,
    /// Ejecutable del proceso de prueba: el de la app (`current_exe`) o, en pruebas, el bin
    /// `kanbania-test-process`.
    pub program: PathBuf,
    /// Argumentos antes del escenario (por defecto, `--kanbania-test-process`).
    pub program_args: Vec<OsString>,
    /// **Solo pruebas**: argumentos tras el escenario (p. ej. `--heartbeat <carpeta>`). El host
    /// nunca los usa.
    pub scenario_args: Vec<OsString>,
    pub poll_interval: Duration,
    /// Gracia entre SIGTERM y SIGKILL (Unix).
    pub grace: Duration,
    pub git_timeout: Duration,
    /// Programa de Git (`git` del `PATH` por defecto). Las pruebas lo sustituyen por uno lento.
    pub git_program: PathBuf,
    /// **Solo pruebas**: sustituye el tiempo máximo de la configuración (mínimo 1 min) por uno
    /// más corto. El host nunca lo usa. El motivo registrado sigue diciendo los minutos de la
    /// configuración.
    pub timeout_override: Option<Duration>,
}

impl SupervisorConfig {
    pub fn new(data_dir: PathBuf, program: PathBuf) -> Self {
        Self {
            data_dir,
            program,
            program_args: vec![RESERVED_FLAG.into()],
            scenario_args: Vec::new(),
            poll_interval: DEFAULT_POLL_INTERVAL,
            grace: DEFAULT_GRACE,
            git_timeout: DEFAULT_GIT_TIMEOUT,
            git_program: PathBuf::from("git"),
            timeout_override: None,
        }
    }
}

/// Configuración de Git del perfil (hooks desactivados con la carpeta vacía propia).
pub fn git_config(data_dir: &Path, timeout: Duration) -> GitConfig {
    let mut git = GitConfig::new(data_dir.join(WORKTREES_DIR).join(NO_HOOKS_DIR));
    git.timeout = timeout;
    git
}

/// Carpeta de salida de un intento. El id debe venir del núcleo (ya validado); aun así se
/// rechaza todo lo que no sea un componente simple.
pub fn run_output_dir(data_dir: &Path, run_id: &str) -> Option<PathBuf> {
    let simple = !run_id.is_empty()
        && run_id.len() <= 64
        && run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    simple.then(|| data_dir.join(RUN_OUTPUT_DIR).join(run_id))
}

/// Últimas líneas de salida de un intento (≤ 500) en el formato del contrato.
pub fn read_run_output(data_dir: &Path, run_id: &str) -> std::io::Result<RunOutputView> {
    let dir = run_output_dir(data_dir, run_id)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "id no válido"))?;
    let tail = read_tail(&dir, MAX_TAIL_LINES)?;
    Ok(RunOutputView {
        lines: tail
            .lines
            .into_iter()
            .map(|line| RunOutputLine {
                at_ms: line.time_ms,
                stream: match line.stream {
                    Stream::Stdout => RunOutputStream::Stdout,
                    Stream::Stderr => RunOutputStream::Stderr,
                    Stream::System => RunOutputStream::System,
                },
                text: line.text,
                cut: line.cut,
            })
            .collect(),
        skipped_corrupt: tail.skipped_corrupt,
        earlier_dropped: tail.earlier_dropped,
    })
}

/// Recuperación al abrir el almacén, **antes** de despachar nada y aunque el supervisor no
/// llegue a arrancar: marca `interrupted` los intentos que quedaron activos de una sesión
/// anterior y anula las peticiones de inicio pendientes (avisando de ambos). Nada se relanza.
/// Exige el token del almacén: el host lo reclama, recupera y después arranca el supervisor o
/// lo suelta. [`Supervisor::start`] la llama siempre.
pub async fn recover_previous_session(
    store: &Store,
    token: &SupervisorToken,
    sink: &ChangeSink,
) -> Result<(), CoreError> {
    let notify = |runs: &[RunSummary]| {
        if !runs.is_empty() {
            sink(RunChange {
                run_ids: runs.iter().map(|r| r.id.clone()).collect(),
                story_ids: runs.iter().map(|r| r.story_id.clone()).collect(),
            });
        }
    };
    let interrupted =
        runs::interrupt_unverifiable_runs(store, token, InterruptCause::AppRestarted).await?;
    notify(&interrupted);
    let voided = runs::void_start_requests(store, token).await?;
    notify(&voided);
    Ok(())
}

/// Despierta al supervisor (tras iniciar, cancelar, aceptar, archivar o cambiar la
/// configuración). Clonable y barato.
#[derive(Debug, Clone)]
pub struct SupervisorWaker(Arc<Notify>);

impl SupervisorWaker {
    pub fn wake(&self) {
        self.0.notify_one();
    }
}

/// Supervisor en marcha.
pub struct Supervisor {
    waker: SupervisorWaker,
    /// `Some(motivo)` = apagar e interrumpir con ese motivo.
    stop: watch::Sender<Option<InterruptCause>>,
    task: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Supervisor")
    }
}

struct Shared {
    store: Store,
    token: SupervisorToken,
    config: SupervisorConfig,
    git: GitConfig,
    sink: ChangeSink,
    shutting_down: AtomicBool,
}

impl Shared {
    fn notify(&self, run_id: &str, story_id: &str) {
        (self.sink)(RunChange {
            run_ids: vec![run_id.to_owned()],
            story_ids: vec![story_id.to_owned()],
        });
    }

    fn notify_many(&self, runs: &[RunSummary]) {
        if runs.is_empty() {
            return;
        }
        (self.sink)(RunChange {
            run_ids: runs.iter().map(|r| r.id.clone()).collect(),
            story_ids: runs.iter().map(|r| r.story_id.clone()).collect(),
        });
    }
}

impl Supervisor {
    /// Arranca el supervisor. Antes del primer despacho marca `interrupted` los intentos que
    /// quedaron activos y anula las peticiones de inicio pendientes (avisando de ambos). Debe
    /// llamarse dentro de un runtime de tokio.
    pub async fn start(
        store: Store,
        token: SupervisorToken,
        config: SupervisorConfig,
        sink: ChangeSink,
    ) -> Result<Self, CoreError> {
        recover_previous_session(&store, &token, &sink).await?;
        let mut git = git_config(&config.data_dir, config.git_timeout);
        git.program = config.git_program.clone();
        let shared = Arc::new(Shared {
            store,
            token,
            config,
            git,
            sink,
            shutting_down: AtomicBool::new(false),
        });
        let waker = SupervisorWaker(Arc::new(Notify::new()));
        let (stop, stopped) = watch::channel(None);
        let task = tokio::spawn(run_loop(shared, waker.0.clone(), stopped));
        Ok(Self { waker, stop, task })
    }

    pub fn waker(&self) -> SupervisorWaker {
        self.waker.clone()
    }

    pub fn wake(&self) {
        self.waker.wake();
    }

    /// Apagado ordenado al salir: cancela todos los árboles de procesos, espera (gracia +
    /// margen), marca `interrupted` («Se cerró la app.») los intentos que quedaran activos y
    /// suelta el token y el almacén.
    pub async fn shutdown(self) {
        self.shutdown_for(InterruptCause::AppExiting).await;
    }

    /// Igual que [`Supervisor::shutdown`], con el motivo indicado (p. ej. `Restoring` antes de
    /// restaurar una copia).
    pub async fn shutdown_for(self, cause: InterruptCause) {
        self.stop.send_replace(Some(cause));
        self.waker.wake();
        if self.task.await.is_err() {
            eprintln!("[runner] el supervisor terminó de forma inesperada");
        }
    }
}

struct Active {
    cancel: CancelHandle,
}

async fn run_loop(
    shared: Arc<Shared>,
    wake: Arc<Notify>,
    mut stop: watch::Receiver<Option<InterruptCause>>,
) {
    let mut tasks: JoinSet<()> = JoinSet::new();
    let mut task_runs: HashMap<tokio::task::Id, String> = HashMap::new();
    let mut active: HashMap<String, Active> = HashMap::new();
    loop {
        while let Some(result) = tasks.try_join_next_with_id() {
            reap(result, &mut task_runs, &mut active);
        }
        if stop.borrow().is_some() {
            break;
        }
        apply_cancellations(&shared, &active).await;
        dispatch(&shared, &mut tasks, &mut task_runs, &mut active).await;
        tokio::select! {
            () = wake.notified() => {}
            () = tokio::time::sleep(shared.config.poll_interval) => {}
            _ = stop.changed() => {}
            Some(result) = tasks.join_next_with_id(), if !tasks.is_empty() => {
                reap(result, &mut task_runs, &mut active);
            }
        }
    }

    // Apagado.
    shared.shutting_down.store(true, Ordering::SeqCst);
    for entry in active.values() {
        entry.cancel.cancel();
    }
    let limit = shared.config.grace + SHUTDOWN_MARGIN;
    let drained = tokio::time::timeout(limit, async {
        while let Some(result) = tasks.join_next_with_id().await {
            reap(result, &mut task_runs, &mut active);
        }
    })
    .await;
    if drained.is_err() {
        // Soltar `RunningProcess` mata su grupo (Unix) o cierra su Job (Windows).
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        eprintln!("[runner] apagado: algunos procesos no terminaron a tiempo y se abortaron");
    }
    let cause = (*stop.borrow()).unwrap_or(InterruptCause::AppExiting);
    match runs::interrupt_unverifiable_runs(&shared.store, &shared.token, cause).await {
        Ok(changed) => shared.notify_many(&changed),
        Err(error) => log_core("interrumpir al salir", &error),
    }
}

fn reap(
    result: Result<(tokio::task::Id, ()), tokio::task::JoinError>,
    task_runs: &mut HashMap<tokio::task::Id, String>,
    active: &mut HashMap<String, Active>,
) {
    let id = match result {
        Ok((id, ())) => id,
        Err(error) => {
            eprintln!("[runner] una tarea de ejecución terminó de forma inesperada");
            error.id()
        }
    };
    if let Some(run_id) = task_runs.remove(&id) {
        active.remove(&run_id);
    }
}

/// Peticiones de cancelación: mata el árbol de los intentos propios; uno sin tarea viva (ya
/// terminó su proceso pero no se pudo cerrar) se cierra como cancelado directamente.
async fn apply_cancellations(shared: &Shared, active: &HashMap<String, Active>) {
    let pending = match runs::pending_cancellations(&shared.store, &shared.token).await {
        Ok(pending) => pending,
        Err(error) => {
            log_core("leer cancelaciones", &error);
            return;
        }
    };
    for request in pending {
        if let Some(entry) = active.get(&request.run_id) {
            entry.cancel.cancel();
            continue;
        }
        match runs::finish_run(
            &shared.store,
            &shared.token,
            &request.run_id,
            RunOutcome::Cancelled,
        )
        .await
        {
            Ok(run) => shared.notify(&run.id, &run.story_id),
            Err(error) => log_core("cerrar un intento cancelado", &error),
        }
    }
}

/// Despacha en orden FIFO mientras haya hueco.
async fn dispatch(
    shared: &Arc<Shared>,
    tasks: &mut JoinSet<()>,
    task_runs: &mut HashMap<tokio::task::Id, String>,
    active: &mut HashMap<String, Active>,
) {
    let limit = match runs::get_run_settings(&shared.store).await {
        Ok(settings) => usize::try_from(settings.max_concurrency.clamp(1, 8)).unwrap_or(1),
        Err(error) => {
            log_core("leer la configuración", &error);
            return;
        }
    };
    while active.len() < limit {
        let next = match runs::next_dispatchable(&shared.store, &shared.token).await {
            Ok(Some(next)) => next,
            Ok(None) => return,
            Err(error) => {
                log_core("buscar peticiones", &error);
                return;
            }
        };
        match runs::begin_dispatch(&shared.store, &shared.token, &next.run_id).await {
            Ok(DispatchOutcome::Ready(dispatch)) => {
                shared.notify(&dispatch.run_id, &dispatch.story_id);
                let (cancel, token) = cancel_pair();
                let run_id = dispatch.run_id.clone();
                let handle = tasks.spawn(execute(shared.clone(), dispatch, cancel.clone(), token));
                task_runs.insert(handle.id(), run_id.clone());
                active.insert(run_id, Active { cancel });
            }
            Ok(DispatchOutcome::Rejected(run)) => shared.notify(&run.id, &run.story_id),
            // Otro cambio se adelantó: la siguiente vuelta relee.
            Ok(DispatchOutcome::Stale) => return,
            Err(error) => {
                log_core("despachar", &error);
                return;
            }
        }
    }
}

/// Prepara, lanza y espera un intento despachado (`preparing`).
async fn execute(
    shared: Arc<Shared>,
    dispatch: Dispatch,
    cancel: CancelHandle,
    token: CancelToken,
) {
    let run_id = dispatch.run_id.clone();
    let Some(output_dir) = run_output_dir(&shared.config.data_dir, &run_id) else {
        finish(&shared, &dispatch, RunOutcome::Failed(RunFailure::Internal)).await;
        return;
    };
    let mut log = match OutputLog::create(&output_dir) {
        Ok(log) => log,
        Err(_) => {
            eprintln!("[runner] no se pudo crear el registro de salida de un intento");
            finish(&shared, &dispatch, RunOutcome::Failed(RunFailure::Internal)).await;
            return;
        }
    };
    log.write_system(&format!(
        "Proceso de prueba (sin IA), escenario «{}». No es trabajo de un agente.",
        dispatch.scenario.as_str()
    ));

    // 1. Registrar el worktree antes de crearlo.
    if let Err(error) = runs::record_worktree(&shared.store, &shared.token, &run_id).await {
        log.write_system(&format!("No se pudo registrar el worktree: {error}"));
        let _ = log.flush();
        let outcome = if token.is_cancelled() {
            RunOutcome::Cancelled
        } else {
            RunOutcome::Failed(RunFailure::Internal)
        };
        finish(&shared, &dispatch, outcome).await;
        return;
    }
    if token.is_cancelled() {
        log.write_system("Cancelado antes de crear el worktree.");
        let _ = log.flush();
        finish(&shared, &dispatch, RunOutcome::Cancelled).await;
        return;
    }

    // 2. Crear el worktree desde el commit de la instantánea.
    let Some(created) = prepare_worktree(&shared, &dispatch, &mut log, &token).await else {
        return;
    };
    if token.is_cancelled() {
        log.write_system("Cancelado antes de lanzar el proceso.");
        let _ = log.flush();
        finish(&shared, &dispatch, RunOutcome::Cancelled).await;
        return;
    }

    // 3. Lanzar el proceso de prueba en el worktree.
    let timeout = shared.config.timeout_override.unwrap_or_else(|| {
        Duration::from_secs(u64::try_from(dispatch.timeout_minutes.max(1)).unwrap_or(1) * 60)
    });
    let mut args = shared.config.program_args.clone();
    args.push(dispatch.scenario.as_str().into());
    args.extend(shared.config.scenario_args.iter().cloned());
    let spec = ProcessSpec {
        program: shared.config.program.clone(),
        args,
        cwd: created,
        extra_env: Vec::new(),
        timeout,
        grace: shared.config.grace,
    };
    let _ = log.flush();
    let process = match RunningProcess::spawn(spec, log) {
        Ok(process) => process,
        Err(failure) => {
            let mut log = failure.log;
            log.write_system(&format!("{} ({})", failure.error, failure.error.detail));
            let _ = log.flush();
            finish(
                &shared,
                &dispatch,
                RunOutcome::Failed(RunFailure::SpawnFailed),
            )
            .await;
            return;
        }
    };

    // 4. `running`. Si ya había una petición de cancelación, se mata en seguida.
    let mut not_running = false;
    match runs::mark_running(&shared.store, &shared.token, &run_id).await {
        Ok(run) => {
            shared.notify(&run.id, &run.story_id);
            if run.cancel_requested_at.is_some() {
                cancel.cancel();
            }
        }
        Err(error) => {
            log_core("marcar en ejecución", &error);
            // Sin `running` no hay salida válida: se mata el árbol y el intento falla.
            not_running = true;
            cancel.cancel();
        }
    }

    // 5. Esperar y cerrar.
    let (report, log) = process.wait(token).await;
    let outcome = match report.outcome {
        ProcessOutcome::Exited { code } => RunOutcome::Exited { code },
        ProcessOutcome::Signaled { signal } => RunOutcome::Signaled {
            signal: Some(signal),
        },
        ProcessOutcome::TimedOut => RunOutcome::TimedOut {
            minutes: dispatch.timeout_minutes,
        },
        ProcessOutcome::Cancelled => RunOutcome::Cancelled,
        ProcessOutcome::WaitFailed => RunOutcome::Failed(RunFailure::Internal),
    };
    let outcome = if not_running {
        RunOutcome::Failed(RunFailure::Internal)
    } else {
        outcome
    };
    if let Some(mut log) = log {
        let stats = report.output;
        log.write_system(&format!(
            "Fin del proceso de prueba: {:?}. Líneas: {}; cortadas: {}; archivos descartados \
             por rotación: {} ({} bytes).",
            report.outcome, stats.lines, stats.cut_lines, stats.dropped_files, stats.dropped_bytes
        ));
        let _ = log.finish();
    }
    finish(&shared, &dispatch, outcome).await;
}

/// Crea el worktree; si falla, registra el detalle y cierra el intento como `failed`.
async fn prepare_worktree(
    shared: &Shared,
    dispatch: &Dispatch,
    log: &mut OutputLog,
    cancel: &CancelToken,
) -> Option<PathBuf> {
    let root = shared.config.data_dir.join(WORKTREES_DIR);
    let expected = root.join(&dispatch.project_id).join(&dispatch.run_id);
    if expected != dispatch.worktree_path {
        log.write_system("La ruta del worktree no coincide con la del núcleo.");
        let _ = log.flush();
        finish(shared, dispatch, RunOutcome::Failed(RunFailure::Internal)).await;
        return None;
    }
    let request = CreateWorktree {
        repository: dispatch.repository_path.clone(),
        root,
        project_segment: dispatch.project_id.clone(),
        run_segment: dispatch.run_id.clone(),
        branch: dispatch.branch.clone(),
        commit: dispatch.base_commit.clone(),
    };
    // Cancelable: una petición de cancelación mata la orden de Git en curso.
    match create_worktree_cancellable(&shared.git, &request, Some(cancel)).await {
        Ok(created) => {
            log.write_system(&format!(
                "Worktree creado en la rama {} desde el commit {}.",
                created.branch, created.base_commit
            ));
            Some(created.path)
        }
        Err(error) => {
            log.write_system(&format!("{error} Detalle: {}", error.detail));
            let _ = log.flush();
            // Si `git worktree add` no llegó a ejecutarse, nada es del intento (p. ej. la rama
            // ya existía): se borra el registro para no atribuírselo ni tocarlo al limpiar.
            if !error.add_attempted
                && let Err(core) =
                    runs::discard_unused_worktree(&shared.store, &shared.token, &dispatch.run_id)
                        .await
            {
                log_core("descartar el registro del worktree", &core);
            }
            let outcome = if error.kind == WorktreeErrorKind::Cancelled {
                RunOutcome::Cancelled
            } else {
                RunOutcome::Failed(failure_of(&error))
            };
            finish(shared, dispatch, outcome).await;
            None
        }
    }
}

fn failure_of(error: &WorktreeError) -> RunFailure {
    match error.kind {
        WorktreeErrorKind::BranchExists => RunFailure::BranchExists,
        WorktreeErrorKind::NotARepository | WorktreeErrorKind::CommitNotFound => {
            RunFailure::RepositoryUnavailable
        }
        WorktreeErrorKind::HooksDir => RunFailure::Internal,
        _ => RunFailure::WorktreeFailed,
    }
}

/// Cierra el intento. Durante el apagado, un intento cancelado lo marca después
/// `interrupt_unverifiable_runs` («Se cerró la app.»), que no exige petición de cancelación.
async fn finish(shared: &Shared, dispatch: &Dispatch, outcome: RunOutcome) {
    if shared.shutting_down.load(Ordering::SeqCst) && outcome == RunOutcome::Cancelled {
        return;
    }
    match runs::finish_run(&shared.store, &shared.token, &dispatch.run_id, outcome).await {
        Ok(run) => shared.notify(&run.id, &run.story_id),
        // El estado cambió por otra vía (p. ej. se restauró una copia): no se reintenta.
        Err(error) => log_core("cerrar un intento", &error),
    }
}

/// Diagnóstico en stderr: solo la operación y el código de error (sin rutas ni detalles).
fn log_core(operation: &str, error: &CoreError) {
    eprintln!("[runner] {operation}: {:?}", error.code());
}
