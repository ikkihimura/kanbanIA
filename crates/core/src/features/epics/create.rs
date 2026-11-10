use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::epic::{ENTITY, Epic, load_epic, validate_description, validate_title};
use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::platform::idempotency::MutationKey;
use crate::platform::storage::Store;

const OPERATION: &str = "epics.create";

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateEpicInput {
    pub request_id: String,
    pub project_id: String,
    pub title: String,
    #[ts(optional)]
    pub description: Option<String>,
}

#[derive(Serialize)]
struct Payload<'a> {
    project_id: &'a str,
    title: &'a str,
    description: &'a str,
}

pub async fn create_epic(
    store: &Store,
    actor: &str,
    input: &CreateEpicInput,
) -> Result<Epic, CoreError> {
    let key = MutationKey::new(actor, OPERATION, &input.request_id)?;
    let title = validate_title(&input.title)?;
    let description = validate_description(input.description.as_deref().unwrap_or(""))?;
    let payload = Payload {
        project_id: &input.project_id,
        title: &title,
        description: &description,
    };
    let project_id = input.project_id.clone();
    let (title, description) = (title.clone(), description.clone());
    let id = uuid::Uuid::now_v7().to_string();

    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                ensure_project_exists(conn, &project_id).await?;
                let number: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(number), 0) + 1 FROM epics WHERE project_id = ?",
                )
                .bind(&project_id)
                .fetch_one(&mut *conn)
                .await?;
                sqlx::query(
                    "INSERT INTO epics (id, project_id, number, title, description) \
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&project_id)
                .bind(number)
                .bind(&title)
                .bind(&description)
                .execute(&mut *conn)
                .await?;
                load_epic(conn, &project_id, &id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}
