use serde::Deserialize;
use ts_rs::TS;

use super::credential::{AgentCredential, load_credential};
use crate::error::CoreError;
use crate::features::claims::close_credential_claims;
use crate::platform::actor::Actor;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RevokeAgentCredentialInput {
    pub project_id: String,
    pub id: String,
}

/// Revoca una credencial: la siguiente petición con ese token falla y sus reclamos abiertos
/// se cierran (`revoked`). Idempotente.
pub async fn revoke_agent_credential(
    store: &Store,
    actor: &Actor,
    input: &RevokeAgentCredentialInput,
) -> Result<AgentCredential, CoreError> {
    actor.require_human("revocar credenciales de agentes")?;
    let (project_id, id, clock) = (input.project_id.clone(), input.id.clone(), store.clone());
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
                // Sus reclamos dejan de valer ya, no al vencer (HU-015). Solo los de este
                // proyecto: la credencial está ligada a él.
                if credential_exists(conn, &project_id, &id).await? {
                    close_credential_claims(conn, &id, clock.now_ms()).await?;
                }
                load_credential(conn, &project_id, &id).await
            })
        })
        .await
}

async fn credential_exists(
    conn: &mut sqlx::SqliteConnection,
    project_id: &str,
    id: &str,
) -> Result<bool, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM agent_credentials WHERE project_id = ? AND id = ?)",
    )
    .bind(project_id)
    .bind(id)
    .fetch_one(&mut *conn)
    .await?)
}
