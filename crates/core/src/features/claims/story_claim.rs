use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use crate::error::CoreError;
pub use crate::features::team::ClaimOwner;

pub(super) const ENTITY: &str = "Reclamo";
/// Duración de un reclamo y de cada renovación (decisión del propietario).
pub const CLAIM_DURATION_MINUTES: i64 = 30;
pub(super) const CLAIM_DURATION_MS: i64 = CLAIM_DURATION_MINUTES * 60 * 1000;
pub(super) const MAX_ID_LEN: usize = 64;
/// Reclamos vigentes a la vez por agente y proyecto: un agente (con fallos o manipulado por el
/// texto de una historia) no puede reservar todo el backlog.
pub const MAX_OPEN_CLAIMS: i64 = 5;

/// Reclamo de una historia. Mientras `expiresAt` no pase, ningún otro agente puede tomarla.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoryClaim {
    pub claim_id: String,
    pub story_id: String,
    #[ts(type = "number")]
    pub story_number: i64,
    pub owner: ClaimOwner,
    pub claimed_at: String,
    pub expires_at: String,
    pub renewed_at: Option<String>,
}

#[derive(sqlx::FromRow)]
pub(super) struct ClaimRow {
    claim_id: String,
    story_id: String,
    story_number: i64,
    agent_id: String,
    agent_number: i64,
    agent_name: String,
    claimed_at: String,
    expires_at: String,
    renewed_at: Option<String>,
}

impl From<ClaimRow> for StoryClaim {
    fn from(row: ClaimRow) -> Self {
        Self {
            claim_id: row.claim_id,
            story_id: row.story_id,
            story_number: row.story_number,
            owner: ClaimOwner {
                agent_id: row.agent_id,
                agent_number: row.agent_number,
                agent_name: row.agent_name,
            },
            claimed_at: row.claimed_at,
            expires_at: row.expires_at,
            renewed_at: row.renewed_at,
        }
    }
}

macro_rules! select_claim {
    ($tail:literal) => {
        concat!(
            "SELECT k.id AS claim_id, k.story_id, s.number AS story_number, \
             a.id AS agent_id, a.number AS agent_number, a.name AS agent_name, ",
            $crate::platform::clock::iso_from_ms!("k.claimed_at_ms"),
            " AS claimed_at, ",
            $crate::platform::clock::iso_from_ms!("k.expires_at_ms"),
            " AS expires_at, ",
            $crate::platform::clock::iso_from_ms!("k.renewed_at_ms"),
            " AS renewed_at \
             FROM story_claims k \
             JOIN stories s ON s.id = k.story_id \
             JOIN agents a ON a.id = k.agent_id ",
            $tail
        )
    };
}
pub(super) use select_claim;

pub(super) async fn load_claim(
    conn: &mut SqliteConnection,
    claim_id: &str,
) -> Result<StoryClaim, CoreError> {
    let row: Option<ClaimRow> = sqlx::query_as(select_claim!("WHERE k.id = ?"))
        .bind(claim_id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(StoryClaim::from)
        .ok_or(CoreError::NotFound { entity: ENTITY })
}

/// Id recibido de fuera: acotado antes de usarlo.
pub(super) fn validate_id(field: &'static str, id: &str) -> Result<(), CoreError> {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return Err(CoreError::Validation {
            field,
            reason: format!("{field} no válido."),
        });
    }
    Ok(())
}

impl StoryClaim {
    /// Historias que cambió reclamar o renovar (para `AgentChange`).
    pub fn story_ids(&self) -> Vec<String> {
        vec![self.story_id.clone()]
    }
}
