use serde::Serialize;
use ts_rs::TS;

use super::authenticate::AgentSession;
use crate::error::CoreError;
use crate::platform::storage::Store;

/// Quién es la sesión y a qué proyecto accede (para que el agente lo confirme).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionInfo {
    pub agent_id: String,
    #[ts(type = "number")]
    pub agent_number: i64,
    pub agent_name: String,
    pub project_id: String,
    pub project_name: String,
}

/// Estado del servicio local para la UI. El puerto no es secreto; el token sí y no sale de aquí.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LocalServiceStatus {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub port: Option<u16>,
}

pub async fn describe_session(
    store: &Store,
    session: &AgentSession,
) -> Result<SessionInfo, CoreError> {
    sqlx::query_as(
        "SELECT a.id AS agent_id, a.number AS agent_number, a.name AS agent_name, \
         p.id AS project_id, p.name AS project_name \
         FROM agents a, projects p WHERE a.id = ? AND p.id = ?",
    )
    .bind(session.agent_id())
    .bind(session.project_id())
    .fetch_optional(store.reader())
    .await?
    .ok_or(CoreError::Unauthorized)
}
