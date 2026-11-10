use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::story::{ENTITY, StorySummary, count_in_column, load_story, validate_title};
use crate::error::CoreError;
use crate::features::columns::{
    Column, ColumnCategory, columns_of_project, ensure_capacity, load_column,
};
use crate::features::projects::ensure_project_exists;
use crate::platform::idempotency::MutationKey;
use crate::platform::storage::Store;

const OPERATION: &str = "stories.create";

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateStoryInput {
    pub request_id: String,
    pub project_id: String,
    pub title: String,
    /// Si se omite: primera columna «backlog» o, si no hay, la primera que no sea «hecho».
    #[ts(optional)]
    pub column_id: Option<String>,
}

#[derive(Serialize)]
struct Payload<'a> {
    project_id: &'a str,
    title: &'a str,
    column_id: Option<&'a str>,
}

pub async fn create_story(
    store: &Store,
    actor: &str,
    input: &CreateStoryInput,
) -> Result<StorySummary, CoreError> {
    let key = MutationKey::new(actor, OPERATION, &input.request_id)?;
    let title = validate_title(&input.title)?;
    let payload = Payload {
        project_id: &input.project_id,
        title: &title,
        column_id: input.column_id.as_deref(),
    };
    let project_id = input.project_id.clone();
    let column_id = input.column_id.clone();
    let title = title.clone();
    let id = uuid::Uuid::now_v7().to_string();

    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                ensure_project_exists(conn, &project_id).await?;
                let column = match column_id {
                    Some(column_id) => load_column(conn, &project_id, &column_id)
                        .await?
                        .ok_or(CoreError::NotFound { entity: "Columna" })?,
                    None => default_column(columns_of_project(conn, &project_id).await?)?,
                };
                if column.category == ColumnCategory::Done {
                    return Err(CoreError::AcceptanceRequired {
                        column: column.name,
                    });
                }
                ensure_capacity(conn, &column, 1).await?;

                let position = count_in_column(conn, &column.id).await?;
                sqlx::query(
                    "INSERT INTO stories (id, project_id, column_id, position, title) \
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&project_id)
                .bind(&column.id)
                .bind(position)
                .bind(&title)
                .execute(&mut *conn)
                .await?;
                load_story(conn, &project_id, &id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}

fn default_column(columns: Vec<Column>) -> Result<Column, CoreError> {
    let backlog = columns
        .iter()
        .position(|c| c.category == ColumnCategory::Backlog);
    let usable = columns
        .iter()
        .position(|c| c.category != ColumnCategory::Done);
    backlog
        .or(usable)
        .and_then(|index| columns.into_iter().nth(index))
        .ok_or(CoreError::NotFound { entity: "Columna" })
}
