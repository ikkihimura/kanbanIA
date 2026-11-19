//! Supervisor de ejecuciones (HU-018): orquesta el núcleo (`runs::supervisor`, protegido por
//! `SupervisorToken`) y los procesos de este crate.
//!
//! - Arranque ([`Supervisor::start`]): marca `interrupted` los intentos activos de una sesión
//!   anterior y anula las peticiones de inicio pendientes. Nada se relanza solo.
//! - Bucle: se despierta con [`SupervisorWaker::wake`] o cada `poll_interval`. En cada vuelta
//!   aplica las peticiones de cancelación y despacha (FIFO) mientras haya hueco según
//!   `max_concurrency`, leído en cada vuelta (bajarlo no detiene los que corren).
//! - Por intento: `begin_dispatch` → `record_worktree` (antes de tocar el disco: si la app muere
//!   en medio, el worktree nunca queda sin registrar) → `create_worktree` → lanzar el ejecutor →
//!   `mark_running` → esperar → `finish_run`.
//! - Ejecutores: el proceso de prueba (sin IA, HU-018) y un agente vía ACP (HU-019: Claude
//!   Code; HU-021: Codex), con el perfil del agente ([`acp_profile`]) y su entorno
//!   ([`adapter_env`]): el adaptador configurado se lanza en modo protocolo (sin argumentos,
//!   `cwd` = worktree) y, ya en `running`, se negocia (`initialize`, `session/new`, modo del
//!   perfil), se registra la sesión (`record_acp_session`), se envían las instrucciones de la
//!   instantánea y se espera el fin del turno. No se lanza si el worktree trae configuración
//!   del agente (perfil: Codex, `.codex` → `AgentConfigInRepository`) ni, en Codex, si su
//!   carpeta propia trae configuración (`codex_home_config` → `AgentConfigInCodexHome`). Fallo al lanzar → `AdapterMissing` (no existe) o
//!   `AdapterSpawnFailed` (otro motivo). Cancelar, el tiempo máximo y el
//!   apagado envían `session/cancel` y terminan el árbol tras una espera acotada.
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
    self, AcpAgent, AcpSessionInfo, ApprovalAsk, ApprovalDecision, ApprovalResolution, Dispatch,
    DispatchExecutor, DispatchOutcome, InterruptCause, RUN_OUTPUT_DIR, RunChange, RunFailure,
    RunOutcome, RunOutputKind, RunOutputLine, RunOutputStream, RunOutputView, RunSummary,
    SupervisorToken, TestScenario, WORKTREES_DIR,
};
use kanbania_core::platform::storage::Store;
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;

use crate::acp::{
    AcpClient, AcpFailure, AcpLimits, AcpProfile, AcpStop, AgentDeclaration, CLAUDE_CODE_PROFILE,
    CLAUDE_SANDBOX_AVAILABLE, CLAUDE_SANDBOX_NOTE, CLAUDE_UNCONFINED_NOTE, CODEX_CONFINEMENT_NOTE,
    CODEX_HOME_NOTE, CODEX_PROFILE, DataConfinement, OpenedSession, PermissionAnswer,
    PermissionEvent, PermissionPolicy, PermissionReply, TurnEnd, clear_codex_plugin_cache,
    codex_config, codex_env, codex_home_config, ensure_codex_home, has_repository_config,
    permission_bridge,
};
use crate::guard::{EXIT_GUARD_ERROR, EXIT_NOT_EXECUTABLE, GuardLauncher};
use crate::output::{AgentKind, MAX_TAIL_LINES, OutputLog, Stream, read_tail};
use crate::process::{
    CancelHandle, CancelToken, DEFAULT_GRACE, OutputSink, ProcessOutcome, ProcessSpec,
    RunningProcess, SpawnErrorKind, cancel_pair,
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

/// Aviso de una petición de permiso nueva (HU-020) para la notificación del sistema. Recibe solo
/// el número de la historia: nunca el comando, la ruta ni el título de la acción. El supervisor
/// solo lo llama si `run_settings.approval_notifications` está activado.
#[derive(Clone)]
pub struct ApprovalNotifier(pub Arc<dyn Fn(ApprovalNotice) + Send + Sync>);

impl std::fmt::Debug for ApprovalNotifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApprovalNotifier")
    }
}

/// Datos de la notificación (sin detalles de la acción).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalNotice {
    pub story_number: i64,
}

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
    /// Guardián de cada proceso (HU-024, Unix): por defecto el mismo ejecutable con
    /// `--kanbania-guard`. `None` solo en pruebas que lo necesiten.
    pub guard: Option<GuardLauncher>,
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
    /// **Solo pruebas**: argumentos para el adaptador ACP (p. ej. el escenario del agente
    /// falso). El host nunca los usa: el adaptador se lanza sin argumentos.
    pub acp_args: Vec<OsString>,
    /// Límites del cliente ACP (las pruebas los acortan).
    pub acp_limits: AcpLimits,
    /// Notificación del sistema al pedir permiso (el host la pone; las pruebas, una propia).
    pub approval_notifier: Option<ApprovalNotifier>,
}

impl SupervisorConfig {
    pub fn new(data_dir: PathBuf, program: PathBuf) -> Self {
        Self {
            data_dir,
            guard: Some(GuardLauncher::new(program.clone())),
            program,
            program_args: vec![RESERVED_FLAG.into()],
            scenario_args: Vec::new(),
            poll_interval: DEFAULT_POLL_INTERVAL,
            grace: DEFAULT_GRACE,
            git_timeout: DEFAULT_GIT_TIMEOUT,
            git_program: PathBuf::from("git"),
            timeout_override: None,
            acp_args: Vec::new(),
            acp_limits: AcpLimits::default(),
            approval_notifier: None,
        }
    }
}

/// Configuración de Git del perfil (hooks desactivados con la carpeta vacía propia).
pub fn git_config(data_dir: &Path, timeout: Duration) -> GitConfig {
    let mut git = GitConfig::new(data_dir.join(WORKTREES_DIR).join(NO_HOOKS_DIR));
    git.timeout = timeout;
    git
}

/// Perfil ACP de un agente del núcleo (el cliente ACP no enlaza el núcleo).
pub fn acp_profile(agent: AcpAgent) -> &'static AcpProfile {
    match agent {
        AcpAgent::ClaudeCode => &CLAUDE_CODE_PROFILE,
        AcpAgent::Codex => &CODEX_PROFILE,
    }
}

/// Variables propias del adaptador de `agent`, además de la lista común de adaptadores (las
/// mismas al ejecutar y al comprobar el adaptador). Codex: su carpeta propia
/// (`runs::codex_home`, que se crea 0700 si falta) y [`codex_env`]. E/S bloqueante y breve.
pub fn adapter_env(agent: AcpAgent, data_dir: &Path) -> std::io::Result<Vec<(OsString, OsString)>> {
    match agent {
        AcpAgent::ClaudeCode => Ok(Vec::new()),
        AcpAgent::Codex => {
            let home = runs::codex_home(data_dir);
            ensure_codex_home(&home)?;
            Ok(codex_env(&home))
        }
    }
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
                    Stream::Agent => RunOutputStream::Agent,
                },
                text: line.text,
                cut: line.cut,
                kind: line.kind.map(|kind| match kind {
                    AgentKind::Message => RunOutputKind::Message,
                    AgentKind::Thought => RunOutputKind::Thought,
                    AgentKind::Tool => RunOutputKind::Tool,
                    AgentKind::Plan => RunOutputKind::Plan,
                    AgentKind::Usage => RunOutputKind::Usage,
                    AgentKind::Permission => RunOutputKind::Permission,
                }),
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

/// Despierta al supervisor (tras iniciar, cancelar, aceptar, archivar, cambiar la
/// configuración o decidir un permiso). Clonable y barato. También avisa a las sesiones ACP de
/// que puede haber decisiones de permiso que entregar (HU-020).
#[derive(Debug, Clone)]
pub struct SupervisorWaker {
    notify: Arc<Notify>,
    approvals: Arc<watch::Sender<u64>>,
}

impl SupervisorWaker {
    pub fn wake(&self) {
        self.approvals
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
        self.notify.notify_one();
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
    /// También lo lee el cliente ACP para esperar menos al cancelar durante el apagado.
    shutting_down: Arc<AtomicBool>,
    /// Sube con cada `wake`: las sesiones ACP vuelven a mirar sus peticiones de permiso.
    approvals: Arc<watch::Sender<u64>>,
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
        let approvals = Arc::new(watch::channel(0_u64).0);
        let shared = Arc::new(Shared {
            store,
            token,
            config,
            git,
            sink,
            shutting_down: Arc::new(AtomicBool::new(false)),
            approvals: approvals.clone(),
        });
        let waker = SupervisorWaker {
            notify: Arc::new(Notify::new()),
            approvals,
        };
        let (stop, stopped) = watch::channel(None);
        let task = tokio::spawn(run_loop(shared, waker.notify.clone(), stopped));
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
    match &dispatch.executor {
        DispatchExecutor::TestProcess(scenario) => log.write_system(&format!(
            "Proceso de prueba (sin IA), escenario «{}». No es trabajo de un agente.",
            scenario.as_str()
        )),
        DispatchExecutor::Acp { agent, .. } => log.write_system(&format!(
            "{} vía ACP. Lo que el agente informa se guarda como «agente (reportado)»: la app no \
             lo comprueba. Permisos: cada petición espera la decisión de una persona (permitir \
             una vez o rechazar; caduca a los 10 min).",
            agent.label()
        )),
    }

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

    // 3. Lanzar el ejecutor en el worktree.
    let timeout = shared.config.timeout_override.unwrap_or_else(|| {
        Duration::from_secs(u64::try_from(dispatch.timeout_minutes.max(1)).unwrap_or(1) * 60)
    });
    match dispatch.executor.clone() {
        DispatchExecutor::TestProcess(scenario) => {
            run_test_process(
                &shared, &dispatch, log, created, timeout, scenario, &cancel, token,
            )
            .await;
        }
        DispatchExecutor::Acp {
            agent,
            adapter_path,
            instructions,
        } => {
            let launch = AcpLaunch {
                agent,
                adapter_path,
                instructions,
                worktree: created,
                timeout,
            };
            run_acp(&shared, &dispatch, log, launch, &cancel, token).await;
        }
    }
}

/// `preparing → running` tras lanzar. Si ya había una petición de cancelación, cancela en
/// seguida. Devuelve `false` si no se pudo marcar (entonces se cancela y el intento falla).
async fn enter_running(shared: &Shared, run_id: &str, cancel: &CancelHandle) -> bool {
    match runs::mark_running(&shared.store, &shared.token, run_id).await {
        Ok(run) => {
            shared.notify(&run.id, &run.story_id);
            if run.cancel_requested_at.is_some() {
                cancel.cancel();
            }
            true
        }
        Err(error) => {
            log_core("marcar en ejecución", &error);
            // Sin `running` no hay salida válida: se mata el árbol y el intento falla.
            cancel.cancel();
            false
        }
    }
}

/// Proceso de prueba (sin IA, HU-018): sin cambios de comportamiento.
#[allow(clippy::too_many_arguments)]
async fn run_test_process(
    shared: &Shared,
    dispatch: &Dispatch,
    log: OutputLog,
    worktree: PathBuf,
    timeout: Duration,
    scenario: TestScenario,
    cancel: &CancelHandle,
    token: CancelToken,
) {
    let mut args = shared.config.program_args.clone();
    args.push(scenario.as_str().into());
    args.extend(shared.config.scenario_args.iter().cloned());
    let spec = ProcessSpec {
        program: shared.config.program.clone(),
        args,
        cwd: worktree,
        extra_env: Vec::new(),
        timeout,
        grace: shared.config.grace,
        guard: shared.config.guard.clone(),
    };
    let mut log = log;
    let _ = log.flush();
    let process = match RunningProcess::spawn(spec, log) {
        Ok(process) => process,
        Err(failure) => {
            let mut log = failure.log;
            log.write_system(&format!("{} ({})", failure.error, failure.error.detail));
            let _ = log.flush();
            finish(
                shared,
                dispatch,
                RunOutcome::Failed(RunFailure::SpawnFailed),
            )
            .await;
            return;
        }
    };

    // 4. `running`. Si ya había una petición de cancelación, se mata en seguida.
    let running = enter_running(shared, &dispatch.run_id, cancel).await;

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
        ProcessOutcome::Stopped | ProcessOutcome::WaitFailed => {
            RunOutcome::Failed(RunFailure::Internal)
        }
    };
    let outcome = if running {
        outcome
    } else {
        RunOutcome::Failed(RunFailure::Internal)
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
    finish(shared, dispatch, outcome).await;
}

/// Lo que necesita el ejecutor ACP.
struct AcpLaunch {
    agent: AcpAgent,
    adapter_path: PathBuf,
    instructions: String,
    worktree: PathBuf,
    timeout: Duration,
}

/// Cómo terminó la sesión ACP.
enum SessionEnd {
    Stop(AcpStop),
    /// No se pudo registrar la sesión en el núcleo.
    Internal,
    /// El adaptador no llegó a ejecutarse (p. ej. le falta su intérprete).
    Missing,
    /// El guardián no pudo ejecutar el adaptador (sin permiso, formato no ejecutable…).
    Unlaunchable,
}

/// Lo que queda listo para lanzar el adaptador.
struct PreparedAcp {
    env: Vec<(OsString, OsString)>,
    /// Lanzar el adaptador dentro del sandbox de macOS (HU-023: Claude Code).
    sandbox: Option<DataConfinement>,
}

/// Antes de lanzar el adaptador: comprobaciones del agente sobre el worktree y su entorno, y el
/// aislamiento de los datos de la app (HU-023). `Err` = motivo para cerrar el intento sin
/// lanzar nada.
async fn prepare_acp(
    shared: &Shared,
    agent: AcpAgent,
    worktree: &Path,
    log: &mut OutputLog,
) -> Result<PreparedAcp, RunFailure> {
    let data_dir = shared.config.data_dir.clone();
    let worktree = worktree.to_path_buf();
    let checked = tokio::task::spawn_blocking(move || {
        // Configuración del repositorio que el agente cargaría al abrir la sesión (perfil).
        let repository = has_repository_config(acp_profile(agent), &worktree);
        let env = adapter_env(agent, &data_dir);
        // Después de `adapter_env`, que crea la carpeta propia si falta.
        let home = agent_home_config(agent, &data_dir);
        let confinement = DataConfinement::new(&data_dir, &worktree);
        (repository, env, home, confinement)
    })
    .await;
    let Ok((repository, env, home, confinement)) = checked else {
        log.write_system("No se pudo preparar el lanzamiento del adaptador.");
        return Err(RunFailure::Internal);
    };
    match repository {
        Ok(None) => {}
        Ok(Some(name)) => {
            log.write_system(&format!(
                "El worktree contiene configuración del agente ({name}): {} la cargaría al abrir \
                 la sesión (servidores MCP, reglas, ajustes). No se lanza el adaptador.",
                agent.label()
            ));
            return Err(RunFailure::AgentConfigInRepository(agent));
        }
        Err(error) => {
            log.write_system(&format!(
                "No se pudo comprobar si el worktree trae configuración del agente: {error}. No se \
                 lanza el adaptador."
            ));
            return Err(RunFailure::AgentConfigInRepository(agent));
        }
    }
    let env = match env {
        Ok(env) => env,
        Err(error) => {
            log.write_system(&format!(
                "No se pudo preparar la carpeta propia del agente: {}.",
                error.kind()
            ));
            return Err(RunFailure::AdapterSpawnFailed(agent));
        }
    };
    match home {
        Ok(None) => {}
        Ok(Some(name)) => {
            log.write_system(&format!(
                "La carpeta de Codex de la app contiene «{name}» (configuración, reglas, \
                 instrucciones o skills propias): Codex lo cargaría y podría saltarse los permisos. \
                 No se lanza el adaptador."
            ));
            return Err(RunFailure::AgentConfigInCodexHome);
        }
        Err(error) => {
            log.write_system(&format!(
                "No se pudo comprobar la carpeta de Codex de la app: {}. No se lanza el adaptador.",
                error.kind()
            ));
            return Err(RunFailure::AgentConfigInCodexHome);
        }
    }
    // HU-023: sin rutas canónicas no hay aislamiento posible → no se lanza (falla cerrado).
    let confinement = match confinement {
        Ok(confinement) => confinement,
        Err(error) => {
            log.write_system(&format!(
                "No se pudo preparar el aislamiento de los datos de la app: {}. No se lanza el \
                 adaptador.",
                error.kind()
            ));
            return Err(RunFailure::AdapterSpawnFailed(agent));
        }
    };
    let mut env = env;
    let sandbox = match agent {
        AcpAgent::Codex => {
            log.write_system(CODEX_HOME_NOTE);
            log.write_system(CODEX_CONFINEMENT_NOTE);
            let config = codex_config(&confinement);
            for (name, value) in &mut env {
                if name == "CODEX_CONFIG" {
                    *value = config.clone().into();
                }
            }
            None
        }
        AcpAgent::ClaudeCode if CLAUDE_SANDBOX_AVAILABLE => {
            log.write_system(CLAUDE_SANDBOX_NOTE);
            Some(confinement)
        }
        AcpAgent::ClaudeCode => {
            log.write_system(CLAUDE_UNCONFINED_NOTE);
            None
        }
    };
    Ok(PreparedAcp { env, sandbox })
}

/// Configuración no admitida en la carpeta propia del agente (Codex: [`codex_home_config`]):
/// el nombre de la entrada o `None`. E/S bloqueante y breve.
fn agent_home_config(agent: AcpAgent, data_dir: &Path) -> std::io::Result<Option<&'static str>> {
    match agent {
        AcpAgent::ClaudeCode => Ok(None),
        AcpAgent::Codex => {
            let home = runs::codex_home(data_dir);
            clear_codex_plugin_cache(&home)?;
            codex_home_config(&home)
        }
    }
}

/// Motivo por el que no se podría lanzar el adaptador (solo se usa con el sandbox, ver
/// `run_acp`): no existe → `AdapterMissing`; no es un archivo ejecutable → `AdapterSpawnFailed`.
fn adapter_unlaunchable(path: &Path, agent: AcpAgent) -> Option<RunFailure> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(RunFailure::AdapterMissing(agent));
        }
        Err(_) => return Some(RunFailure::AdapterSpawnFailed(agent)),
    };
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let executable = true;
    (!metadata.is_file() || !executable).then_some(RunFailure::AdapterSpawnFailed(agent))
}

/// Un agente vía ACP (HU-019, HU-021).
async fn run_acp(
    shared: &Shared,
    dispatch: &Dispatch,
    log: OutputLog,
    launch: AcpLaunch,
    cancel: &CancelHandle,
    mut token: CancelToken,
) {
    let agent = launch.agent;
    let mut log = log;
    let prepared = match prepare_acp(shared, agent, &launch.worktree, &mut log).await {
        Ok(prepared) => prepared,
        Err(failure) => {
            let _ = log.finish();
            finish(shared, dispatch, RunOutcome::Failed(failure)).await;
            return;
        }
    };
    // Con el sandbox o el guardián, el programa que se lanza es `sandbox-exec` o la propia app:
    // un adaptador que no existe o no es ejecutable ya no fallaría al lanzar (fallaría después),
    // así que se comprueba antes, con los mismos motivos que sin ellos.
    if (prepared.sandbox.is_some() || shared.config.guard.is_some())
        && let Some(failure) = adapter_unlaunchable(&launch.adapter_path, agent)
    {
        log.write_system(match failure {
            RunFailure::AdapterMissing(_) => "No se pudo lanzar el adaptador: no existe.",
            _ => "No se pudo lanzar el adaptador: no es un archivo ejecutable.",
        });
        let _ = log.finish();
        finish(shared, dispatch, RunOutcome::Failed(failure)).await;
        return;
    }

    let (program, args) = match &prepared.sandbox {
        Some(confinement) => confinement.sandboxed(&launch.adapter_path, &shared.config.acp_args),
        None => (launch.adapter_path, shared.config.acp_args.clone()),
    };
    let spec = ProcessSpec {
        program,
        args,
        cwd: launch.worktree.clone(),
        extra_env: prepared.env,
        timeout: launch.timeout,
        grace: shared.config.grace,
        guard: shared.config.guard.clone(),
    };
    let _ = log.flush();
    let (process, io) = match RunningProcess::spawn_protocol(spec, log) {
        Ok(started) => started,
        Err(failure) => {
            let mut log = failure.log;
            log.write_system(&format!(
                "No se pudo lanzar el adaptador: {} ({})",
                failure.error, failure.error.detail
            ));
            let _ = log.flush();
            // No existe → «no se encontró»; cualquier otro fallo (permiso, ruta no válida…) →
            // «no se pudo lanzar».
            let failure = if failure.error.kind == SpawnErrorKind::NotFound {
                RunFailure::AdapterMissing(agent)
            } else {
                RunFailure::AdapterSpawnFailed(agent)
            };
            finish(shared, dispatch, RunOutcome::Failed(failure)).await;
            return;
        }
    };

    // 4. `running` en cuanto el proceso existe; la sesión se negocia ya en ejecución.
    let running = enter_running(shared, &dispatch.run_id, cancel).await;

    // 5. Negociar, registrar la sesión, enviar las instrucciones y esperar el turno.
    let limits = shared.config.acp_limits;
    let deadline = process.deadline();
    let (bridge, events, answers) = permission_bridge();
    let mut client = AcpClient::new(io, limits, Some(shared.shutting_down.clone()))
        .with_permissions(PermissionPolicy::Ask(bridge));
    // El broker de permisos vive mientras dure la sesión. Al terminar esta, acaba lo que tenga
    // en curso (nunca se suelta a medias una escritura del núcleo), registra las retiradas que
    // queden en la cola y sale; lo que quede abierto lo cierra `finish_run`.
    let (done, session_done) = tokio::sync::oneshot::channel::<()>();
    let session = async {
        let ended = acp_session(
            shared,
            dispatch,
            &mut client,
            acp_profile(agent),
            &launch.worktree,
            &launch.instructions,
            &mut token,
            deadline,
        )
        .await;
        let _ = done.send(());
        ended
    };
    let mut events = events;
    let mut open = Vec::new();
    let (ended, ()) = tokio::join!(
        session,
        approval_broker(
            shared,
            dispatch,
            &mut events,
            &answers,
            &mut open,
            session_done
        )
    );
    while let Ok(event) = events.try_recv() {
        // Lo que quedó en la cola se registra igual (sin notificar), para que el historial
        // de peticiones sea completo; `Closed`/`Withdrawn` las cierran en seguida.
        match event {
            PermissionEvent::Asked(ask) => {
                if let Some(item) = register_ask(shared, dispatch, ask, None).await {
                    open.push(item);
                }
            }
            PermissionEvent::Withdrawn { key } => {
                withdraw_open(shared, dispatch, &mut open, key, false).await;
            }
            PermissionEvent::Closed { key } => {
                withdraw_open(shared, dispatch, &mut open, key, true).await;
            }
        }
    }
    client.close_input();
    drop(client);
    // Tras un fin normal, el adaptador puede terminar solo al cerrar su stdin (y tras una
    // desconexión, ya está terminando: así se conoce su código); en cualquier otro caso se
    // termina el árbol en seguida (aviso, gracia, fuerza).
    let wait_exit = if matches!(
        ended,
        Ok(_) | Err(SessionEnd::Stop(AcpStop::Failed(AcpFailure::Disconnected)))
    ) {
        limits.close_wait
    } else {
        Duration::ZERO
    };
    let (report, mut log) = process.close(wait_exit).await;

    // Un script `#!/usr/bin/env node` sin `node` en el PATH sale con 127 sin hablar ACP: es un
    // adaptador que no se puede ejecutar, no una desconexión del agente.
    let ended = match ended {
        Err(SessionEnd::Stop(AcpStop::Failed(AcpFailure::Disconnected)))
            if report.outcome == (ProcessOutcome::Exited { code: 127 }) =>
        {
            if let Some(log) = log.as_mut() {
                log.write_system(
                    "El adaptador terminó con el código 127 sin responder: suele indicar que no \
                     encuentra su intérprete (p. ej. `node` en el PATH con el que se abrió la app).",
                );
            }
            Err(SessionEnd::Missing)
        }
        // Con guardián (HU-024), un adaptador que no se puede ejecutar ya no falla al lanzar:
        // el guardián sale con 126 (sin permiso) o 125 (otro error del sistema).
        Err(SessionEnd::Stop(AcpStop::Failed(AcpFailure::Disconnected)))
            if shared.config.guard.is_some()
                && matches!(
                    report.outcome,
                    ProcessOutcome::Exited {
                        code: EXIT_NOT_EXECUTABLE | EXIT_GUARD_ERROR
                    }
                ) =>
        {
            if let Some(log) = log.as_mut() {
                log.write_system("No se pudo ejecutar el adaptador (código del guardián).");
            }
            Err(SessionEnd::Unlaunchable)
        }
        other => other,
    };
    let (outcome, summary) = match ended {
        Ok(TurnEnd::EndTurn) => (RunOutcome::AgentEndedTurn, "el agente terminó su turno"),
        Ok(TurnEnd::StoppedAfterRejection) => (
            RunOutcome::AgentStoppedAfterRejection,
            "el agente detuvo su turno tras un permiso rechazado",
        ),
        Err(SessionEnd::Stop(AcpStop::Cancelled)) => (RunOutcome::Cancelled, "cancelado"),
        Err(SessionEnd::Stop(AcpStop::TimedOut)) => (
            RunOutcome::TimedOut {
                minutes: dispatch.timeout_minutes,
            },
            "tiempo máximo agotado",
        ),
        Err(SessionEnd::Stop(AcpStop::Failed(failure))) => (
            RunOutcome::Failed(acp_failure(failure, agent)),
            "fallo de la sesión ACP",
        ),
        Err(SessionEnd::Internal) => (
            RunOutcome::Failed(RunFailure::Internal),
            "no se pudo registrar la sesión",
        ),
        Err(SessionEnd::Missing) => (
            RunOutcome::Failed(RunFailure::AdapterMissing(agent)),
            "el adaptador no se pudo ejecutar",
        ),
        Err(SessionEnd::Unlaunchable) => (
            RunOutcome::Failed(RunFailure::AdapterSpawnFailed(agent)),
            "el adaptador no se pudo ejecutar",
        ),
    };
    let outcome = if running {
        outcome
    } else {
        RunOutcome::Failed(RunFailure::Internal)
    };
    if let Some(mut log) = log {
        let stats = report.output;
        log.write_system(&format!(
            "Fin de la sesión ACP: {summary}. Proceso del adaptador: {:?}. Líneas: {}; \
             cortadas: {}; archivos descartados por rotación: {} ({} bytes).",
            report.outcome, stats.lines, stats.cut_lines, stats.dropped_files, stats.dropped_bytes
        ));
        let _ = log.finish();
    }
    finish(shared, dispatch, outcome).await;
}

#[allow(clippy::too_many_arguments)]
async fn acp_session(
    shared: &Shared,
    dispatch: &Dispatch,
    client: &mut AcpClient,
    profile: &AcpProfile,
    worktree: &Path,
    instructions: &str,
    token: &mut CancelToken,
    deadline: tokio::time::Instant,
) -> Result<TurnEnd, SessionEnd> {
    let declared = client
        .initialize(token, deadline)
        .await
        .map_err(SessionEnd::Stop)?;
    let session = client
        .open_session(worktree, &declared, profile, token, deadline)
        .await
        .map_err(SessionEnd::Stop)?;
    // La huella es la del texto exacto que va en `session/prompt` (`instructions`, sin cambios).
    record_session(
        shared,
        dispatch,
        &client.sink(),
        &declared,
        &session,
        instructions,
    )
    .await?;
    client
        .prompt(instructions, token, deadline)
        .await
        .map_err(SessionEnd::Stop)
}

/// Registra una vez lo que el agente declaró (CA-1) con la huella del texto exacto que se
/// enviará en `session/prompt`. El núcleo omite lo que no cumpla sus límites
/// (`AcpSessionInfo::from_reported`); si omite algo, se anota en `sys` (sin el dato).
async fn record_session(
    shared: &Shared,
    dispatch: &Dispatch,
    sink: &OutputSink,
    declared: &AgentDeclaration,
    session: &OpenedSession,
    prompt: &str,
) -> Result<(), SessionEnd> {
    let reported = AcpSessionInfo::from_reported(
        declared.protocol_version,
        declared.name.clone(),
        declared.title.clone(),
        declared.version.clone(),
        session
            .capabilities
            .iter()
            .map(|c| (*c).to_owned())
            .collect(),
        Some(session.mode.clone()),
    );
    if reported.identity_dropped || reported.mode_dropped || reported.dropped_capabilities > 0 {
        sink.system(
            "Parte de lo que declaró el agente no cumple los límites de la app y no se registra \
             (identidad, modo o capacidades desconocidas).",
        )
        .await;
    }
    let info = reported
        .info
        .with_model(session.model.as_deref())
        .with_prompt(prompt);
    if session.model.is_some() && info.model.is_none() {
        sink.system(
            "El modelo que declaró el agente no cumple los límites de la app y no se registra.",
        )
        .await;
    }
    match runs::record_acp_session(&shared.store, &shared.token, &dispatch.run_id, &info).await {
        Ok(run) => {
            shared.notify(&run.id, &run.story_id);
            Ok(())
        }
        Err(error) => {
            log_core("registrar la sesión del agente", &error);
            Err(SessionEnd::Internal)
        }
    }
}

// Los límites del cliente ACP (que no enlaza el núcleo) deben coincidir con los del núcleo.
const _: () = assert!(
    crate::acp::MAX_PENDING_PERMISSIONS as i64 == runs::MAX_PENDING_APPROVALS
        && crate::acp::MAX_SCOPE_CHARS == runs::MAX_APPROVAL_SCOPE_CHARS
);

/// Peticiones de permiso de una sesión ACP abiertas en el núcleo.
struct OpenApproval {
    key: u64,
    approval_id: String,
    expires_at_ms: i64,
}

/// Registra una petición del agente en el núcleo y avisa a la app (y, al empezar a esperar, la
/// notificación del sistema si está activada). Si no se puede registrar y hay a quién
/// responder (`answers`), se responde `cancelled` (intento que ya no admite decisiones) o
/// rechazo sin preguntar.
async fn register_ask(
    shared: &Shared,
    dispatch: &Dispatch,
    ask: crate::acp::PermissionAsk,
    answers: Option<&tokio::sync::mpsc::Sender<PermissionAnswer>>,
) -> Option<OpenApproval> {
    let core_ask = ApprovalAsk {
        tool_title: ask.title,
        tool_kind: ask.kind,
        scope: ask.scope,
        can_allow: ask.can_allow,
    };
    match runs::request_approval(&shared.store, &shared.token, &dispatch.run_id, &core_ask).await {
        Ok(requested) => {
            shared.notify(&dispatch.run_id, &requested.story_id);
            // Una notificación al empezar a esperar, no una por petición.
            if requested.started_waiting && answers.is_some() {
                notify_approval(shared, requested.story_number).await;
            }
            Some(OpenApproval {
                key: ask.key,
                approval_id: requested.approval.id,
                expires_at_ms: requested.expires_at_ms,
            })
        }
        Err(error) => {
            let reply = if matches!(error, CoreError::ApprovalClosed { .. }) {
                PermissionReply::Cancelled
            } else {
                log_core("registrar una petición de permiso", &error);
                PermissionReply::Unrecorded
            };
            if let Some(answers) = answers {
                let _ = answers.try_send(PermissionAnswer {
                    key: ask.key,
                    reply,
                });
            }
            None
        }
    }
}

/// Pasa las peticiones del cliente ACP al núcleo (`request_approval`) y entrega las decisiones
/// (`resolve_approval`) cuando el supervisor se despierta o vence la caducidad más próxima.
/// Termina cuando acaba la sesión (`session_done`), entre dos operaciones.
async fn approval_broker(
    shared: &Shared,
    dispatch: &Dispatch,
    events: &mut tokio::sync::mpsc::Receiver<PermissionEvent>,
    answers: &tokio::sync::mpsc::Sender<PermissionAnswer>,
    open: &mut Vec<OpenApproval>,
    mut session_done: tokio::sync::oneshot::Receiver<()>,
) {
    let mut epoch = shared.approvals.subscribe();
    let mut next_expiry: Option<i64> = None;
    let mut events_closed = false;
    loop {
        // Hasta la caducidad más próxima, y como mucho `poll_interval` mientras haya alguna
        // abierta: así también caducan si el reloj salta (equipo suspendido) o si nadie despierta
        // al supervisor. Revisar una pendiente es solo una lectura.
        let wait_expiry = async {
            match next_expiry {
                Some(at_ms) => {
                    let wait = u64::try_from(at_ms - shared.store.now_ms()).unwrap_or(0);
                    // Un pequeño margen: el núcleo compara con su propio reloj.
                    let wait = Duration::from_millis(wait + 50).min(shared.config.poll_interval);
                    tokio::time::sleep(wait).await;
                }
                None => std::future::pending().await,
            }
        };
        let event = async {
            if events_closed {
                std::future::pending().await
            } else {
                events.recv().await
            }
        };
        tokio::select! {
            biased;
            _ = &mut session_done => return,
            event = event => match event {
                Some(PermissionEvent::Asked(ask)) => {
                    if let Some(item) = register_ask(shared, dispatch, ask, Some(answers)).await {
                        next_expiry = Some(
                            next_expiry.map_or(item.expires_at_ms, |at| at.min(item.expires_at_ms)),
                        );
                        open.push(item);
                    }
                }
                Some(PermissionEvent::Withdrawn { key }) => {
                    withdraw_open(shared, dispatch, open, key, false).await;
                }
                Some(PermissionEvent::Closed { key }) => {
                    withdraw_open(shared, dispatch, open, key, true).await;
                }
                None => events_closed = true,
            },
            changed = epoch.changed() => {
                if changed.is_err() {
                    // El supervisor se apaga: la cancelación cerrará la sesión.
                    let _ = (&mut session_done).await;
                    return;
                }
                next_expiry = deliver_decisions(shared, dispatch, open, answers).await;
            }
            () = wait_expiry => {
                next_expiry = deliver_decisions(shared, dispatch, open, answers).await;
            }
        }
    }
}

/// El agente retiró una petición: se registra en el núcleo.
async fn withdraw_open(
    shared: &Shared,
    dispatch: &Dispatch,
    open: &mut Vec<OpenApproval>,
    key: u64,
    closed: bool,
) {
    if let Some(index) = open.iter().position(|o| o.key == key) {
        let item = open.remove(index);
        let result = if closed {
            runs::cancel_approval(&shared.store, &shared.token, &item.approval_id).await
        } else {
            runs::withdraw_approval(&shared.store, &shared.token, &item.approval_id).await
        };
        match result {
            Ok(_) => shared.notify(&dispatch.run_id, &dispatch.story_id),
            Err(error) => log_core("retirar una petición de permiso", &error),
        }
    }
}

/// Mira cada petición abierta en el núcleo y responde las que ya tienen respuesta. Devuelve la
/// caducidad más próxima de las que siguen pendientes.
async fn deliver_decisions(
    shared: &Shared,
    dispatch: &Dispatch,
    open: &mut Vec<OpenApproval>,
    answers: &tokio::sync::mpsc::Sender<PermissionAnswer>,
) -> Option<i64> {
    let mut next_expiry: Option<i64> = None;
    let mut changed = false;
    let mut still_open = Vec::with_capacity(open.len());
    for item in open.drain(..) {
        let resolution =
            match runs::resolve_approval(&shared.store, &shared.token, &item.approval_id).await {
                Ok(resolution) => resolution,
                Err(error) => {
                    // Error transitorio (p. ej. base ocupada): se reintenta en un segundo, no
                    // solo con el próximo `wake`.
                    log_core("entregar una decisión de permiso", &error);
                    let retry = shared.store.now_ms() + 1_000;
                    next_expiry = Some(next_expiry.map_or(retry, |at| at.min(retry)));
                    still_open.push(item);
                    continue;
                }
            };
        let reply = match resolution {
            ApprovalResolution::Pending { expires_at_ms } => {
                next_expiry = Some(next_expiry.map_or(expires_at_ms, |at| at.min(expires_at_ms)));
                still_open.push(item);
                continue;
            }
            ApprovalResolution::Deliver(ApprovalDecision::AllowOnce) => PermissionReply::AllowOnce,
            ApprovalResolution::Deliver(ApprovalDecision::Reject) => PermissionReply::Reject,
            ApprovalResolution::Expired => PermissionReply::Expired,
            // Cerrada por una cancelación o el fin del intento: no se responde aquí. La
            // cancelación del turno (`session/cancel`) responde `cancelled` a las pendientes en
            // el orden de ACP; si no llegara, el tiempo máximo termina la sesión.
            ApprovalResolution::Closed => {
                changed = true;
                continue;
            }
        };
        changed = true;
        let _ = answers.try_send(PermissionAnswer {
            key: item.key,
            reply,
        });
    }
    *open = still_open;
    if changed {
        shared.notify(&dispatch.run_id, &dispatch.story_id);
    }
    next_expiry
}

/// Notificación del sistema si está activada en la configuración (solo el número de historia).
async fn notify_approval(shared: &Shared, story_number: i64) {
    let Some(notifier) = shared.config.approval_notifier.as_ref() else {
        return;
    };
    match runs::get_run_settings(&shared.store).await {
        Ok(settings) if settings.approval_notifications => {
            (notifier.0)(ApprovalNotice { story_number });
        }
        Ok(_) => {}
        Err(error) => log_core("leer la configuración de notificaciones", &error),
    }
}

fn acp_failure(failure: AcpFailure, agent: AcpAgent) -> RunFailure {
    match failure {
        AcpFailure::Disconnected => RunFailure::AgentDisconnected,
        AcpFailure::Protocol => RunFailure::AgentProtocolError,
        AcpFailure::VersionUnsupported => RunFailure::AgentVersionUnsupported,
        AcpFailure::AuthRequired => RunFailure::AgentAuthRequired(agent),
        AcpFailure::ModeUnavailable => RunFailure::AgentModeUnavailable,
        AcpFailure::Refused => RunFailure::AgentRefused,
        AcpFailure::LimitReached => RunFailure::AgentLimitReached,
        AcpFailure::RequestFailed => RunFailure::AgentRequestFailed,
        AcpFailure::TurnFailed => RunFailure::AgentTurnFailed,
    }
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
