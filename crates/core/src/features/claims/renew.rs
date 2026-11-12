use serde::{Deserialize, Serialize};

use super::story_claim::{CLAIM_DURATION_MS, ENTITY, StoryClaim, load_claim, validate_id};
use crate::error::CoreError;
use crate::features::access::AgentSession;
use crate::platform::storage::Store;

const OPERATION: &str = "claims.renew";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenewClaimInput {
    /// Repítelo al reintentar la misma renovación.
    pub request_id: String,
    pub claim_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    claim_id: &'a str,
}

/// Extiende un reclamo vigente del agente de la sesión otros 30 minutos desde ahora.
/// Reclamo de otro agente o proyecto → «no encontrado». Vencido, liberado, o de una historia
/// archivada o aceptada → `ClaimEnded`:
/// un reclamo vencido no se resucita; hay que reclamar de nuevo (y la regla vuelve a aplicarse).
pub async fn renew_claim(
    store: &Store,
    session: &AgentSession,
    input: &RenewClaimInput,
) -> Result<StoryClaim, CoreError> {
    validate_id("claimId", &input.claim_id)?;
    let key = session.receipt_key(OPERATION, &input.request_id)?;
    let payload = Payload {
        claim_id: &input.claim_id,
    };
    let (session, claim_id, clock) = (session.clone(), input.claim_id.clone(), store.clone());
    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let row: Option<(String, String, i64, Option<String>, bool)> = sqlx::query_as(
                    "SELECT k.project_id, k.agent_id, k.expires_at_ms, k.end_reason, \
                     (s.archived_at IS NOT NULL OR s.accepted_at IS NOT NULL) AS closed \
                     FROM story_claims k JOIN stories s ON s.id = k.story_id WHERE k.id = ?",
                )
                .bind(&claim_id)
                .fetch_optional(&mut *conn)
                .await?;
                let Some((project_id, agent_id, expires_at, end_reason, closed)) = row else {
                    return Err(CoreError::NotFound { entity: ENTITY });
                };
                if project_id != session.project_id() || agent_id != session.agent_id() {
                    return Err(CoreError::NotFound { entity: ENTITY });
                }
                if let Some(reason) = end_reason {
                    return Err(CoreError::ClaimEnded {
                        expired: reason == "expired",
                    });
                }
                if expires_at <= now {
                    return Err(CoreError::ClaimEnded { expired: true });
                }
                // Archivada o aceptada: ya no hay trabajo que reservar.
                if closed {
                    return Err(CoreError::ClaimEnded { expired: false });
                }
                sqlx::query(
                    "UPDATE story_claims SET expires_at_ms = ?, renewed_at_ms = ? WHERE id = ?",
                )
                .bind(now + CLAIM_DURATION_MS)
                .bind(now)
                .bind(&claim_id)
                .execute(&mut *conn)
                .await?;
                load_claim(conn, &claim_id).await
            })
        })
        .await
}
