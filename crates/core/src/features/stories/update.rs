use std::collections::HashSet;

use serde::Deserialize;
use ts_rs::TS;

use super::story::{
    ENTITY, MAX_CRITERIA, Priority, StoryDetail, load_detail, validate_criterion,
    validate_description, validate_title,
};
use crate::error::CoreError;
use crate::platform::revision;
use crate::platform::storage::Store;

/// Criterio en el editor: con `id` conserva su número CA-n; sin `id` es nuevo.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CriterionDraft {
    #[ts(optional)]
    pub id: Option<String>,
    pub text: String,
}

/// Edición completa de la historia como un solo documento con una sola revisión:
/// título, descripción, prioridad y lista de criterios (en orden).
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateStoryInput {
    pub project_id: String,
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub title: String,
    pub description: String,
    pub priority: Priority,
    pub criteria: Vec<CriterionDraft>,
}

struct ValidCriterion {
    id: Option<String>,
    text: String,
}

pub async fn update_story(
    store: &Store,
    input: &UpdateStoryInput,
) -> Result<StoryDetail, CoreError> {
    let title = validate_title(&input.title)?;
    let description = validate_description(&input.description)?;
    let criteria = validate_criteria(&input.criteria)?;
    let input = input.clone();

    store
        .write(move |conn| {
            Box::pin(async move {
                let current = load_detail(conn, &input.project_id, &input.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })?;
                if current.revision != input.expected_revision {
                    // El cliente recibe la versión vigente completa y conserva su borrador.
                    return Err(CoreError::Conflict {
                        current: serde_json::to_value(&current)?,
                    });
                }
                if current.archived_at.is_some() {
                    return Err(CoreError::Validation {
                        field: "id",
                        reason: format!(
                            "La historia #{} está archivada; restáurala antes.",
                            current.number
                        ),
                    });
                }

                let known: HashSet<&str> = current.criteria.iter().map(|c| c.id.as_str()).collect();
                if let Some(unknown) = criteria
                    .iter()
                    .filter_map(|c| c.id.as_deref())
                    .find(|id| !known.contains(id))
                {
                    return Err(CoreError::Validation {
                        field: "criteria",
                        reason: format!("El criterio {unknown} no pertenece a esta historia."),
                    });
                }

                // Criterios eliminados: los que ya no aparecen. Su número no se reutiliza.
                let kept: HashSet<&str> = criteria.iter().filter_map(|c| c.id.as_deref()).collect();
                for removed in current
                    .criteria
                    .iter()
                    .filter(|c| !kept.contains(c.id.as_str()))
                {
                    sqlx::query("DELETE FROM acceptance_criteria WHERE id = ? AND story_id = ?")
                        .bind(&removed.id)
                        .bind(&input.id)
                        .execute(&mut *conn)
                        .await?;
                }

                let mut next_number: i64 =
                    sqlx::query_scalar("SELECT next_criterion_number FROM stories WHERE id = ?")
                        .bind(&input.id)
                        .fetch_one(&mut *conn)
                        .await?;
                for (position, criterion) in (0_i64..).zip(&criteria) {
                    match &criterion.id {
                        Some(id) => {
                            sqlx::query(
                                "UPDATE acceptance_criteria SET text = ?, position = ?, \
                                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                                 WHERE id = ? AND story_id = ? \
                                   AND (text <> ? OR position <> ?)",
                            )
                            .bind(&criterion.text)
                            .bind(position)
                            .bind(id)
                            .bind(&input.id)
                            .bind(&criterion.text)
                            .bind(position)
                            .execute(&mut *conn)
                            .await?;
                        }
                        None => {
                            sqlx::query(
                                "INSERT INTO acceptance_criteria \
                                 (id, story_id, number, position, text) VALUES (?, ?, ?, ?, ?)",
                            )
                            .bind(uuid::Uuid::now_v7().to_string())
                            .bind(&input.id)
                            .bind(next_number)
                            .bind(position)
                            .bind(&criterion.text)
                            .execute(&mut *conn)
                            .await?;
                            next_number += 1;
                        }
                    }
                }

                let rows = sqlx::query(
                    "UPDATE stories SET title = ?, description = ?, priority = ?, \
                     next_criterion_number = ?, revision = revision + 1, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE project_id = ? AND id = ? AND revision = ?",
                )
                .bind(&title)
                .bind(&description)
                .bind(input.priority.as_str())
                .bind(next_number)
                .bind(&input.project_id)
                .bind(&input.id)
                .bind(input.expected_revision)
                .execute(&mut *conn)
                .await?
                .rows_affected();
                // Defensa en profundidad: la revisión ya se comprobó en esta misma transacción.
                revision::ensure_updated(
                    ENTITY,
                    rows,
                    load_detail(conn, &input.project_id, &input.id),
                )
                .await?;

                load_detail(conn, &input.project_id, &input.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}

fn validate_criteria(drafts: &[CriterionDraft]) -> Result<Vec<ValidCriterion>, CoreError> {
    if drafts.len() > MAX_CRITERIA {
        return Err(CoreError::Validation {
            field: "criteria",
            reason: "Una historia admite como máximo 50 criterios.".into(),
        });
    }
    let mut seen = HashSet::new();
    drafts
        .iter()
        .map(|draft| {
            if let Some(id) = &draft.id
                && !seen.insert(id.as_str())
            {
                return Err(CoreError::Validation {
                    field: "criteria",
                    reason: "Un criterio aparece repetido.".into(),
                });
            }
            Ok(ValidCriterion {
                id: draft.id.clone(),
                text: validate_criterion(&draft.text)?,
            })
        })
        .collect()
}
