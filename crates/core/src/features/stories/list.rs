use super::story::{StorySummary, select_story};
use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::platform::storage::Store;

/// Tarjetas del proyecto ordenadas por columna y posición.
pub async fn list_stories(store: &Store, project_id: &str) -> Result<Vec<StorySummary>, CoreError> {
    let mut conn = store.reader().acquire().await?;
    ensure_project_exists(&mut conn, project_id).await?;
    Ok(sqlx::query_as(select_story!(
        "WHERE project_id = ? \
         ORDER BY (SELECT position FROM columns c WHERE c.id = stories.column_id), position"
    ))
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?)
}
