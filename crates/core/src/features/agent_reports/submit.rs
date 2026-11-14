use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::features::access::AgentSession;
use crate::features::agent_activity::{ActivityKind, record_activity};
use crate::features::claims::{EndReason, end_claim, require_active_claim};
use crate::features::review::{
    DeliverySource, NewDelivery, ReferenceInput, insert_delivery, move_to_review,
    validate_references, validate_summary,
};
use crate::features::stories::load_story;
use crate::platform::storage::Store;

const OPERATION: &str = "agent_reports.submit";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitResultInput {
    /// Repítelo al reintentar la misma entrega.
    pub request_id: String,
    pub claim_id: String,
    /// Hasta 2 000 caracteres.
    pub summary: String,
    /// Hasta 10 referencias (URL http(s) o rutas locales absolutas), no se visitan.
    #[serde(default)]
    pub references: Vec<ReferenceInput>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    claim_id: &'a str,
    summary: &'a str,
    references: &'a [ReferenceInput],
}

/// Resultado de entregar. La entrega queda pendiente de revisión humana: no es una aceptación.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmittedResult {
    pub delivery_id: String,
    /// Número de la entrega en la historia (E-n).
    pub delivery_number: i64,
    pub story_id: String,
    pub story_number: i64,
    /// Si la tarjeta quedó en una columna «revisión» (no la hay, o estaba llena → `false`).
    pub in_review: bool,
}

/// Registra el resultado del agente como entrega `reported` pendiente de revisión, mueve la
/// tarjeta a la primera columna «revisión» si cabe (sin perder la entrega si el WIP está
/// lleno), quita el bloqueo que reportó el propio agente y cierra el reclamo (`delivered`); la
/// asignación se queda. Con la entrega pendiente, la historia deja de estar «lista». Exige el reclamo vigente:
/// con uno vencido o revocado no se escribe nada, así que nunca pisa el trabajo de otro agente.
pub async fn submit_result(
    store: &Store,
    session: &AgentSession,
    input: &SubmitResultInput,
) -> Result<SubmittedResult, CoreError> {
    let summary = validate_summary(&input.summary)?;
    let references = validate_references(&input.references)?;
    let key = session.receipt_key(OPERATION, &input.request_id)?;
    let payload = Payload {
        claim_id: &input.claim_id,
        summary: &summary,
        references: &references,
    };
    let (session, claim_id, clock) = (session.clone(), input.claim_id.clone(), store.clone());
    let id = uuid::Uuid::now_v7().to_string();
    let (summary_tx, references_tx) = (summary.clone(), references.clone());
    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let claim = require_active_claim(conn, &session, &claim_id, now).await?;
                let story = load_story(conn, session.project_id(), &claim.story_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Historia" })?;
                let in_review = move_to_review(conn, session.project_id(), &story, false).await?;
                let number = insert_delivery(
                    conn,
                    &NewDelivery {
                        id: &id,
                        project_id: session.project_id(),
                        story_id: &story.id,
                        source: DeliverySource::Reported,
                        agent_id: Some(session.agent_id()),
                        summary: &summary_tx,
                        references: &references_tx,
                    },
                )
                .await?;
                // Entregar quita el bloqueo que reportó este agente (decisión del propietario):
                // sin reclamo ya no podría quitarlo. Uno de una persona u otro agente se queda.
                let cleared = sqlx::query(
                    "DELETE FROM story_blockers \
                     WHERE story_id = ? AND source = 'reported' AND agent_id = ?",
                )
                .bind(&story.id)
                .bind(session.agent_id())
                .execute(&mut *conn)
                .await?
                .rows_affected();
                if cleared > 0 {
                    record_activity(
                        conn,
                        &session,
                        &story.id,
                        ActivityKind::BlockerCleared,
                        None,
                        now,
                    )
                    .await?;
                }
                end_claim(conn, &claim.claim_id, EndReason::Delivered, now).await?;
                record_activity(
                    conn,
                    &session,
                    &story.id,
                    ActivityKind::Delivered,
                    Some(&format!("E{number}")),
                    now,
                )
                .await?;
                Ok(SubmittedResult {
                    delivery_id: id,
                    delivery_number: number,
                    story_id: story.id,
                    story_number: story.number,
                    in_review,
                })
            })
        })
        .await
}

impl SubmittedResult {
    /// Historias que cambió la entrega (para `AgentChange`).
    pub fn story_ids(&self) -> Vec<String> {
        vec![self.story_id.clone()]
    }
}
