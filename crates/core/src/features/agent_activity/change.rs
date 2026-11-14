use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::features::access::AgentSession;

/// Aviso de que un agente cambió el tablero (reclamar, renovar, bloqueo, subtarea, entrega).
/// El transporte lo emite tras confirmar la operación; la app lo usa solo para recargar (el
/// núcleo sigue siendo la autoridad). No lleva texto del agente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentChange {
    pub project_id: String,
    /// Historias afectadas (la reclamada y, al crear una subtarea, también la nueva).
    pub story_ids: Vec<String>,
}

impl AgentChange {
    pub fn new(session: &AgentSession, story_ids: Vec<String>) -> Self {
        Self {
            project_id: session.project_id().to_owned(),
            story_ids,
        }
    }
}
