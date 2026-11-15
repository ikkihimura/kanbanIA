use sqlx::SqliteConnection;

use super::cancel::cancel_or_request;
use super::event::RunEvent;
use super::run::{RunState, insert_event, require_run};
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

/// Cancela el intento activo de la historia (aceptar, archivar), en la transacción de quien
/// llama: en cola → `cancelled` directo con su evento; con proceso (`preparing`, `running`,
/// `waiting_approval`) → petición de cancelación con el motivo, que confirma el supervisor.
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
    cancel_or_request(conn, &run, cause.reason(), now).await
}

const RESTORED_REASON: &str =
    "Se restauró una copia de los datos: no se puede verificar el estado del intento.";

/// Al restaurar una copia, ningún intento activo de la copia se da por bueno: pasan a
/// `interrupted` (no verificables; nunca se reinician), con la transición normal. Copias
/// anteriores a la tabla: nada que hacer.
///
/// Corre sobre la copia **antes de migrarla**, así que solo usa tablas y columnas de la 0011
/// (`runs`, `run_events`): nada de `select_run!`, que lee las de la 0012 (peticiones de inicio,
/// cancelación). Las peticiones de inicio de una copia más nueva no importan: el intento deja
/// de estar en cola, y el host anula las que queden al arrancar el supervisor.
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
    let active: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT id, state, last_sequence FROM runs \
         WHERE state IN ('queued', 'preparing', 'running', 'waiting_approval')",
    )
    .fetch_all(&mut *restored)
    .await?;
    let event = RunEvent::StateChanged {
        to: RunState::Interrupted,
        reason: Some(RESTORED_REASON.into()),
    };
    let payload = event.to_payload()?;
    for (run_id, state, last_sequence) in &active {
        RunState::parse(state)?.ensure_transition(RunState::Interrupted)?;
        let sequence = last_sequence + 1;
        sqlx::query(
            "UPDATE runs SET state = 'interrupted', last_sequence = ?, updated_at_ms = ?, \
             ended_at_ms = ?, end_reason = ? WHERE id = ? AND last_sequence = ?",
        )
        .bind(sequence)
        .bind(now)
        .bind(now)
        .bind(RESTORED_REASON)
        .bind(run_id)
        .bind(last_sequence)
        .execute(&mut *restored)
        .await?;
        insert_event(restored, run_id, sequence, event.kind(), &payload, now).await?;
    }
    Ok(active.len() as u64)
}
