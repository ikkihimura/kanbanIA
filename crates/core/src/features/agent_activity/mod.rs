//! Slice `agent_activity`: lo que los agentes reportan (HU-015) y el aviso de cambios.
//!
//! - `agent_activity` guarda las acciones de agentes (reclamar, renovar, bloqueos, subtareas,
//!   entregas) para la UI. Son afirmaciones del agente: la UI muestra «última interacción
//!   reportada», nunca que el proceso siga en marcha. `detail` lo genera el núcleo.
//! - `AgentChange` es el aviso que el host emite a la UI tras una mutación de agente; su
//!   contenido (proyecto e historias afectadas) lo decide el núcleo con `story_ids()` de cada
//!   resultado.
//! - Slice hoja: solo depende de `access::AgentSession`, `team` y `platform`. `claims` y
//!   `agent_reports` lo usan; él no usa a ninguno.

mod activity;
mod change;

pub use activity::{
    ActivityEntry, ActivityKind, StoryActivity, StoryActivityInput, list_story_activity,
};
pub use change::AgentChange;

pub(crate) use activity::record_activity;
