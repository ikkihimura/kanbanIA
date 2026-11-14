use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Agente que reclamó una historia o reportó una acción.
// Vive en `team` para que ni `claims` ni `agent_activity` dependan del otro.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ClaimOwner {
    pub agent_id: String,
    /// Se muestra como A-n.
    #[ts(type = "number")]
    pub agent_number: i64,
    pub agent_name: String,
}
