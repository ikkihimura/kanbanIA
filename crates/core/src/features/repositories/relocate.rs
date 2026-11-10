use serde::Deserialize;
use ts_rs::TS;

use super::git_path::validate_repository_path;
use super::repository::{
    ENTITY, RepositoryLink, ensure_not_linked_on_disk, ensure_path_not_linked, load_link,
    map_unique_violation,
};
use crate::error::CoreError;
use crate::platform::revision;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RelocateRepositoryInput {
    pub project_id: String,
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub path: String,
}

/// Apunta un enlace existente a la nueva ubicación del repositorio (movido o renombrado).
pub async fn relocate_repository(
    store: &Store,
    input: &RelocateRepositoryInput,
) -> Result<RepositoryLink, CoreError> {
    // Disco fuera de la transacción de escritura.
    let path = validate_repository_path(&input.path)?;
    ensure_not_linked_on_disk(store, &input.project_id, &path, Some(&input.id)).await?;

    let project_id = input.project_id.clone();
    let id = input.id.clone();
    let expected = input.expected_revision;

    let row = store
        .write(move |conn| {
            Box::pin(async move {
                ensure_path_not_linked(conn, &project_id, &path, &id).await?;
                let rows = sqlx::query(
                    "UPDATE repository_links SET path = ?, revision = revision + 1, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE project_id = ? AND id = ? AND revision = ?",
                )
                .bind(&path)
                .bind(&project_id)
                .bind(&id)
                .bind(expected)
                .execute(&mut *conn)
                .await
                .map_err(map_unique_violation)?
                .rows_affected();
                revision::ensure_updated(ENTITY, rows, load_link(conn, &project_id, &id)).await?;

                load_link(conn, &project_id, &id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await?;

    Ok(row.into())
}
