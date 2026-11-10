use super::epic::{Epic, epics_of_project};
use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::platform::storage::Store;

/// Épicas del proyecto por número, con su avance.
pub async fn list_epics(store: &Store, project_id: &str) -> Result<Vec<Epic>, CoreError> {
    let mut conn = store.reader().acquire().await?;
    ensure_project_exists(&mut conn, project_id).await?;
    epics_of_project(&mut conn, project_id).await
}
