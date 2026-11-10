use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::accept::SatisfiedCriterion;
use super::delivery::{Delivery, DeliverySource, deliveries_of_story, resolve_reference_status};
use crate::error::CoreError;
use crate::features::stories::load_story;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AcceptanceEventKind {
    Accepted,
    Reopened,
}

/// Evento del historial de aceptación (nunca se borra).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcceptanceEvent {
    pub kind: AcceptanceEventKind,
    pub criteria: Vec<SatisfiedCriterion>,
    pub reason: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Blocker {
    pub reason: String,
    pub source: DeliverySource,
    pub agent_name: Option<String>,
    pub created_at: String,
}

/// Todo lo relativo a la revisión de una historia, para el editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoryReview {
    pub story_id: String,
    pub accepted_at: Option<String>,
    pub blocker: Option<Blocker>,
    /// Más reciente primero.
    pub deliveries: Vec<Delivery>,
    /// Más reciente primero.
    pub history: Vec<AcceptanceEvent>,
}

pub async fn get_story_review(
    store: &Store,
    project_id: &str,
    story_id: &str,
) -> Result<StoryReview, CoreError> {
    let mut conn = store.reader().acquire().await?;
    let story = load_story(&mut conn, project_id, story_id)
        .await?
        .ok_or(CoreError::NotFound { entity: "Historia" })?;

    let blocker: Option<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT b.reason, b.source, a.name, b.created_at FROM story_blockers b \
         LEFT JOIN agents a ON a.id = b.agent_id WHERE b.project_id = ? AND b.story_id = ?",
    )
    .bind(project_id)
    .bind(story_id)
    .fetch_optional(&mut *conn)
    .await?;

    let events: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT kind, criteria, reason, created_at FROM acceptance_events \
         WHERE project_id = ? AND story_id = ? ORDER BY created_at DESC, rowid DESC",
    )
    .bind(project_id)
    .bind(story_id)
    .fetch_all(&mut *conn)
    .await?;
    let history = events
        .into_iter()
        .map(|(kind, criteria, reason, created_at)| {
            Ok(AcceptanceEvent {
                kind: if kind == "accepted" {
                    AcceptanceEventKind::Accepted
                } else {
                    AcceptanceEventKind::Reopened
                },
                criteria: serde_json::from_str(&criteria)?,
                reason,
                created_at,
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;

    let mut deliveries = deliveries_of_story(&mut conn, project_id, story_id).await?;
    drop(conn);
    // Las rutas se comprueban fuera de la conexión.
    resolve_reference_status(&mut deliveries).await;

    Ok(StoryReview {
        story_id: story.id,
        accepted_at: story.accepted_at,
        blocker: blocker.map(|(reason, source, agent_name, created_at)| Blocker {
            reason,
            source: if source == "reported" {
                DeliverySource::Reported
            } else {
                DeliverySource::Manual
            },
            agent_name,
            created_at,
        }),
        deliveries,
        history,
    })
}
