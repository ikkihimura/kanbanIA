//! Regla de límite WIP. Toda entrada de tarjetas a una columna (crear, mover, destino al
//! eliminar otra columna) debe pasar por `ensure_capacity` dentro de su transacción de
//! escritura: `BEGIN IMMEDIATE` serializa las escrituras, así que dos movimientos
//! simultáneos no pueden superar el límite.

use sqlx::SqliteConnection;

use super::column::Column;
use crate::error::CoreError;

/// Falla con `WipLimitExceeded` si `incoming` tarjetas nuevas superarían el límite de `column`.
/// Las historias archivadas no ocupan sitio.
/// Leer el recuento aquí (no el `card_count` cargado antes) evita usar un dato desfasado.
pub(crate) async fn ensure_capacity(
    conn: &mut SqliteConnection,
    column: &Column,
    incoming: i64,
) -> Result<(), CoreError> {
    let Some(limit) = column.wip_limit else {
        return Ok(());
    };
    let current: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stories WHERE column_id = ? AND archived_at IS NULL",
    )
    .bind(&column.id)
    .fetch_one(&mut *conn)
    .await?;
    if current + incoming > limit {
        return Err(CoreError::WipLimitExceeded {
            column: column.name.clone(),
            limit,
            current,
        });
    }
    Ok(())
}
