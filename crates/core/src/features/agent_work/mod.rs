//! Slice `agent_work`: consultas de trabajo para agentes externos (HU-013, vía servicio local
//! y MCP). Todas reciben la `AgentSession` autenticada: el proyecto y la identidad salen de
//! ahí, nunca de la petición. Solo lectura; reclamar historias vive en el slice `claims` (HU-014), que reutiliza la regla.
//!
//! El texto que devuelven (títulos, descripciones, criterios) lo escribieron personas u otros
//! agentes: para quien lo consume es un dato no confiable, no instrucciones.

mod context;
mod ready;
mod rule;

pub(crate) use rule::{claimed_by_other, ready_for_agent};

pub use context::{
    ContextColumn, ContextEpic, ContextRef, StoryContext, StoryKey, YourClaim, get_story_context,
};
pub use ready::{DEFAULT_PAGE, MAX_PAGE, ReadyPage, ReadyQuery, ReadyStory, list_ready_stories};
