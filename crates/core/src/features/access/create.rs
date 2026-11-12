use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::credential::{AgentCredential, MAX_ACTIVE_PER_PROJECT, load_credential};
use super::token;
use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::features::team::load_agent;
use crate::platform::actor::Actor;
use crate::platform::storage::Store;

/// Sin `requestId`: un recibo de idempotencia guardaría el token en claro. Repetir la
/// creación solo produce otra credencial, que se puede revocar.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateAgentCredentialInput {
    pub project_id: String,
    pub agent_id: String,
}

/// Resultado de crear: el token solo viaja aquí, una vez.
#[derive(Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreatedAgentCredential {
    pub credential: AgentCredential,
    pub token: String,
}

impl std::fmt::Debug for CreatedAgentCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreatedAgentCredential")
            .field("credential", &self.credential)
            .field("token", &"<oculto>")
            .finish()
    }
}

pub async fn create_agent_credential(
    store: &Store,
    actor: &Actor,
    input: &CreateAgentCredentialInput,
) -> Result<CreatedAgentCredential, CoreError> {
    actor.require_human("crear credenciales de agentes")?;
    let new = token::generate()?;
    let id = uuid::Uuid::now_v7().to_string();
    let (project_id, agent_id) = (input.project_id.clone(), input.agent_id.clone());
    let (hash, hint) = (new.hash, new.hint);

    let credential = store
        .write(move |conn| {
            Box::pin(async move {
                ensure_project_exists(conn, &project_id).await?;
                if load_agent(conn, &agent_id).await?.is_none() {
                    return Err(CoreError::NotFound { entity: "Agente" });
                }
                let active: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM agent_credentials \
                     WHERE project_id = ? AND revoked_at IS NULL",
                )
                .bind(&project_id)
                .fetch_one(&mut *conn)
                .await?;
                if active >= MAX_ACTIVE_PER_PROJECT {
                    return Err(CoreError::Validation {
                        field: "agentId",
                        reason: format!(
                            "Este proyecto ya tiene {MAX_ACTIVE_PER_PROJECT} credenciales activas. \
                             Revoca alguna antes de crear otra."
                        ),
                    });
                }
                sqlx::query(
                    "INSERT INTO agent_credentials (id, project_id, agent_id, token_hash, token_hint) \
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&project_id)
                .bind(&agent_id)
                .bind(&hash)
                .bind(&hint)
                .execute(&mut *conn)
                .await?;
                load_credential(conn, &project_id, &id).await
            })
        })
        .await?;

    Ok(CreatedAgentCredential {
        credential,
        token: new.token,
    })
}
