//! Slice `runs`: intentos de ejecución de una historia (HU-017) y su ejecución supervisada
//! (HU-018). Ejecutores: un proceso de prueba (sin IA, HU-018) y Claude Code vía ACP (HU-019).
//!
//! - Instantánea inmutable: al crear el intento se congela en JSON (con su SHA-256) lo que
//!   recibiría el agente: historia, criterios, épica, padre, prerequisitos, perfil del agente,
//!   repositorios de contexto con su HEAD (leído de `.git` sin ejecutar `git`) e instrucciones
//!   Markdown generadas por el núcleo más una nota opcional. Un trigger impide cualquier
//!   UPDATE de la instantánea. Sin secretos: se rechaza todo lo que parezca un secreto
//!   (`platform::text::contains_secret_like`) y nunca se copian hashes, `service.json`,
//!   entorno ni rutas de datos.
//! - Máquina de estados (`RunState::can_transition_to`); cualquier otro cambio →
//!   `invalid_run_transition`. Restaurar una copia marca `interrupted` todo intento activo
//!   con la transición normal.
//! - Eventos con secuencia 1..n sin huecos por intento (`record_run_event`): duplicado exacto
//!   → no se aplica dos veces; misma secuencia con otro contenido → `conflict`; hueco → error.
//! - Un solo intento activo por historia (índice único parcial + `run_active`).
//! - Precondiciones en `ensure_runnable`: al crear, al pedir el inicio y al despachar.
//! - Crear, iniciar y cancelar exigen `Actor::Human` y son idempotentes por `requestId`.
//!   Cancelar (y aceptar o archivar la historia, `cancel_story_runs`) es directo en `queued` y
//!   una petición que confirma el supervisor si hay proceso.
//! - Ejecución (HU-018): `start_run` registra una petición de inicio; el supervisor
//!   (`crates/runner`) usa la API de `supervisor`, protegida por `SupervisorToken`.
//!   Configuración de concurrencia y tiempo máximo en `run_settings`. Worktrees registrados en
//!   `run_worktrees` (rutas absolutas, nunca para agentes).
//! - ACP (HU-019): el ejecutor se elige al pedir el inicio (`RunExecutor`); la ruta del
//!   adaptador vive en `run_settings` (validada en disco antes de guardarla). El supervisor
//!   registra lo que el agente declaró al negociar (`record_acp_session`, evento
//!   `acp_session`) y termina con motivos del núcleo (`RunFailure`, `RunOutcome`), nunca
//!   con texto del agente.
//! - Aprobaciones (HU-020, `approval`): peticiones de permiso del agente que decide una persona
//!   una sola vez (permitir una vez o rechazar), con caducidad, y que el supervisor entrega en
//!   una transacción que vuelve a comprobar el intento. Cancelar o terminar las cierra.
//! - Nada de esto se expone a agentes (servicio local o MCP).

mod acp_session;
mod approval;
mod cancel;
mod close;
mod create;
mod event;
mod execution_views;
mod get;
mod git_head;
mod list;
mod run;
mod runnable;
mod settings;
mod snapshot;
mod start;
mod supervisor;
mod worktree;

pub use acp_session::{
    ACP_CAPABILITIES, AcpAdapterCheck, AcpSessionInfo, MAX_ACP_CAPABILITIES, MAX_ACP_NAME_CHARS,
    MAX_ACP_VERSION_CHARS, ReportedSession, prompt_digest, record_acp_session,
};
pub use approval::{
    APPROVAL_TTL_MS, ApprovalAsk, ApprovalDecision, ApprovalResolution, ApprovalState,
    DecideApprovalInput, MAX_APPROVAL_SCOPE_CHARS, MAX_APPROVAL_TITLE_CHARS, MAX_APPROVALS_PER_RUN,
    MAX_PENDING_APPROVALS, RequestedApproval, RunApproval, WaitingApproval, cancel_approval,
    decide_approval, list_waiting_approvals, request_approval, resolve_approval, withdraw_approval,
};
pub use cancel::{CancelRunInput, cancel_run};
pub use create::{CreateRunInput, MAX_NOTE_CHARS, create_run};
pub use event::{
    MAX_EVENT_PAYLOAD_BYTES, RecordRunEventInput, RecordedRunEvent, RunEvent, record_run_event,
};
pub use execution_views::{
    RUN_OUTPUT_DIR, RemoveRunWorktreeInput, RunChange, RunOutputKind, RunOutputLine,
    RunOutputStream, RunOutputView, WorktreeChanges,
};
pub use get::{MAX_DETAIL_EVENTS, RunDetail, RunEventRecord, RunStartRequest, get_run};
pub use list::list_story_runs;
pub use run::{RunState, RunSummary, RunUsage};
pub use settings::{
    MAX_ADAPTER_PATH_CHARS, MAX_CONCURRENCY, MAX_TIMEOUT_MINUTES, MIN_CONCURRENCY,
    MIN_TIMEOUT_MINUTES, RunSettings, UpdateRunSettingsInput, check_adapter_path, get_run_settings,
    update_run_settings,
};
pub use snapshot::{
    RunSnapshot, SNAPSHOT_SCHEMA_VERSION, SnapshotAgent, SnapshotColumn, SnapshotCriterion,
    SnapshotEpic, SnapshotRepository, SnapshotStory, SnapshotStoryRef,
};
pub use start::{RunExecutor, StartRunInput, TestScenario, start_run};
pub use supervisor::{
    Dispatch, DispatchExecutor, DispatchOutcome, InterruptCause, PendingCancellation, PendingStart,
    RunFailure, RunOutcome, SupervisorToken, begin_dispatch, discard_unused_worktree, finish_run,
    interrupt_unverifiable_runs, mark_running, next_dispatchable, pending_cancellations,
    record_worktree, void_start_requests,
};
pub use worktree::{
    RecordWorktreeRemovalInput, RunWorktree, WORKTREES_DIR, WorktreeRemoval, WorktreeTarget,
    get_worktree_target, record_worktree_removal, worktrees_root,
};

pub(crate) use close::{StoryClosed, cancel_story_runs, interrupt_restored_runs};
// También la reevalúan `start_run` y `begin_dispatch`.
pub(crate) use runnable::ensure_runnable;
