use super::story::{ENTITY, StoryDetail, StorySummary, list_rows, load_detail};
use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::platform::storage::Store;

/// Historias del proyecto por columna y posición; las archivadas, al final y solo si se piden.
pub async fn list_stories(
    store: &Store,
    project_id: &str,
    include_archived: bool,
) -> Result<Vec<StorySummary>, CoreError> {
    let mut conn = store.reader().acquire().await?;
    ensure_project_exists(&mut conn, project_id).await?;
    list_rows(&mut conn, project_id, include_archived).await
}

/// Historia completa (incluidas las archivadas: consultarlas no las modifica).
pub async fn get_story(
    store: &Store,
    project_id: &str,
    id: &str,
) -> Result<StoryDetail, CoreError> {
    let mut conn = store.reader().acquire().await?;
    load_detail(&mut conn, project_id, id)
        .await?
        .ok_or(CoreError::NotFound { entity: ENTITY })
}
