//! Control de revisión optimista.
//!
//! Patrón para entidades editables (columna `revision INTEGER NOT NULL`):
//!
//! ```text
//! UPDATE t SET ..., revision = revision + 1 WHERE id = ? AND revision = ?expected
//! revision::ensure_updated("historia", result.rows_affected(), load_current(conn)).await?
//! ```
//!
//! Si no se actualizó ninguna fila, devuelve `Conflict` con el estado vigente o `NotFound`.
//! Nunca sobrescribe en silencio.

use std::future::Future;

use serde::Serialize;

use crate::error::CoreError;

pub async fn ensure_updated<C, L>(
    entity: &'static str,
    rows_affected: u64,
    load_current: L,
) -> Result<(), CoreError>
where
    C: Serialize,
    L: Future<Output = Result<Option<C>, CoreError>>,
{
    if rows_affected > 0 {
        return Ok(());
    }
    match load_current.await? {
        Some(current) => Err(CoreError::Conflict {
            current: serde_json::to_value(current)?,
        }),
        None => Err(CoreError::NotFound { entity }),
    }
}
