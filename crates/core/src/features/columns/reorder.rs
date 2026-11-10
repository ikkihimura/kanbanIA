use serde::Deserialize;
use ts_rs::TS;

use super::column::{Column, ENTITY, load_column};
use crate::error::CoreError;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReorderColumnInput {
    pub project_id: String,
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    /// Posición final (0 = primera).
    #[ts(type = "number")]
    pub to_index: i64,
}

/// Mueve una columna a `to_index`, desplazando las intermedias. Las posiciones quedan densas
/// (0..n). Solo la columna movida cambia de revisión: el orden es disposición, no contenido.
pub async fn reorder_column(
    store: &Store,
    input: &ReorderColumnInput,
) -> Result<Column, CoreError> {
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let current = load_column(conn, &input.project_id, &input.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })?;
                if current.revision != input.expected_revision {
                    return Err(CoreError::Conflict {
                        current: serde_json::to_value(&current)?,
                    });
                }

                let count: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM columns WHERE project_id = ?")
                        .bind(&input.project_id)
                        .fetch_one(&mut *conn)
                        .await?;
                if !(0..count).contains(&input.to_index) {
                    return Err(CoreError::Validation {
                        field: "toIndex",
                        reason: format!("La posición debe estar entre 0 y {}.", count - 1),
                    });
                }

                let from = current.position;
                let to = input.to_index;
                if from != to {
                    let (shift, low, high) = if to > from {
                        (-1, from + 1, to)
                    } else {
                        (1, to, from - 1)
                    };
                    sqlx::query(
                        "UPDATE columns SET position = position + ? \
                         WHERE project_id = ? AND position BETWEEN ? AND ? AND id <> ?",
                    )
                    .bind(shift)
                    .bind(&input.project_id)
                    .bind(low)
                    .bind(high)
                    .bind(&input.id)
                    .execute(&mut *conn)
                    .await?;
                    sqlx::query(
                        "UPDATE columns SET position = ?, revision = revision + 1, \
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                         WHERE project_id = ? AND id = ?",
                    )
                    .bind(to)
                    .bind(&input.project_id)
                    .bind(&input.id)
                    .execute(&mut *conn)
                    .await?;
                }

                load_column(conn, &input.project_id, &input.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}
