use serde::Deserialize;
use ts_rs::TS;

use super::project::{ENTITY, Project, load_project, validate_name};
use crate::error::CoreError;
use crate::platform::revision;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenameProjectInput {
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub name: String,
}

pub async fn rename_project(
    store: &Store,
    input: &RenameProjectInput,
) -> Result<Project, CoreError> {
    let name = validate_name(&input.name)?;
    let id = input.id.clone();
    let expected = input.expected_revision;

    store
        .write(move |conn| {
            Box::pin(async move {
                let rows = sqlx::query(
                    "UPDATE projects SET name = ?, revision = revision + 1, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE id = ? AND revision = ?",
                )
                .bind(&name)
                .bind(&id)
                .bind(expected)
                .execute(&mut *conn)
                .await?
                .rows_affected();
                revision::ensure_updated(ENTITY, rows, load_project(conn, &id)).await?;
                load_project(conn, &id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}
