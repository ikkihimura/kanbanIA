use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::project::{Project, load_project, validate_name};
use crate::error::CoreError;
use crate::platform::idempotency::MutationKey;
use crate::platform::storage::Store;

const OPERATION: &str = "projects.create";

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateProjectInput {
    pub request_id: String,
    pub name: String,
}

#[derive(Serialize)]
struct Payload<'a> {
    name: &'a str,
}

pub async fn create_project(
    store: &Store,
    actor: &str,
    input: &CreateProjectInput,
) -> Result<Project, CoreError> {
    let key = MutationKey::new(actor, OPERATION, &input.request_id)?;
    let name = validate_name(&input.name)?;
    let id = uuid::Uuid::now_v7().to_string();
    let payload = Payload { name: &name };
    let name = name.clone();

    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                sqlx::query("INSERT INTO projects (id, name) VALUES (?, ?)")
                    .bind(&id)
                    .bind(&name)
                    .execute(&mut *conn)
                    .await?;
                load_project(conn, &id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Proyecto" })
            })
        })
        .await
}
