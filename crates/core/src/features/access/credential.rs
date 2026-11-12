use serde::Serialize;
use sqlx::SqliteConnection;
use ts_rs::TS;

use crate::error::CoreError;

pub(super) const ENTITY: &str = "Credencial";

/// Credenciales activas (sin revocar) por proyecto.
pub const MAX_ACTIVE_PER_PROJECT: i64 = 20;

/// Credencial de un agente para un proyecto. Nunca incluye el token ni su hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentCredential {
    pub id: String,
    pub project_id: String,
    pub agent_id: String,
    pub agent_name: String,
    /// Número legible del agente (A-n).
    #[ts(type = "number")]
    pub agent_number: i64,
    /// Últimos 4 caracteres del token, para reconocerlo.
    pub token_hint: String,
    pub created_at: String,
    /// Último uso observado por el servicio local (con un minuto de resolución).
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

/// SQL estático (sqlx 0.9 rechaza cadenas dinámicas): `select_credential!("WHERE ...")`.
macro_rules! select_credential {
    ($tail:literal) => {
        concat!(
            "SELECT c.id, c.project_id, c.agent_id, a.name AS agent_name, \
             a.number AS agent_number, c.token_hint, c.created_at, c.last_used_at, \
             c.revoked_at \
             FROM agent_credentials c JOIN agents a ON a.id = c.agent_id ",
            $tail
        )
    };
}
pub(super) use select_credential;

pub(super) async fn load_credential(
    conn: &mut SqliteConnection,
    project_id: &str,
    id: &str,
) -> Result<AgentCredential, CoreError> {
    sqlx::query_as(select_credential!("WHERE c.project_id = ? AND c.id = ?"))
        .bind(project_id)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(CoreError::NotFound { entity: ENTITY })
}
