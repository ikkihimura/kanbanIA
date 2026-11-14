//! Slice `agent_reports`: lo que un agente reporta al tablero por MCP (HU-015).
//!
//! - `report_blocker`, `create_subtask` y `submit_result` reciben la `AgentSession` y un
//!   `claimId`: exigen un reclamo vigente del agente (`claims::require_active_claim`). Con un
//!   reclamo vencido, cerrado o revocado no escriben nada (decisión del propietario).
//! - Son reintentables con `requestId` (recibo por credencial, `AgentSession::receipt_key`).
//! - Todo lo que registran es `reported`: afirmaciones del agente, no hechos observados. Un
//!   agente nunca acepta: entregar deja la entrega pendiente de revisión humana.
//! - Entregar cierra el reclamo (`delivered`) y deja la asignación (decisión del propietario).
//! - Las subtareas nacen en la columna por defecto (backlog), sin agente, como hijas de la
//!   historia reclamada, para que otro agente pueda reclamarlas (decisión del propietario).
//! - Cada operación registra su actividad (`agent_activity::record_activity`) y su resultado
//!   dice qué historias cambió (`story_ids()`), para el aviso a la UI.

mod blocker;
mod submit;
mod subtask;

pub use blocker::{ReportBlockerInput, ReportedBlocker, report_blocker};
pub use submit::{SubmitResultInput, SubmittedResult, submit_result};
pub use subtask::{
    CreateSubtaskInput, CreatedSubtask, MAX_PENDING_AGENT_SUBTASKS, MAX_SUBTASK_DESCRIPTION_CHARS,
    MAX_SUBTASKS_PER_STORY, create_subtask,
};
