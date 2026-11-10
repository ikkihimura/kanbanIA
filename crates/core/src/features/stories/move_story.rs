use serde::Deserialize;
use ts_rs::TS;

use super::story::{ENTITY, StorySummary, count_in_column, load_story, not_archived};
use crate::error::CoreError;
use crate::features::columns::{ColumnCategory, ensure_capacity, load_column};
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MoveStoryInput {
    pub project_id: String,
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub to_column_id: String,
    /// Posición en la columna destino; se acota al final si es mayor.
    #[ts(type = "number")]
    pub to_index: i64,
}

/// Mueve una tarjeta (entre columnas o dentro de la misma) de forma atómica.
/// Entrar en otra columna aplica el límite WIP; entrar en «hecho» exige aceptación (HU-009).
pub async fn move_story(store: &Store, input: &MoveStoryInput) -> Result<StorySummary, CoreError> {
    if input.to_index < 0 {
        return Err(CoreError::Validation {
            field: "toIndex",
            reason: "La posición no puede ser negativa.".into(),
        });
    }
    let input = input.clone();

    store
        .write(move |conn| {
            Box::pin(async move {
                let story = load_story(conn, &input.project_id, &input.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })?;
                if story.revision != input.expected_revision {
                    return Err(CoreError::Conflict {
                        current: serde_json::to_value(&story)?,
                    });
                }
                not_archived(&story)?;
                let old_position = story.position.unwrap_or_default();
                let destination = load_column(conn, &input.project_id, &input.to_column_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Columna" })?;
                let changes_column = destination.id != story.column_id;
                if changes_column {
                    if destination.category == ColumnCategory::Done {
                        return Err(CoreError::AcceptanceRequired {
                            column: destination.name,
                        });
                    }
                    ensure_capacity(conn, &destination, 1).await?;
                }

                // Sacar la tarjeta de su posición: compactar el origen.
                sqlx::query(
                    "UPDATE stories SET position = position - 1 \
                     WHERE column_id = ? AND position > ?",
                )
                .bind(&story.column_id)
                .bind(old_position)
                .execute(&mut *conn)
                .await?;

                // Hueco en el destino (sin contar la propia tarjeta).
                let others =
                    count_in_column(conn, &destination.id).await? - i64::from(!changes_column);
                let index = input.to_index.min(others);
                sqlx::query(
                    "UPDATE stories SET position = position + 1 \
                     WHERE column_id = ? AND position >= ? AND id <> ?",
                )
                .bind(&destination.id)
                .bind(index)
                .bind(&story.id)
                .execute(&mut *conn)
                .await?;

                sqlx::query(
                    "UPDATE stories SET column_id = ?, position = ?, revision = revision + 1, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE project_id = ? AND id = ?",
                )
                .bind(&destination.id)
                .bind(index)
                .bind(&input.project_id)
                .bind(&story.id)
                .execute(&mut *conn)
                .await?;

                load_story(conn, &input.project_id, &story.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}
