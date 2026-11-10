use serde::Deserialize;
use ts_rs::TS;

use super::column::{
    Column, ColumnCategory, ENTITY, ensure_usable_column_remains, load_column, validate_name,
    validate_wip,
};
use crate::error::CoreError;
use crate::platform::revision;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateColumnInput {
    pub project_id: String,
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub name: String,
    pub category: ColumnCategory,
    #[ts(type = "number | null")]
    pub wip_limit: Option<i64>,
}

/// Edita nombre, categoría y límite WIP.
///
/// - Bajar el límite por debajo del número actual de tarjetas está permitido: solo impide
///   nuevas entradas (la UI marca la columna como excedida).
/// - Una columna con tarjetas no puede pasar a «hecho» ni dejar de serlo: equivaldría a
///   aceptar o reabrir trabajo sin la operación de aceptación.
pub async fn update_column(store: &Store, input: &UpdateColumnInput) -> Result<Column, CoreError> {
    let name = validate_name(&input.name)?;
    let wip_limit = validate_wip(input.wip_limit)?;
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

                let touches_done = (current.category == ColumnCategory::Done)
                    != (input.category == ColumnCategory::Done);
                // Cuentan también las archivadas: restaurarlas no debe aterrizar en «hecho».
                let any_stories: bool =
                    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM stories WHERE column_id = ?)")
                        .bind(&input.id)
                        .fetch_one(&mut *conn)
                        .await?;
                if touches_done && any_stories {
                    return Err(CoreError::Validation {
                        field: "category",
                        reason: "Una columna con historias (también archivadas) no puede pasar a \
                                 «hecho» ni dejar de serlo: muévelas antes."
                            .into(),
                    });
                }
                if input.category == ColumnCategory::Done {
                    ensure_usable_column_remains(conn, &input.project_id, &input.id).await?;
                }

                let rows = sqlx::query(
                    "UPDATE columns SET name = ?, category = ?, wip_limit = ?, \
                     revision = revision + 1, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE project_id = ? AND id = ? AND revision = ?",
                )
                .bind(&name)
                .bind(input.category.as_str())
                .bind(wip_limit)
                .bind(&input.project_id)
                .bind(&input.id)
                .bind(input.expected_revision)
                .execute(&mut *conn)
                .await?
                .rows_affected();
                revision::ensure_updated(
                    ENTITY,
                    rows,
                    load_column(conn, &input.project_id, &input.id),
                )
                .await?;

                load_column(conn, &input.project_id, &input.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}
