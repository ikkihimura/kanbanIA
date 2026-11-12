use super::story_claim::{ClaimRow, StoryClaim, select_claim};
use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::platform::storage::Store;

/// Reclamos vigentes del proyecto (para el tablero y el editor), por número de historia. Los de
/// historias archivadas o aceptadas no cuentan: ya no se pueden renovar.
pub async fn list_active_claims(
    store: &Store,
    project_id: &str,
) -> Result<Vec<StoryClaim>, CoreError> {
    let mut conn = store.reader().acquire().await?;
    ensure_project_exists(&mut conn, project_id).await?;
    let rows: Vec<ClaimRow> = sqlx::query_as(select_claim!(
        "WHERE k.project_id = ? AND k.ended_at_ms IS NULL AND k.expires_at_ms > ? \
         AND s.archived_at IS NULL AND s.accepted_at IS NULL ORDER BY s.number"
    ))
    .bind(project_id)
    .bind(store.now_ms())
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(StoryClaim::from).collect())
}
