use sqlx::SqliteConnection;

use super::story_claim::{ENTITY, validate_id};
use crate::error::CoreError;
use crate::features::access::AgentSession;

/// Reclamo vigente del agente de la sesión, comprobado dentro de una transacción de escritura.
#[derive(Debug, Clone)]
pub(crate) struct ActiveClaim {
    pub claim_id: String,
    pub story_id: String,
    pub story_number: i64,
}

#[derive(sqlx::FromRow)]
struct ClaimState {
    project_id: String,
    agent_id: String,
    story_id: String,
    story_number: i64,
    expires_at_ms: i64,
    end_reason: Option<String>,
    closed: bool,
}

/// Exige que `claim_id` sea un reclamo abierto y sin vencer del agente y proyecto de la
/// sesión, sobre una historia ni archivada ni aceptada. Es la puerta de toda operación de
/// agentes sobre una historia (renovar, bloqueos, subtareas, entregas, HU-015):
///
/// - de otro agente, de otro proyecto o inexistente → «no encontrado» (sin distinguir);
/// - vencido → `ClaimEnded { expired: true }` (no se resucita: hay que reclamar de nuevo);
/// - cerrado por otro motivo, o historia archivada/aceptada → `ClaimEnded { expired: false }`.
///
/// Así un agente con un reclamo vencido o revocado nunca escribe sobre una historia que ya
/// trabaja otro.
pub(crate) async fn require_active_claim(
    conn: &mut SqliteConnection,
    session: &AgentSession,
    claim_id: &str,
    now: i64,
) -> Result<ActiveClaim, CoreError> {
    validate_id("claimId", claim_id)?;
    let row: Option<ClaimState> = sqlx::query_as(
        "SELECT k.project_id, k.agent_id, k.story_id, s.number AS story_number, \
         k.expires_at_ms, k.end_reason, \
         (s.archived_at IS NOT NULL OR s.accepted_at IS NOT NULL) AS closed \
         FROM story_claims k JOIN stories s ON s.id = k.story_id WHERE k.id = ?",
    )
    .bind(claim_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(ClaimState {
        project_id,
        agent_id,
        story_id,
        story_number,
        expires_at_ms: expires_at,
        end_reason,
        closed,
    }) = row
    else {
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
    if closed {
        return Err(CoreError::ClaimEnded { expired: false });
    }
    Ok(ActiveClaim {
        claim_id: claim_id.to_owned(),
        story_id,
        story_number,
    })
}
