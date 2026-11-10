//! Colocación de una historia activa al final de otra columna, compartida por las
//! transiciones de revisión (enviar a revisión, aceptar, reabrir). Quien llama aplica antes
//! las reglas de entrada (WIP, «hecho»); aquí solo se mantienen las posiciones densas.

use sqlx::SqliteConnection;

use super::story::{StorySummary, count_in_column};
use crate::error::CoreError;

/// Mueve `story` (activa) al final de `column_id` y sube su revisión. No hace nada si ya está.
pub(crate) async fn append_to_column(
    conn: &mut SqliteConnection,
    story: &StorySummary,
    column_id: &str,
) -> Result<(), CoreError> {
    if story.column_id == column_id {
        return Ok(());
    }
    if let Some(old) = story.position {
        sqlx::query(
            "UPDATE stories SET position = position - 1 WHERE column_id = ? AND position > ?",
        )
        .bind(&story.column_id)
        .bind(old)
        .execute(&mut *conn)
        .await?;
    }
    let position = count_in_column(conn, column_id).await?;
    sqlx::query(
        "UPDATE stories SET column_id = ?, position = ?, \
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
    )
    .bind(column_id)
    .bind(position)
    .bind(&story.id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Recoloca una historia que está al final de `column_id` en `index` (acotado al final),
/// desplazando las siguientes. Mantiene las posiciones densas.
pub(crate) async fn place_at_index(
    conn: &mut SqliteConnection,
    story_id: &str,
    column_id: &str,
    index: i64,
) -> Result<(), CoreError> {
    let last = count_in_column(conn, column_id).await? - 1;
    let index = index.clamp(0, last.max(0));
    if index >= last {
        return Ok(());
    }
    sqlx::query(
        "UPDATE stories SET position = position + 1 \
         WHERE column_id = ? AND position >= ? AND id <> ?",
    )
    .bind(column_id)
    .bind(index)
    .bind(story_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query("UPDATE stories SET position = ? WHERE id = ?")
        .bind(index)
        .bind(story_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}
