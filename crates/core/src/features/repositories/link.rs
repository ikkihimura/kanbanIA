use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::git_path::validate_repository_path;
use super::repository::{
    ENTITY, LinkRow, RepositoryLink, ensure_not_linked_on_disk, ensure_path_not_linked, load_link,
    map_unique_violation,
};
use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::platform::idempotency::MutationKey;
use crate::platform::storage::Store;

const OPERATION: &str = "repositories.link";

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LinkRepositoryInput {
    pub request_id: String,
    pub project_id: String,
    pub path: String,
}

/// El hash usa la ruta tal como la envió el cliente: un reintento devuelve el resultado
/// original aunque el repositorio se haya movido después.
#[derive(Serialize)]
struct Payload<'a> {
    project_id: &'a str,
    path: &'a str,
}

pub async fn link_repository(
    store: &Store,
    actor: &str,
    input: &LinkRepositoryInput,
) -> Result<RepositoryLink, CoreError> {
    let key = MutationKey::new(actor, OPERATION, &input.request_id)?;
    let payload = Payload {
        project_id: &input.project_id,
        path: &input.path,
    };
    if let Some(previous) = store.replay::<_, LinkRow>(&key, &payload).await? {
        return Ok(previous.into());
    }

    // Disco fuera de la transacción de escritura: validar y comparar identidades.
    let path = validate_repository_path(&input.path)?;
    ensure_not_linked_on_disk(store, &input.project_id, &path, None).await?;

    let project_id = input.project_id.clone();
    let id = uuid::Uuid::now_v7().to_string();
    let row = store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                ensure_project_exists(conn, &project_id).await?;
                ensure_path_not_linked(conn, &project_id, &path, &id).await?;
                sqlx::query("INSERT INTO repository_links (id, project_id, path) VALUES (?, ?, ?)")
                    .bind(&id)
                    .bind(&project_id)
                    .bind(&path)
                    .execute(&mut *conn)
                    .await
                    .map_err(map_unique_violation)?;
                load_link(conn, &project_id, &id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await?;

    // El estado se calcula tras confirmar, fuera del bloqueo de escritura.
    Ok(row.into())
}
