//! Devolver entregas y marcar/quitar bloqueos.

use serde::Deserialize;
use ts_rs::TS;

use super::source_of;
use crate::error::CoreError;
use crate::features::stories::load_story;
use crate::platform::actor::Actor;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReturnDeliveryInput {
    pub project_id: String,
    pub delivery_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetBlockerInput {
    pub project_id: String,
    pub story_id: String,
    /// `null` quita el bloqueo.
    pub reason: Option<String>,
}

fn validate_reason(raw: &str) -> Result<String, CoreError> {
    let reason = raw.trim();
    if reason.is_empty()
        || reason.chars().count() > 500
        || reason.chars().any(char::is_control)
        || super::has_invisible_formatting(reason)
    {
        return Err(CoreError::Validation {
            field: "reason",
            reason: "El motivo es obligatorio (hasta 500 caracteres, una línea).".into(),
        });
    }
    Ok(reason.to_owned())
}

/// Devuelve una entrega pendiente con un motivo. Es una decisión de revisión: solo personas.
/// No mueve la tarjeta.
pub async fn return_delivery(
    store: &Store,
    actor: &Actor,
    input: &ReturnDeliveryInput,
) -> Result<(), CoreError> {
    actor.require_human("devolver una entrega")?;
    let reason = validate_reason(&input.reason)?;
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let status: Option<String> = sqlx::query_scalar(
                    "SELECT status FROM deliveries WHERE project_id = ? AND id = ?",
                )
                .bind(&input.project_id)
                .bind(&input.delivery_id)
                .fetch_optional(&mut *conn)
                .await?;
                match status.as_deref() {
                    None => return Err(CoreError::NotFound { entity: "Entrega" }),
                    Some("pending") => {}
                    Some(_) => {
                        return Err(CoreError::Validation {
                            field: "deliveryId",
                            reason: "Solo se pueden devolver entregas pendientes de revisión."
                                .into(),
                        });
                    }
                }
                sqlx::query(
                    "UPDATE deliveries SET status = 'returned', return_reason = ?, \
                     decided_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE project_id = ? AND id = ? AND status = 'pending'",
                )
                .bind(&reason)
                .bind(&input.project_id)
                .bind(&input.delivery_id)
                .execute(&mut *conn)
                .await?;
                Ok(())
            })
        })
        .await
}

/// Marca la historia como bloqueada con un motivo (o quita el bloqueo). Una persona lo hace
/// como `manual`; un agente, como `reported`. Idempotente; no cambia la revisión.
pub async fn set_story_blocker(
    store: &Store,
    actor: &Actor,
    input: &SetBlockerInput,
) -> Result<(), CoreError> {
    let reason = input.reason.as_deref().map(validate_reason).transpose()?;
    let actor = actor.clone();
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let story = load_story(conn, &input.project_id, &input.story_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Historia" })?;
                // Un agente solo puede cambiar o quitar sus propios bloqueos reportados: no
                // puede ocultar un bloqueo marcado por una persona ni el de otro agente.
                if let Actor::Agent { agent_id } = &actor {
                    let existing: Option<(String, Option<String>)> = sqlx::query_as(
                        "SELECT source, agent_id FROM story_blockers WHERE story_id = ?",
                    )
                    .bind(&story.id)
                    .fetch_optional(&mut *conn)
                    .await?;
                    if let Some((source, owner)) = existing
                        && (source != "reported" || owner.as_deref() != Some(agent_id.as_str()))
                    {
                        return Err(CoreError::Forbidden {
                            reason: "Un agente no puede cambiar ni quitar un bloqueo marcado por \
                                     una persona u otro agente."
                                .into(),
                        });
                    }
                }
                match reason {
                    Some(reason) => {
                        if story.archived_at.is_some() || story.accepted_at.is_some() {
                            return Err(CoreError::Validation {
                                field: "storyId",
                                reason: format!(
                                    "La historia #{} está archivada o aceptada: no se bloquea.",
                                    story.number
                                ),
                            });
                        }
                        let (source, agent_id) = source_of(conn, &actor).await?;
                        sqlx::query(
                            "INSERT INTO story_blockers (story_id, project_id, reason, source, \
                             agent_id) VALUES (?, ?, ?, ?, ?) \
                             ON CONFLICT (story_id) DO UPDATE SET reason = excluded.reason, \
                             source = excluded.source, agent_id = excluded.agent_id, \
                             created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                             WHERE story_blockers.reason <> excluded.reason \
                                OR story_blockers.source <> excluded.source",
                        )
                        .bind(&story.id)
                        .bind(&input.project_id)
                        .bind(&reason)
                        .bind(source.as_str())
                        .bind(&agent_id)
                        .execute(&mut *conn)
                        .await?;
                    }
                    None => {
                        sqlx::query("DELETE FROM story_blockers WHERE story_id = ?")
                            .bind(&story.id)
                            .execute(&mut *conn)
                            .await?;
                    }
                }
                Ok(())
            })
        })
        .await
}
