use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::features::access::AgentSession;
use crate::features::agent_activity::{ActivityKind, record_activity};
use crate::features::claims::require_active_claim;
use crate::features::review::{validate_reason, write_blocker};
use crate::features::stories::load_story;
use crate::platform::storage::Store;

const OPERATION: &str = "agent_reports.blocker";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportBlockerInput {
    /// Repítelo al reintentar el mismo reporte.
    pub request_id: String,
    pub claim_id: String,
    /// Motivo (una línea, hasta 500 caracteres). `null` u omitido: quita tu propio bloqueo.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    claim_id: &'a str,
    reason: Option<&'a str>,
}

/// Estado del bloqueo tras el reporte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportedBlocker {
    pub story_id: String,
    pub story_number: i64,
    pub blocked: bool,
    pub reason: Option<String>,
}

/// Marca la historia reclamada como bloqueada (`reported`) o quita el bloqueo propio. No
/// puede cambiar ni quitar un bloqueo de una persona u otro agente (`forbidden`). El reclamo
/// sigue vigente: el agente lo renueva mientras espera.
pub async fn report_blocker(
    store: &Store,
    session: &AgentSession,
    input: &ReportBlockerInput,
) -> Result<ReportedBlocker, CoreError> {
    let reason = input.reason.as_deref().map(validate_reason).transpose()?;
    let key = session.receipt_key(OPERATION, &input.request_id)?;
    let payload = Payload {
        claim_id: &input.claim_id,
        reason: reason.as_deref(),
    };
    let (session, claim_id, clock) = (session.clone(), input.claim_id.clone(), store.clone());
    let reason_tx = reason.clone();
    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let claim = require_active_claim(conn, &session, &claim_id, now).await?;
                let story = load_story(conn, session.project_id(), &claim.story_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Historia" })?;
                // Solo se registra actividad si el bloqueo cambia (repetir el mismo motivo con
                // otro `requestId` no añade filas).
                let previous = story.blocker_reason.clone();
                write_blocker(
                    conn,
                    &session.actor(),
                    session.project_id(),
                    &story,
                    reason_tx.as_deref(),
                )
                .await?;
                let kind = match (&reason_tx, &previous) {
                    (Some(new), Some(old)) if new == old => None,
                    (Some(_), _) => Some(ActivityKind::BlockerReported),
                    (None, Some(_)) => Some(ActivityKind::BlockerCleared),
                    (None, None) => None,
                };
                if let Some(kind) = kind {
                    record_activity(conn, &session, &story.id, kind, None, now).await?;
                }
                Ok(ReportedBlocker {
                    story_id: story.id,
                    story_number: story.number,
                    blocked: reason_tx.is_some(),
                    reason: reason_tx,
                })
            })
        })
        .await
}

impl ReportedBlocker {
    /// Historias que cambió el reporte (para `AgentChange`).
    pub fn story_ids(&self) -> Vec<String> {
        vec![self.story_id.clone()]
    }
}
