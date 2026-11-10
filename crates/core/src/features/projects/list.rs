use serde::Serialize;
use ts_rs::TS;

use crate::error::CoreError;
use crate::platform::storage::Store;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    #[ts(type = "number")]
    pub revision: i64,
    pub last_opened_at: Option<String>,
    #[ts(type = "number")]
    pub repository_count: i64,
}

/// Proyectos ordenados por apertura más reciente; los nunca abiertos, al final por nombre.
pub async fn list_projects(store: &Store) -> Result<Vec<ProjectSummary>, CoreError> {
    Ok(sqlx::query_as(
        "SELECT p.id, p.name, p.revision, p.last_opened_at, \
                (SELECT COUNT(*) FROM repository_links r WHERE r.project_id = p.id) \
                    AS repository_count \
         FROM projects p \
         ORDER BY p.last_opened_at IS NULL, p.last_opened_at DESC, p.name COLLATE NOCASE",
    )
    .fetch_all(store.reader())
    .await?)
}
