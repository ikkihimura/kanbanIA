//! Slice `runs`: intentos de ejecución de una historia por un agente (HU-017). Solo el modelo:
//! todavía no hay ejecutor ni supervisor (HU-018/019), así que un intento nace en `queued` y
//! se queda en cola hasta que una persona lo cancela.
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
//! - Precondiciones en `ensure_runnable` (HU-018 la vuelve a evaluar al despachar).
//! - Crear y cancelar exigen `Actor::Human` y son idempotentes por `requestId`. Aceptar o
//!   archivar la historia cancela su intento activo (`cancel_story_runs`).
//! - Nada de esto se expone a agentes (servicio local o MCP) en este hito.

mod cancel;
mod close;
mod create;
mod event;
mod get;
mod git_head;
mod list;
mod run;
mod runnable;
mod snapshot;

pub use cancel::{CancelRunInput, cancel_run};
pub use create::{CreateRunInput, MAX_NOTE_CHARS, create_run};
pub use event::{
    MAX_EVENT_PAYLOAD_BYTES, RecordRunEventInput, RecordedRunEvent, RunEvent, record_run_event,
};
pub use get::{MAX_DETAIL_EVENTS, RunDetail, RunEventRecord, get_run};
pub use list::list_story_runs;
pub use run::{RunState, RunSummary, RunUsage};
pub use snapshot::{
    RunSnapshot, SNAPSHOT_SCHEMA_VERSION, SnapshotAgent, SnapshotColumn, SnapshotCriterion,
    SnapshotEpic, SnapshotRepository, SnapshotStory, SnapshotStoryRef,
};

pub(crate) use close::{StoryClosed, cancel_story_runs, interrupt_restored_runs};
// HU-018 debe volver a evaluarla al despachar un intento en cola.
pub(crate) use runnable::ensure_runnable;
