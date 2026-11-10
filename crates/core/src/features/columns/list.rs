use super::column::{Column, columns_of_project};
use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::platform::storage::Store;

/// Columnas del proyecto en orden, con su número de tarjetas.
pub async fn list_columns(store: &Store, project_id: &str) -> Result<Vec<Column>, CoreError> {
    let mut conn = store.reader().acquire().await?;
    ensure_project_exists(&mut conn, project_id).await?;
    columns_of_project(&mut conn, project_id).await
}
