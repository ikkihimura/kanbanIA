use super::repository::{RepositoryLink, links_of_project};
use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::platform::storage::Store;

/// Repositorios del proyecto con su estado actual en disco.
pub async fn list_repositories(
    store: &Store,
    project_id: &str,
) -> Result<Vec<RepositoryLink>, CoreError> {
    let mut conn = store.reader().acquire().await?;
    ensure_project_exists(&mut conn, project_id).await?;
    let rows = links_of_project(&mut conn, project_id).await?;
    drop(conn);
    // El estado se calcula fuera de la conexión: solo lee metadatos del disco.
    Ok(rows.into_iter().map(RepositoryLink::from).collect())
}
