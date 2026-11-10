use serde::Deserialize;
use ts_rs::TS;

use super::column::{ColumnCategory, ENTITY, ensure_usable_column_remains, load_column};
use super::wip::ensure_capacity;
use crate::error::CoreError;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeleteColumnInput {
    pub project_id: String,
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    /// Obligatorio si la columna tiene tarjetas: se mueven al final de esta columna.
    #[ts(optional)]
    pub destination_column_id: Option<String>,
}

/// Elimina una columna. Si tiene tarjetas, las mueve al final del destino conservando su
/// orden, todo en una transacción: si algo falla (WIP, destino inválido…) no cambia nada.
pub async fn delete_column(store: &Store, input: &DeleteColumnInput) -> Result<(), CoreError> {
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let column = load_column(conn, &input.project_id, &input.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })?;
                if column.revision != input.expected_revision {
                    return Err(CoreError::Conflict {
                        current: serde_json::to_value(&column)?,
                    });
                }
                ensure_usable_column_remains(conn, &input.project_id, &input.id).await?;

                if column.card_count > 0 {
                    let destination_id = input.destination_column_id.as_deref().ok_or_else(
                        || CoreError::Validation {
                            field: "destinationColumnId",
                            reason: format!(
                                "La columna «{}» tiene {} tarjetas: elige a qué columna moverlas.",
                                column.name, column.card_count
                            ),
                        },
                    )?;
                    if destination_id == input.id {
                        return Err(CoreError::Validation {
                            field: "destinationColumnId",
                            reason: "El destino debe ser otra columna.".into(),
                        });
                    }
                    let destination = load_column(conn, &input.project_id, destination_id)
                        .await?
                        .ok_or(CoreError::NotFound { entity: ENTITY })?;
                    if destination.category == ColumnCategory::Done {
                        return Err(CoreError::AcceptanceRequired {
                            column: destination.name,
                        });
                    }
                    ensure_capacity(conn, &destination, column.card_count).await?;

                    // Al final del destino, en el mismo orden (las posiciones del origen son 0..n).
                    sqlx::query(
                        "UPDATE stories SET column_id = ?, position = position + ?, \
                         revision = revision + 1, \
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                         WHERE project_id = ? AND column_id = ? AND archived_at IS NULL",
                    )
                    .bind(&destination.id)
                    .bind(destination.card_count)
                    .bind(&input.project_id)
                    .bind(&input.id)
                    .execute(&mut *conn)
                    .await?;
                }

                // Las archivadas conservan su columna de origen para restaurarse: se reasignan
                // al destino elegido o, sin él, a la primera columna utilizable restante.
                // El destino indicado se valida siempre (también sin tarjetas activas). Nunca se
                // envían archivadas a una columna «hecho»: se usa la primera utilizable.
                let requested = match input.destination_column_id.as_deref() {
                    Some(destination) if destination != input.id => Some(
                        load_column(conn, &input.project_id, destination)
                            .await?
                            .ok_or(CoreError::NotFound { entity: ENTITY })?,
                    ),
                    _ => None,
                };
                let archived_target: Option<String> = match requested {
                    Some(destination) if destination.category != ColumnCategory::Done => {
                        Some(destination.id)
                    }
                    _ => {
                        sqlx::query_scalar(
                            "SELECT id FROM columns WHERE project_id = ? AND id <> ? \
                             AND category <> 'done' ORDER BY position LIMIT 1",
                        )
                        .bind(&input.project_id)
                        .bind(&input.id)
                        .fetch_optional(&mut *conn)
                        .await?
                    }
                };
                if let Some(target) = archived_target {
                    sqlx::query(
                        "UPDATE stories SET column_id = ?, revision = revision + 1, \
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                         WHERE project_id = ? AND column_id = ? AND archived_at IS NOT NULL",
                    )
                    .bind(&target)
                    .bind(&input.project_id)
                    .bind(&input.id)
                    .execute(&mut *conn)
                    .await?;
                }

                sqlx::query("DELETE FROM columns WHERE project_id = ? AND id = ?")
                    .bind(&input.project_id)
                    .bind(&input.id)
                    .execute(&mut *conn)
                    .await?;
                sqlx::query(
                    "UPDATE columns SET position = position - 1 \
                     WHERE project_id = ? AND position > ?",
                )
                .bind(&input.project_id)
                .bind(column.position)
                .execute(&mut *conn)
                .await?;
                Ok(())
            })
        })
        .await
}
