//! Revocaciones y copias de seguridad: restaurar una copia antigua no debe reactivar una
//! credencial que la persona revocó después (p. ej. porque se filtró).

use sqlx::SqliteConnection;

use crate::error::CoreError;

/// Lo que se sabe de las revocaciones de la base vigente antes de restaurar.
pub(crate) enum CurrentRevocations {
    /// `(token_hash, revoked_at)` de las credenciales revocadas en la base vigente.
    Known(Vec<(String, String)>),
    /// No hay base vigente, no se pudo leer o no tiene la tabla: no se sabe qué se revocó.
    Unknown,
}

/// Lee las revocaciones de la base vigente (`None` si no se puede abrir).
pub(crate) async fn current_revocations(conn: Option<&mut SqliteConnection>) -> CurrentRevocations {
    let Some(conn) = conn else {
        return CurrentRevocations::Unknown;
    };
    match sqlx::query_as(
        "SELECT token_hash, revoked_at FROM agent_credentials WHERE revoked_at IS NOT NULL",
    )
    .fetch_all(&mut *conn)
    .await
    {
        Ok(rows) => CurrentRevocations::Known(rows),
        Err(_) => CurrentRevocations::Unknown,
    }
}

/// Aplica las revocaciones a la base que se va a restaurar. Si no se conocen, revoca todas
/// las credenciales activas de la copia (falla de forma segura: basta con crear otras).
/// Devuelve cuántas credenciales quedaron revocadas por esta reconciliación.
pub(crate) async fn reconcile_restored(
    restored: &mut SqliteConnection,
    current: &CurrentRevocations,
) -> Result<u64, CoreError> {
    let has_table: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master \
         WHERE type = 'table' AND name = 'agent_credentials')",
    )
    .fetch_one(&mut *restored)
    .await?;
    if !has_table {
        // Copia anterior a las credenciales: no trae ninguna.
        return Ok(0);
    }
    let mut revoked = 0;
    match current {
        CurrentRevocations::Known(rows) => {
            for (hash, revoked_at) in rows {
                revoked += sqlx::query(
                    "UPDATE agent_credentials SET revoked_at = ? \
                     WHERE token_hash = ? AND revoked_at IS NULL",
                )
                .bind(revoked_at)
                .bind(hash)
                .execute(&mut *restored)
                .await?
                .rows_affected();
            }
        }
        CurrentRevocations::Unknown => {
            revoked = sqlx::query(
                "UPDATE agent_credentials \
                 SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                 WHERE revoked_at IS NULL",
            )
            .execute(&mut *restored)
            .await?
            .rows_affected();
        }
    }
    Ok(revoked)
}
