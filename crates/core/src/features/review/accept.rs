use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::first_column_of;
use crate::error::CoreError;
use crate::features::claims::{EndReason, close_story_claims};
use crate::features::columns::{ColumnCategory, columns_of_project, ensure_capacity, load_column};
use crate::features::runs::{StoryClosed, cancel_story_runs};
use crate::features::stories::{
    StorySummary, append_to_column, load_detail, load_story, place_at_index,
};
use crate::platform::actor::Actor;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcceptStoryInput {
    pub project_id: String,
    pub story_id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    /// Ids de los criterios que la persona marcó como satisfechos: deben ser todos.
    pub satisfied_criteria: Vec<String>,
    /// Columna «hecho» destino (p. ej. donde se soltó en el tablero); por defecto la primera.
    #[ts(optional)]
    pub column_id: Option<String>,
    /// Posición en esa columna; por defecto, al final. Se acota al final si es mayor.
    #[ts(optional, type = "number")]
    pub to_index: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReopenStoryInput {
    pub project_id: String,
    pub story_id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub reason: String,
    /// Columna destino (no «hecho»); por defecto la primera «revisión» o la primera utilizable.
    #[ts(optional)]
    pub column_id: Option<String>,
}

/// Criterio tal como estaba al aceptar (queda en el historial aunque luego cambie).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SatisfiedCriterion {
    #[ts(type = "number")]
    pub number: i64,
    pub text: String,
}

/// Acepta la historia. Solo una persona; exige al menos un criterio y que todos estén
/// marcados como satisfechos. La mueve a la columna «hecho» indicada (por defecto, al final de
/// la primera), con su límite WIP, en la misma transacción. Marca como
/// aceptadas sus entregas pendientes y registra el evento con los criterios en el historial.
pub async fn accept_story(
    store: &Store,
    actor: &Actor,
    input: &AcceptStoryInput,
) -> Result<StorySummary, CoreError> {
    actor.require_human("aceptar una historia")?;
    if input.to_index.is_some_and(|i| i < 0) {
        return Err(invalid(
            "toIndex",
            "La posición no puede ser negativa.".into(),
        ));
    }
    let (input, clock) = (input.clone(), store.clone());

    store
        .write(move |conn| {
            Box::pin(async move {
                let story = load_detail(conn, &input.project_id, &input.story_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Historia" })?;
                if story.revision != input.expected_revision {
                    return Err(CoreError::Conflict {
                        current: serde_json::to_value(&story)?,
                    });
                }
                if story.archived_at.is_some() {
                    return Err(invalid(
                        "storyId",
                        format!("La historia #{} está archivada.", story.number),
                    ));
                }
                if story.accepted_at.is_some() {
                    return Err(invalid(
                        "storyId",
                        format!("La historia #{} ya está aceptada.", story.number),
                    ));
                }
                if story.criteria.is_empty() {
                    return Err(invalid(
                        "satisfiedCriteria",
                        "Añade al menos un criterio de aceptación verificable antes de aceptar."
                            .into(),
                    ));
                }
                let marked: HashSet<&str> = input
                    .satisfied_criteria
                    .iter()
                    .map(String::as_str)
                    .collect();
                let missing: Vec<String> = story
                    .criteria
                    .iter()
                    .filter(|c| !marked.contains(c.id.as_str()))
                    .map(|c| format!("CA-{}", c.number))
                    .collect();
                if !missing.is_empty() {
                    return Err(invalid(
                        "satisfiedCriteria",
                        format!(
                            "Faltan criterios por marcar como satisfechos: {}.",
                            missing.join(", ")
                        ),
                    ));
                }
                let known: HashSet<&str> = story.criteria.iter().map(|c| c.id.as_str()).collect();
                if marked.iter().any(|id| !known.contains(id)) {
                    return Err(invalid(
                        "satisfiedCriteria",
                        "Hay criterios marcados que no pertenecen a esta historia.".into(),
                    ));
                }

                let done = match &input.column_id {
                    Some(id) => {
                        let column = load_column(conn, &input.project_id, id)
                            .await?
                            .ok_or(CoreError::NotFound { entity: "Columna" })?;
                        if column.category != ColumnCategory::Done {
                            return Err(invalid(
                                "columnId",
                                "Al aceptar, la historia entra en una columna «hecho».".into(),
                            ));
                        }
                        column
                    }
                    None => first_column_of(conn, &input.project_id, ColumnCategory::Done)
                        .await?
                        .ok_or_else(|| {
                            invalid(
                                "storyId",
                                "El proyecto no tiene ninguna columna de tipo «hecho»: crea una \
                                 para poder aceptar."
                                    .into(),
                            )
                        })?,
                };
                ensure_capacity(conn, &done, 1).await?;

                let summary = load_story(conn, &input.project_id, &input.story_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Historia" })?;
                append_to_column(conn, &summary, &done.id).await?;
                if let Some(index) = input.to_index {
                    place_at_index(conn, &story.id, &done.id, index).await?;
                }
                sqlx::query(
                    "UPDATE stories SET accepted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), \
                     revision = revision + 1 WHERE id = ?",
                )
                .bind(&story.id)
                .execute(&mut *conn)
                .await?;
                // Aceptar resuelve el bloqueo: no debe reaparecer al reabrir.
                sqlx::query("DELETE FROM story_blockers WHERE story_id = ?")
                    .bind(&story.id)
                    .execute(&mut *conn)
                    .await?;
                sqlx::query(
                    "UPDATE deliveries SET status = 'accepted', \
                     decided_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE story_id = ? AND status = 'pending'",
                )
                .bind(&story.id)
                .execute(&mut *conn)
                .await?;
                // Aceptada: ya no hay trabajo que reservar (HU-015).
                let now = clock.now_ms();
                close_story_claims(conn, &story.id, EndReason::Closed, now).await?;
                // Y ningún intento sigue activo (HU-017).
                cancel_story_runs(conn, &story.id, StoryClosed::Accepted, now).await?;

                let snapshot: Vec<SatisfiedCriterion> = story
                    .criteria
                    .iter()
                    .map(|c| SatisfiedCriterion {
                        number: c.number,
                        text: c.text.clone(),
                    })
                    .collect();
                record_event(
                    conn,
                    &input.project_id,
                    &story.id,
                    "accepted",
                    &snapshot,
                    None,
                )
                .await?;

                load_story(conn, &input.project_id, &input.story_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Historia" })
            })
        })
        .await
}

/// Reabre una historia aceptada. Solo una persona y con motivo. Sale de «hecho» hacia una
/// columna utilizable (aplica WIP); el historial conserva la aceptación anterior.
pub async fn reopen_story(
    store: &Store,
    actor: &Actor,
    input: &ReopenStoryInput,
) -> Result<StorySummary, CoreError> {
    actor.require_human("reabrir una historia")?;
    let reason = input.reason.trim().to_owned();
    if reason.is_empty()
        || reason.chars().count() > 500
        || reason.chars().any(char::is_control)
        || super::has_invisible_formatting(&reason)
    {
        return Err(invalid(
            "reason",
            "Indica el motivo de la reapertura (hasta 500 caracteres, una línea).".into(),
        ));
    }
    let input = input.clone();

    store
        .write(move |conn| {
            Box::pin(async move {
                let story = load_story(conn, &input.project_id, &input.story_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Historia" })?;
                if story.revision != input.expected_revision {
                    return Err(CoreError::Conflict {
                        current: serde_json::to_value(&story)?,
                    });
                }
                if story.accepted_at.is_none() {
                    return Err(invalid(
                        "storyId",
                        format!("La historia #{} no está aceptada.", story.number),
                    ));
                }

                let destination = match &input.column_id {
                    Some(id) => load_column(conn, &input.project_id, id)
                        .await?
                        .ok_or(CoreError::NotFound { entity: "Columna" })?,
                    None => match first_column_of(conn, &input.project_id, ColumnCategory::Review)
                        .await?
                    {
                        Some(review) => review,
                        None => columns_of_project(conn, &input.project_id)
                            .await?
                            .into_iter()
                            .find(|c| c.category != ColumnCategory::Done)
                            .ok_or(CoreError::NotFound { entity: "Columna" })?,
                    },
                };
                if destination.category == ColumnCategory::Done {
                    return Err(invalid(
                        "columnId",
                        "Al reabrir, la historia debe salir de las columnas «hecho».".into(),
                    ));
                }

                if story.archived_at.is_some() {
                    // Archivada: solo cambia la columna de referencia para restaurarla después.
                    sqlx::query("UPDATE stories SET column_id = ? WHERE id = ?")
                        .bind(&destination.id)
                        .bind(&story.id)
                        .execute(&mut *conn)
                        .await?;
                } else {
                    ensure_capacity(conn, &destination, 1).await?;
                    append_to_column(conn, &story, &destination.id).await?;
                }
                sqlx::query(
                    "UPDATE stories SET accepted_at = NULL, revision = revision + 1 WHERE id = ?",
                )
                .bind(&story.id)
                .execute(&mut *conn)
                .await?;
                record_event(
                    conn,
                    &input.project_id,
                    &story.id,
                    "reopened",
                    &[],
                    Some(&reason),
                )
                .await?;

                load_story(conn, &input.project_id, &input.story_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Historia" })
            })
        })
        .await
}

async fn record_event(
    conn: &mut sqlx::SqliteConnection,
    project_id: &str,
    story_id: &str,
    kind: &str,
    criteria: &[SatisfiedCriterion],
    reason: Option<&str>,
) -> Result<(), CoreError> {
    sqlx::query(
        "INSERT INTO acceptance_events (id, project_id, story_id, kind, actor, criteria, reason) \
         VALUES (?, ?, ?, ?, 'human', ?, ?)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(project_id)
    .bind(story_id)
    .bind(kind)
    .bind(serde_json::to_string(criteria)?)
    .bind(reason)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

fn invalid(field: &'static str, reason: String) -> CoreError {
    CoreError::Validation { field, reason }
}
