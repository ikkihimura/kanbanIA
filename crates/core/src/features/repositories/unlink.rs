use serde::Deserialize;
use ts_rs::TS;

use super::repository::{ENTITY, load_link};
use crate::error::CoreError;
use crate::platform::revision;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UnlinkRepositoryInput {
    pub project_id: String,
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
}

/// Elimina solo el enlace. No toca archivos del repositorio ni datos del proyecto.
pub async fn unlink_repository(
    store: &Store,
    input: &UnlinkRepositoryInput,
) -> Result<(), CoreError> {
    let project_id = input.project_id.clone();
    let id = input.id.clone();
    let expected = input.expected_revision;

    store
        .write(move |conn| {
            Box::pin(async move {
                let rows = sqlx::query(
                    "DELETE FROM repository_links \
                     WHERE project_id = ? AND id = ? AND revision = ?",
                )
                .bind(&project_id)
                .bind(&id)
                .bind(expected)
                .execute(&mut *conn)
                .await?
                .rows_affected();
                // El estado vigente del conflicto es la fila guardada: sin acceso a disco.
                revision::ensure_updated(ENTITY, rows, load_link(conn, &project_id, &id)).await
            })
        })
        .await
}
