use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::column::{Column, ColumnCategory, ENTITY, load_column, validate_name, validate_wip};
use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::platform::idempotency::MutationKey;
use crate::platform::storage::Store;

const OPERATION: &str = "columns.create";

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateColumnInput {
    pub request_id: String,
    pub project_id: String,
    pub name: String,
    pub category: ColumnCategory,
    #[ts(type = "number | null")]
    pub wip_limit: Option<i64>,
}

#[derive(Serialize)]
struct Payload<'a> {
    project_id: &'a str,
    name: &'a str,
    category: ColumnCategory,
    wip_limit: Option<i64>,
}

/// Crea una columna al final del flujo.
pub async fn create_column(
    store: &Store,
    actor: &str,
    input: &CreateColumnInput,
) -> Result<Column, CoreError> {
    let key = MutationKey::new(actor, OPERATION, &input.request_id)?;
    let name = validate_name(&input.name)?;
    let wip_limit = validate_wip(input.wip_limit)?;
    let payload = Payload {
        project_id: &input.project_id,
        name: &name,
        category: input.category,
        wip_limit,
    };
    let project_id = input.project_id.clone();
    let category = input.category;
    let name = name.clone();
    let id = uuid::Uuid::now_v7().to_string();

    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                ensure_project_exists(conn, &project_id).await?;
                let position: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM columns WHERE project_id = ?")
                        .bind(&project_id)
                        .fetch_one(&mut *conn)
                        .await?;
                sqlx::query(
                    "INSERT INTO columns (id, project_id, name, category, position, wip_limit) \
                     VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&project_id)
                .bind(&name)
                .bind(category.as_str())
                .bind(position)
                .bind(wip_limit)
                .execute(&mut *conn)
                .await?;
                load_column(conn, &project_id, &id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}
