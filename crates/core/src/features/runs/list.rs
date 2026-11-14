use super::run::{RunSummary, runs_of_story};
use crate::error::CoreError;
use crate::features::stories::ensure_story_exists;
use crate::platform::storage::Store;

/// Intentos de una historia, del más reciente al más antiguo. Historia de otro proyecto o
/// inexistente → «no encontrada».
pub async fn list_story_runs(
    store: &Store,
    project_id: &str,
    story_id: &str,
) -> Result<Vec<RunSummary>, CoreError> {
    let mut conn = store.reader().acquire().await?;
    ensure_story_exists(&mut conn, project_id, story_id).await?;
    runs_of_story(&mut conn, project_id, story_id).await
}
