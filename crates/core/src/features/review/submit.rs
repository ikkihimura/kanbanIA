use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::delivery::{
    Delivery, DeliverySource, ReferenceInput, deliveries_of_story, resolve_reference_status,
    validate_references, validate_summary,
};
use super::{first_column_of, source_of};
use crate::error::CoreError;
use crate::features::columns::{ColumnCategory, ensure_capacity};
use crate::features::stories::{StorySummary, append_to_column, load_story};
use crate::platform::actor::Actor;
use crate::platform::idempotency::MutationKey;
use crate::platform::storage::Store;

const OPERATION: &str = "deliveries.submit";

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SubmitDeliveryInput {
    pub request_id: String,
    pub project_id: String,
    pub story_id: String,
    pub summary: String,
    pub references: Vec<ReferenceInput>,
    /// Mover la tarjeta a la primera columna «revisión» (si hay y no está ya en una).
    /// No marca la historia como hecha.
    pub send_to_review: bool,
}

/// Registra una entrega pendiente de revisión. Una persona la registra como `manual`; un
/// agente, como `reported` (es su afirmación, no algo observado por la app).
pub async fn submit_delivery(
    store: &Store,
    actor: &Actor,
    input: &SubmitDeliveryInput,
) -> Result<Delivery, CoreError> {
    // Los agentes entregan con `agent_reports::submit_result`, que exige su reclamo vigente.
    actor.require_human("registrar una entrega desde la app")?;
    let key = MutationKey::new(actor.receipt_key(), OPERATION, &input.request_id)?;
    let summary = validate_summary(&input.summary)?;
    let references = validate_references(&input.references)?;
    let canonical = SubmitDeliveryInput {
        summary: summary.clone(),
        references: references.clone(),
        ..input.clone()
    };
    let actor_for_status = actor;
    let actor = actor.clone();
    let id = uuid::Uuid::now_v7().to_string();
    let input = input.clone();

    let delivery_id = store
        .write_idempotent(&key, &canonical, move |conn| {
            Box::pin(async move {
                let (source, agent_id) = source_of(conn, &actor).await?;
                let story = load_story(conn, &input.project_id, &input.story_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Historia" })?;
                if story.archived_at.is_some() {
                    return Err(CoreError::Validation {
                        field: "storyId",
                        reason: format!("La historia #{} está archivada.", story.number),
                    });
                }
                if story.accepted_at.is_some() {
                    return Err(CoreError::Validation {
                        field: "storyId",
                        reason: format!(
                            "La historia #{} ya está aceptada: reábrela para entregar de nuevo.",
                            story.number
                        ),
                    });
                }

                if input.send_to_review {
                    move_to_review(conn, &input.project_id, &story, true).await?;
                }
                insert_delivery(
                    conn,
                    &NewDelivery {
                        id: &id,
                        project_id: &input.project_id,
                        story_id: &story.id,
                        source,
                        agent_id: agent_id.as_deref(),
                        summary: &summary,
                        references: &references,
                    },
                )
                .await?;
                Ok(id)
            })
        })
        .await?;

    // Se lee tras confirmar y fuera de la transacción (las rutas se comprueban en disco).
    let mut conn = store.reader().acquire().await?;
    let mut deliveries =
        deliveries_of_story(&mut conn, &canonical.project_id, &canonical.story_id).await?;
    drop(conn);
    deliveries.retain(|d| d.id == delivery_id);
    // A un agente no se le dice qué rutas existen (evita usarlo para sondear el disco).
    if *actor_for_status == Actor::Human {
        resolve_reference_status(&mut deliveries).await;
    }
    deliveries
        .pop()
        .ok_or(CoreError::NotFound { entity: "Entrega" })
}

/// Entrega a insertar dentro de una transacción.
pub(crate) struct NewDelivery<'a> {
    pub id: &'a str,
    pub project_id: &'a str,
    pub story_id: &'a str,
    pub source: DeliverySource,
    pub agent_id: Option<&'a str>,
    pub summary: &'a str,
    pub references: &'a [ReferenceInput],
}

/// Inserta la entrega y sus referencias (ya validadas). Devuelve su número (E-n).
pub(crate) async fn insert_delivery(
    conn: &mut SqliteConnection,
    delivery: &NewDelivery<'_>,
) -> Result<i64, CoreError> {
    let number: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(number), 0) + 1 FROM deliveries WHERE story_id = ?",
    )
    .bind(delivery.story_id)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO deliveries (id, project_id, story_id, number, source, agent_id, \
         summary) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(delivery.id)
    .bind(delivery.project_id)
    .bind(delivery.story_id)
    .bind(number)
    .bind(delivery.source.as_str())
    .bind(delivery.agent_id)
    .bind(delivery.summary)
    .execute(&mut *conn)
    .await?;
    for (position, reference) in (0_i64..).zip(delivery.references) {
        sqlx::query(
            "INSERT INTO delivery_references (delivery_id, position, kind, value) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(delivery.id)
        .bind(position)
        .bind(reference.kind.as_str())
        .bind(&reference.value)
        .execute(&mut *conn)
        .await?;
    }
    Ok(number)
}

/// Mueve la tarjeta a la primera columna «revisión» si hay una y no está ya en ella.
/// Con `strict`, una columna llena devuelve el error de WIP; sin él, la tarjeta se queda donde
/// está (la entrega de un agente no se pierde por el WIP). Devuelve si la tarjeta está en
/// revisión al terminar.
pub(crate) async fn move_to_review(
    conn: &mut SqliteConnection,
    project_id: &str,
    story: &StorySummary,
    strict: bool,
) -> Result<bool, CoreError> {
    let Some(review) = first_column_of(conn, project_id, ColumnCategory::Review).await? else {
        return Ok(false);
    };
    let in_review =
        sqlx::query_scalar::<_, bool>("SELECT category = 'review' FROM columns WHERE id = ?")
            .bind(&story.column_id)
            .fetch_one(&mut *conn)
            .await?;
    if in_review {
        return Ok(true);
    }
    match ensure_capacity(conn, &review, 1).await {
        Ok(()) => {}
        Err(CoreError::WipLimitExceeded { .. }) if !strict => return Ok(false),
        Err(error) => return Err(error),
    }
    append_to_column(conn, story, &review.id).await?;
    sqlx::query("UPDATE stories SET revision = revision + 1 WHERE id = ?")
        .bind(&story.id)
        .execute(&mut *conn)
        .await?;
    Ok(true)
}
