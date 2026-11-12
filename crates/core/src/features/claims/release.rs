use serde::Deserialize;
use ts_rs::TS;

use super::story_claim::{ENTITY, validate_id};
use crate::error::CoreError;
use crate::platform::actor::Actor;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReleaseClaimInput {
    pub project_id: String,
    pub claim_id: String,
}

/// Una persona libera un reclamo: la historia deja de estar reservada, se quita la asignación
/// a ese agente (decisión del propietario: liberar = quitar al agente; si no, el mismo agente
/// la volvería a reclamar enseguida) y el agente recibe `claim_ended` al renovar. Idempotente:
/// liberar uno ya cerrado no hace nada. Si ya había vencido, se registra como vencido.
pub async fn release_claim(
    store: &Store,
    actor: &Actor,
    input: &ReleaseClaimInput,
) -> Result<(), CoreError> {
    actor.require_human("liberar el reclamo de un agente")?;
    validate_id("claimId", &input.claim_id)?;
    let (input, clock) = (input.clone(), store.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM story_claims WHERE id = ? AND project_id = ?)",
                )
                .bind(&input.claim_id)
                .bind(&input.project_id)
                .fetch_one(&mut *conn)
                .await?;
                if !exists {
                    return Err(CoreError::NotFound { entity: ENTITY });
                }
                let closed = sqlx::query(
                    "UPDATE story_claims SET \
                         ended_at_ms = MIN(?, expires_at_ms), \
                         end_reason = CASE WHEN expires_at_ms <= ? THEN 'expired' \
                                           ELSE 'released' END \
                     WHERE id = ? AND ended_at_ms IS NULL",
                )
                .bind(now)
                .bind(now)
                .bind(&input.claim_id)
                .execute(&mut *conn)
                .await?
                .rows_affected();
                if closed > 0 {
                    // Solo si la historia sigue asignada al agente del reclamo.
                    sqlx::query(
                        "DELETE FROM story_agents WHERE (story_id, agent_id) IN \
                         (SELECT story_id, agent_id FROM story_claims WHERE id = ?)",
                    )
                    .bind(&input.claim_id)
                    .execute(&mut *conn)
                    .await?;
                }
                Ok(())
            })
        })
        .await
}
