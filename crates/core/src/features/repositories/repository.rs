use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::git_path::{RepositoryStatus, display_name, repository_status, same_directory};
use crate::error::CoreError;
use crate::platform::storage::Store;

pub(super) const ENTITY: &str = "Repositorio";

/// Fila persistida (también el resultado guardado en recibos y el `current` de un conflicto).
/// El estado se calcula al leer, nunca se guarda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinkRow {
    pub id: String,
    pub project_id: String,
    pub path: String,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepositoryLink {
    pub id: String,
    pub project_id: String,
    pub path: String,
    pub name: String,
    #[ts(type = "number")]
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
    pub status: RepositoryStatus,
}

impl From<LinkRow> for RepositoryLink {
    fn from(row: LinkRow) -> Self {
        let status = repository_status(Path::new(&row.path));
        Self {
            name: display_name(&row.path),
            id: row.id,
            project_id: row.project_id,
            path: row.path,
            revision: row.revision,
            created_at: row.created_at,
            updated_at: row.updated_at,
            status,
        }
    }
}

/// SQL estático (sqlx 0.9 rechaza cadenas dinámicas).
macro_rules! select_link {
    ($tail:literal) => {
        concat!(
            "SELECT id, project_id, path, revision, created_at, updated_at \
             FROM repository_links ",
            $tail
        )
    };
}

pub(super) async fn load_link(
    conn: &mut SqliteConnection,
    project_id: &str,
    id: &str,
) -> Result<Option<LinkRow>, CoreError> {
    Ok(
        sqlx::query_as(select_link!("WHERE project_id = ? AND id = ?"))
            .bind(project_id)
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

pub(super) async fn links_of_project(
    conn: &mut SqliteConnection,
    project_id: &str,
) -> Result<Vec<LinkRow>, CoreError> {
    Ok(sqlx::query_as(select_link!(
        "WHERE project_id = ? ORDER BY path COLLATE NOCASE"
    ))
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?)
}

/// Duplicado por identidad de carpeta (otra grafía, enlace simbólico…). Accede al disco, así
/// que se ejecuta *antes* de la transacción de escritura, con una lectura.
pub(super) async fn ensure_not_linked_on_disk(
    store: &Store,
    project_id: &str,
    candidate: &str,
    except_id: Option<&str>,
) -> Result<(), CoreError> {
    let links = {
        let mut conn = store.reader().acquire().await?;
        links_of_project(&mut conn, project_id).await?
    };
    let duplicate = links
        .iter()
        .filter(|link| Some(link.id.as_str()) != except_id)
        .any(|link| same_directory(Path::new(&link.path), Path::new(candidate)));
    if duplicate {
        return Err(already_linked());
    }
    Ok(())
}

/// Comprobación dentro de la transacción: solo SQL, sin disco. Junto con UNIQUE cubre la
/// misma ruta canónica escrita por otra petición concurrente.
pub(super) async fn ensure_path_not_linked(
    conn: &mut SqliteConnection,
    project_id: &str,
    path: &str,
    except_id: &str,
) -> Result<(), CoreError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM repository_links \
         WHERE project_id = ? AND path = ? AND id <> ?)",
    )
    .bind(project_id)
    .bind(path)
    .bind(except_id)
    .fetch_one(&mut *conn)
    .await?;
    if exists {
        return Err(already_linked());
    }
    Ok(())
}

pub(super) fn already_linked() -> CoreError {
    CoreError::AlreadyExists {
        entity: "El repositorio en este proyecto",
        field: "path",
    }
}

/// Traduce la violación de UNIQUE(project_id, path) a un error de dominio.
pub(super) fn map_unique_violation(error: sqlx::Error) -> CoreError {
    match &error {
        sqlx::Error::Database(db) if db.is_unique_violation() => already_linked(),
        _ => CoreError::Database(error),
    }
}
