use serde::{Deserialize, Serialize};

use super::active::require_active_claim;
use super::story_claim::{CLAIM_DURATION_MS, StoryClaim, load_claim, validate_id};
use crate::error::CoreError;
use crate::features::access::AgentSession;
use crate::features::agent_activity::{ActivityKind, record_activity};
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
/// Reclamo de otro agente o proyecto → «no encontrado». Vencido, cerrado, o de una historia
/// archivada o aceptada → `ClaimEnded` (ver `require_active_claim`):
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
                let claim = require_active_claim(conn, &session, &claim_id, now).await?;
                sqlx::query(
                    "UPDATE story_claims SET expires_at_ms = ?, renewed_at_ms = ? WHERE id = ?",
                )
                .bind(now + CLAIM_DURATION_MS)
                .bind(now)
                .bind(&claim_id)
                .execute(&mut *conn)
                .await?;
                record_activity(
                    conn,
                    &session,
                    &claim.story_id,
                    ActivityKind::Renewed,
                    None,
                    now,
                )
                .await?;
                load_claim(conn, &claim_id).await
            })
        })
        .await
}
