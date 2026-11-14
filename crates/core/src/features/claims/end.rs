use sqlx::SqliteConnection;

use crate::error::CoreError;

/// Cierra el reclamo abierto de la historia salvo que sea de `keep_agent`. Lo usa
/// `relations::set_story_agent`: reasignar o quitar la asignación quita también el reclamo
/// del agente anterior, que recibe `claim_ended` al renovar. Uno ya vencido se registra como
/// vencido. Va dentro de la transacción de quien llama.
pub(crate) async fn end_open_claims(
    conn: &mut SqliteConnection,
    story_id: &str,
    keep_agent: Option<&str>,
    now: i64,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE story_claims SET \
             ended_at_ms = MIN(?, expires_at_ms), \
             end_reason = CASE WHEN expires_at_ms <= ? THEN 'expired' ELSE 'released' END \
         WHERE story_id = ? AND ended_at_ms IS NULL AND (? IS NULL OR agent_id <> ?)",
    )
    .bind(now)
    .bind(now)
    .bind(story_id)
    .bind(keep_agent)
    .bind(keep_agent)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Motivo con el que se cierra un reclamo vigente (uno ya vencido se registra como vencido).
#[derive(Debug, Clone, Copy)]
pub(crate) enum EndReason {
    /// El agente entregó (HU-015).
    Delivered,
    /// La historia se aceptó o archivó.
    Closed,
}

impl EndReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Closed => "closed",
        }
    }
}

/// Cierra el reclamo abierto de la historia, sea de quien sea (aceptar, archivar).
pub(crate) async fn close_story_claims(
    conn: &mut SqliteConnection,
    story_id: &str,
    reason: EndReason,
    now: i64,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE story_claims SET \
             ended_at_ms = MIN(?, expires_at_ms), \
             end_reason = CASE WHEN expires_at_ms <= ? THEN 'expired' ELSE ? END \
         WHERE story_id = ? AND ended_at_ms IS NULL",
    )
    .bind(now)
    .bind(now)
    .bind(reason.as_str())
    .bind(story_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Cierra un reclamo concreto (entregar).
pub(crate) async fn end_claim(
    conn: &mut SqliteConnection,
    claim_id: &str,
    reason: EndReason,
    now: i64,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE story_claims SET \
             ended_at_ms = MIN(?, expires_at_ms), \
             end_reason = CASE WHEN expires_at_ms <= ? THEN 'expired' ELSE ? END \
         WHERE id = ? AND ended_at_ms IS NULL",
    )
    .bind(now)
    .bind(now)
    .bind(reason.as_str())
    .bind(claim_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Cierra los reclamos abiertos hechos con una credencial (al revocarla). La asignación se
/// queda, como al vencer.
pub(crate) async fn close_credential_claims(
    conn: &mut SqliteConnection,
    credential_id: &str,
    now: i64,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE story_claims SET \
             ended_at_ms = MIN(?, expires_at_ms), \
             end_reason = CASE WHEN expires_at_ms <= ? THEN 'expired' ELSE 'revoked' END \
         WHERE credential_id = ? AND ended_at_ms IS NULL",
    )
    .bind(now)
    .bind(now)
    .bind(credential_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Al restaurar una copia, ningún reclamo de la copia sigue abierto: una persona pudo
/// liberarlo después y los recibos de idempotencia también se revierten. Falla de forma
/// segura: el agente recibe `claim_ended` y vuelve a reclamar. Copias anteriores a la tabla:
/// nada que hacer.
pub(crate) async fn close_restored_claims(
    restored: &mut SqliteConnection,
    now: i64,
) -> Result<u64, CoreError> {
    let has_table: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master \
         WHERE type = 'table' AND name = 'story_claims')",
    )
    .fetch_one(&mut *restored)
    .await?;
    if !has_table {
        return Ok(0);
    }
    Ok(sqlx::query(
        "UPDATE story_claims SET \
             ended_at_ms = MIN(?, expires_at_ms), \
             end_reason = CASE WHEN expires_at_ms <= ? THEN 'expired' ELSE 'released' END \
         WHERE ended_at_ms IS NULL",
    )
    .bind(now)
    .bind(now)
    .execute(&mut *restored)
    .await?
    .rows_affected())
}
