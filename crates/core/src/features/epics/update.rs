use serde::Deserialize;
use ts_rs::TS;

use super::epic::{ENTITY, Epic, load_epic, validate_description, validate_title};
use crate::error::CoreError;
use crate::platform::revision;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateEpicInput {
    pub project_id: String,
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub title: String,
    pub description: String,
}

pub async fn update_epic(store: &Store, input: &UpdateEpicInput) -> Result<Epic, CoreError> {
    let title = validate_title(&input.title)?;
    let description = validate_description(&input.description)?;
    let input = input.clone();

    store
        .write(move |conn| {
            Box::pin(async move {
                let rows = sqlx::query(
                    "UPDATE epics SET title = ?, description = ?, revision = revision + 1, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE project_id = ? AND id = ? AND revision = ?",
                )
                .bind(&title)
                .bind(&description)
                .bind(&input.project_id)
                .bind(&input.id)
                .bind(input.expected_revision)
                .execute(&mut *conn)
                .await?
                .rows_affected();
                revision::ensure_updated(
                    ENTITY,
                    rows,
                    load_epic(conn, &input.project_id, &input.id),
                )
                .await?;
                load_epic(conn, &input.project_id, &input.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}
