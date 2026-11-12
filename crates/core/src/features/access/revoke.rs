use serde::Deserialize;
use ts_rs::TS;

use super::credential::{AgentCredential, load_credential};
use crate::error::CoreError;
use crate::platform::actor::Actor;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RevokeAgentCredentialInput {
    pub project_id: String,
    pub id: String,
}

/// Revoca una credencial: la siguiente petición con ese token falla. Idempotente.
pub async fn revoke_agent_credential(
    store: &Store,
    actor: &Actor,
    input: &RevokeAgentCredentialInput,
) -> Result<AgentCredential, CoreError> {
    actor.require_human("revocar credenciales de agentes")?;
    let (project_id, id) = (input.project_id.clone(), input.id.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                sqlx::query(
                    "UPDATE agent_credentials \
                     SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE project_id = ? AND id = ? AND revoked_at IS NULL",
                )
                .bind(&project_id)
                .bind(&id)
                .execute(&mut *conn)
                .await?;
                load_credential(conn, &project_id, &id).await
            })
        })
        .await
}
