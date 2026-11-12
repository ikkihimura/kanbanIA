use super::credential::{AgentCredential, select_credential};
use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::platform::storage::Store;

/// Credenciales del proyecto: activas primero, después revocadas; las más nuevas arriba.
pub async fn list_agent_credentials(
    store: &Store,
    project_id: &str,
) -> Result<Vec<AgentCredential>, CoreError> {
    let mut conn = store.reader().acquire().await?;
    ensure_project_exists(&mut conn, project_id).await?;
    Ok(sqlx::query_as(select_credential!(
        "WHERE c.project_id = ? \
         ORDER BY c.revoked_at IS NOT NULL, c.created_at DESC, c.id DESC"
    ))
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?)
}
