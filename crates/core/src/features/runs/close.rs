use sqlx::SqliteConnection;

use super::event::RunEvent;
use super::run::{RunState, append_event, require_run};
use crate::error::CoreError;

/// Por qué una historia deja de admitir intentos activos.
#[derive(Debug, Clone, Copy)]
pub(crate) enum StoryClosed {
    Accepted,
    Archived,
}

impl StoryClosed {
    fn reason(self) -> &'static str {
        match self {
            Self::Accepted => "La historia se aceptó.",
            Self::Archived => "La historia se archivó.",
        }
    }
}

/// Cancela el intento activo de la historia (aceptar, archivar), con su evento y en la
/// transacción de quien llama.
pub(crate) async fn cancel_story_runs(
    conn: &mut SqliteConnection,
    story_id: &str,
    cause: StoryClosed,
    now: i64,
) -> Result<(), CoreError> {
    let active: Option<(String, String)> = sqlx::query_as(
        "SELECT project_id, id FROM runs WHERE story_id = ? \
         AND state IN ('queued', 'preparing', 'running', 'waiting_approval')",
    )
    .bind(story_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((project_id, run_id)) = active else {
        return Ok(());
    };
    let run = require_run(conn, &project_id, &run_id).await?;
    let event = RunEvent::StateChanged {
        to: RunState::Cancelled,
        reason: Some(cause.reason().into()),
    };
    append_event(conn, &run, &event, now).await
}

const RESTORED_REASON: &str =
    "Se restauró una copia de los datos: no se puede verificar el estado del intento.";

/// Al restaurar una copia, ningún intento activo de la copia se da por bueno: pasan a
/// `interrupted` (no verificables; nunca se reinician), con la transición normal. Copias
/// anteriores a la tabla: nada que hacer.
pub(crate) async fn interrupt_restored_runs(
    restored: &mut SqliteConnection,
    now: i64,
) -> Result<u64, CoreError> {
    let has_table: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'runs')",
    )
    .fetch_one(&mut *restored)
    .await?;
    if !has_table {
        return Ok(0);
    }
    let active: Vec<(String, String)> = sqlx::query_as(
        "SELECT project_id, id FROM runs \
         WHERE state IN ('queued', 'preparing', 'running', 'waiting_approval')",
    )
    .fetch_all(&mut *restored)
    .await?;
    let event = RunEvent::StateChanged {
        to: RunState::Interrupted,
        reason: Some(RESTORED_REASON.into()),
    };
    for (project_id, run_id) in &active {
        let run = require_run(restored, project_id, run_id).await?;
        append_event(restored, &run, &event, now).await?;
    }
    Ok(active.len() as u64)
}
