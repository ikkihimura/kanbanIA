use serde::Deserialize;
use ts_rs::TS;

use super::story::{ENTITY, StorySummary, count_in_column, load_story, not_archived};
use crate::error::CoreError;
use crate::features::columns::{ColumnCategory, columns_of_project, ensure_capacity, load_column};
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArchiveStoryInput {
    pub project_id: String,
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreStoryInput {
    pub project_id: String,
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    /// Columna de destino; por defecto, la que tenía al archivarse.
    #[ts(optional)]
    pub column_id: Option<String>,
}

/// Archiva: la historia sale del orden del tablero y deja de contar para WIP, pero conserva
/// todo su contenido, criterios y (en historias futuras) resultados. Nada se borra.
pub async fn archive_story(
    store: &Store,
    input: &ArchiveStoryInput,
) -> Result<StorySummary, CoreError> {
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let story =
                    current(conn, &input.project_id, &input.id, input.expected_revision).await?;
                not_archived(&story)?;
                let position = story.position.unwrap_or_default();

                sqlx::query(
                    "UPDATE stories SET archived_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), \
                     position = NULL, revision = revision + 1, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE project_id = ? AND id = ?",
                )
                .bind(&input.project_id)
                .bind(&input.id)
                .execute(&mut *conn)
                .await?;
                sqlx::query(
                    "UPDATE stories SET position = position - 1 \
                     WHERE column_id = ? AND position > ?",
                )
                .bind(&story.column_id)
                .bind(position)
                .execute(&mut *conn)
                .await?;

                load_story(conn, &input.project_id, &input.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}

/// Restaura al final de la columna indicada (o la original). Es una entrada: aplica WIP y
/// no admite columnas «hecho».
pub async fn restore_story(
    store: &Store,
    input: &RestoreStoryInput,
) -> Result<StorySummary, CoreError> {
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let story =
                    current(conn, &input.project_id, &input.id, input.expected_revision).await?;
                if story.archived_at.is_none() {
                    return Err(CoreError::Validation {
                        field: "id",
                        reason: format!("La historia #{} no está archivada.", story.number),
                    });
                }
                let accepted = story.accepted_at.is_some();
                let column = match input.column_id.as_deref() {
                    Some(id) => load_column(conn, &input.project_id, id)
                        .await?
                        .ok_or(CoreError::NotFound { entity: "Columna" })?,
                    None => {
                        let original = load_column(conn, &input.project_id, &story.column_id)
                            .await?
                            .ok_or(CoreError::NotFound { entity: "Columna" })?;
                        // Una aceptada vuelve a «hecho»: si su columna ya no lo es, a la primera.
                        if accepted && original.category != ColumnCategory::Done {
                            columns_of_project(conn, &input.project_id)
                                .await?
                                .into_iter()
                                .find(|c| c.category == ColumnCategory::Done)
                                .unwrap_or(original)
                        } else {
                            original
                        }
                    }
                };
                let into_done = column.category == ColumnCategory::Done;
                if accepted && !into_done {
                    return Err(CoreError::Validation {
                        field: "columnId",
                        reason: format!(
                            "La historia #{} está aceptada: se restaura en una columna «hecho» \
                             (o reábrela antes).",
                            story.number
                        ),
                    });
                }
                if !accepted && into_done {
                    return Err(CoreError::AcceptanceRequired {
                        column: column.name,
                    });
                }
                ensure_capacity(conn, &column, 1).await?;
                let position = count_in_column(conn, &column.id).await?;

                sqlx::query(
                    "UPDATE stories SET archived_at = NULL, column_id = ?, position = ?, \
                     revision = revision + 1, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE project_id = ? AND id = ?",
                )
                .bind(&column.id)
                .bind(position)
                .bind(&input.project_id)
                .bind(&input.id)
                .execute(&mut *conn)
                .await?;

                load_story(conn, &input.project_id, &input.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}

async fn current(
    conn: &mut sqlx::SqliteConnection,
    project_id: &str,
    id: &str,
    expected_revision: i64,
) -> Result<StorySummary, CoreError> {
    let story = load_story(conn, project_id, id)
        .await?
        .ok_or(CoreError::NotFound { entity: ENTITY })?;
    if story.revision != expected_revision {
        return Err(CoreError::Conflict {
            current: serde_json::to_value(&story)?,
        });
    }
    Ok(story)
}
