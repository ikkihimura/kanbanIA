use std::collections::HashSet;

use serde::Deserialize;
use sqlx::SqliteConnection;
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

                let criteria: Vec<(Option<String>, String)> =
                    criteria.into_iter().map(|c| (c.id, c.text)).collect();
                let rows = replace_document(
                    conn,
                    &input.project_id,
                    &input.id,
                    &Document {
                        title: &title,
                        description: &description,
                        priority: input.priority,
                        criteria: &criteria,
                    },
                    Some(input.expected_revision),
                )
                .await?;
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

/// Documento completo de una historia ya validado.
pub(crate) struct Document<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub priority: Priority,
    /// (id, texto) en orden. Un id que la historia no tiene se crea con ese id (importación).
    pub criteria: &'a [(Option<String>, String)],
}

/// Sustituye título, descripción, prioridad y criterios y sube la revisión. Compartida por el
/// editor y la importación (HU-010) para que ambas apliquen las mismas reglas. Quien llama
/// valida antes el contenido, la revisión y el archivo en la misma transacción. Con
/// `expected_revision`, el UPDATE final la exige (devuelve las filas afectadas).
///
/// Criterios: los que ya no aparecen se borran; los que tienen id conocido conservan su
/// número CA-n; los nuevos reciben el siguiente número, que nunca se reutiliza.
pub(crate) async fn replace_document(
    conn: &mut SqliteConnection,
    project_id: &str,
    id: &str,
    document: &Document<'_>,
    expected_revision: Option<i64>,
) -> Result<u64, CoreError> {
    let local: Vec<String> =
        sqlx::query_scalar("SELECT id FROM acceptance_criteria WHERE story_id = ?")
            .bind(id)
            .fetch_all(&mut *conn)
            .await?;
    let kept: HashSet<&str> = document
        .criteria
        .iter()
        .filter_map(|(cid, _)| cid.as_deref())
        .collect();
    for removed in local.iter().filter(|cid| !kept.contains(cid.as_str())) {
        sqlx::query("DELETE FROM acceptance_criteria WHERE id = ? AND story_id = ?")
            .bind(removed)
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    let local: HashSet<&str> = local.iter().map(String::as_str).collect();
    let mut next_number: i64 =
        sqlx::query_scalar("SELECT next_criterion_number FROM stories WHERE id = ?")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    for (position, (criterion_id, text)) in (0_i64..).zip(document.criteria) {
        match criterion_id {
            Some(cid) if local.contains(cid.as_str()) => {
                sqlx::query(
                    "UPDATE acceptance_criteria SET text = ?, position = ?, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE id = ? AND story_id = ? AND (text <> ? OR position <> ?)",
                )
                .bind(text)
                .bind(position)
                .bind(cid)
                .bind(id)
                .bind(text)
                .bind(position)
                .execute(&mut *conn)
                .await?;
            }
            other => {
                sqlx::query(
                    "INSERT INTO acceptance_criteria (id, story_id, number, position, text) \
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(
                    other
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
                )
                .bind(id)
                .bind(next_number)
                .bind(position)
                .bind(text)
                .execute(&mut *conn)
                .await?;
                next_number += 1;
            }
        }
    }
    Ok(sqlx::query(
        "UPDATE stories SET title = ?, description = ?, priority = ?, \
         next_criterion_number = ?, revision = revision + 1, \
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE project_id = ? AND id = ? AND (? IS NULL OR revision = ?)",
    )
    .bind(document.title)
    .bind(document.description)
    .bind(document.priority.as_str())
    .bind(next_number)
    .bind(project_id)
    .bind(id)
    .bind(expected_revision)
    .bind(expected_revision)
    .execute(&mut *conn)
    .await?
    .rows_affected())
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
