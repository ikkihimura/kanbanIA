use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::platform::storage::Store;

/// Una historia que necesita atención. Aparece una sola vez aunque tenga varios motivos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AttentionItem {
    pub story_id: String,
    #[ts(type = "number")]
    pub number: i64,
    pub title: String,
    pub column_name: String,
    /// Bloqueo marcado (manual o reportado).
    pub blocker_reason: Option<String>,
    /// Prerequisitos sin aceptar; solo cuenta en columnas listo/en curso/revisión.
    #[ts(type = "number")]
    pub pending_prerequisites: i64,
    #[ts(type = "number")]
    pub pending_deliveries: i64,
    /// Instante más reciente entre los motivos (para ordenar).
    pub since: String,
}

/// Bandeja de atención del proyecto: historias no archivadas ni aceptadas con un bloqueo,
/// entregas pendientes de revisión o prerequisitos sin aceptar una vez planificadas
/// (listo, en curso o revisión; en el backlog es normal tener dependencias). Sin duplicados.
pub async fn list_attention(
    store: &Store,
    project_id: &str,
) -> Result<Vec<AttentionItem>, CoreError> {
    let mut conn = store.reader().acquire().await?;
    ensure_project_exists(&mut conn, project_id).await?;
    Ok(sqlx::query_as(
        "SELECT * FROM ( \
           SELECT s.id AS story_id, s.number, s.title, c.name AS column_name, \
             b.reason AS blocker_reason, \
             CASE WHEN c.category IN ('ready', 'active', 'review') THEN \
               (SELECT COUNT(*) FROM story_dependencies d JOIN stories p ON p.id = d.depends_on_id \
                WHERE d.story_id = s.id AND p.accepted_at IS NULL) ELSE 0 END \
               AS pending_prerequisites, \
             (SELECT COUNT(*) FROM deliveries v WHERE v.story_id = s.id AND v.status = 'pending') \
               AS pending_deliveries, \
             MAX(COALESCE(b.created_at, ''), \
                 COALESCE((SELECT MAX(v.created_at) FROM deliveries v \
                           WHERE v.story_id = s.id AND v.status = 'pending'), ''), \
                 COALESCE((SELECT MAX(d.created_at) FROM story_dependencies d \
                           JOIN stories p ON p.id = d.depends_on_id \
                           WHERE d.story_id = s.id AND p.accepted_at IS NULL), '')) AS since \
           FROM stories s \
           JOIN columns c ON c.id = s.column_id \
           LEFT JOIN story_blockers b ON b.story_id = s.id \
           WHERE s.project_id = ? AND s.archived_at IS NULL AND s.accepted_at IS NULL \
         ) WHERE blocker_reason IS NOT NULL OR pending_prerequisites > 0 \
                 OR pending_deliveries > 0 \
         ORDER BY since DESC, number",
    )
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?)
}
