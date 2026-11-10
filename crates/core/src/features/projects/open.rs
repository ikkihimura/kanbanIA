use super::project::{ENTITY, Project, load_project, select_project};
use crate::error::CoreError;
use crate::platform::storage::Store;

/// Abre un proyecto: registra la apertura (no es una edición, no cambia `revision`).
pub async fn open_project(store: &Store, id: &str) -> Result<Project, CoreError> {
    let id = id.to_owned();
    store
        .write(move |conn| {
            Box::pin(async move {
                // Estrictamente creciente aunque dos aperturas caigan en el mismo milisegundo:
                // el orden por recencia (y "reabrir el último") nunca depende de un empate.
                let rows = sqlx::query(
                    "UPDATE projects SET last_opened_at = max( \
                         strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), \
                         coalesce( \
                             (SELECT strftime('%Y-%m-%dT%H:%M:%fZ', max(last_opened_at), \
                                              '+0.001 seconds') FROM projects), \
                             '')) \
                     WHERE id = ?",
                )
                .bind(&id)
                .execute(&mut *conn)
                .await?
                .rows_affected();
                if rows == 0 {
                    return Err(CoreError::NotFound { entity: ENTITY });
                }
                load_project(conn, &id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}

/// Último proyecto abierto, si existe, para reabrirlo al iniciar.
pub async fn last_opened_project(store: &Store) -> Result<Option<Project>, CoreError> {
    Ok(sqlx::query_as(select_project!(
        "WHERE last_opened_at IS NOT NULL ORDER BY last_opened_at DESC LIMIT 1"
    ))
    .fetch_optional(store.reader())
    .await?)
}
