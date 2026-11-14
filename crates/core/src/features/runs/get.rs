use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::event::RunEvent;
use super::run::{RunSummary, require_run, validate_id};
use super::snapshot::RunSnapshot;
use crate::error::CoreError;
use crate::platform::clock::iso_from_ms;
use crate::platform::storage::Store;

/// Eventos que devuelve el detalle como mucho (los más recientes).
pub const MAX_DETAIL_EVENTS: i64 = 200;

/// Evento registrado de un intento.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunEventRecord {
    #[ts(type = "number")]
    pub sequence: i64,
    pub at: String,
    pub event: RunEvent,
}

/// Detalle de un intento: resumen, instantánea congelada y eventos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunDetail {
    pub run: RunSummary,
    pub snapshot: RunSnapshot,
    /// La instantánea guardada coincide con su SHA-256.
    pub snapshot_intact: bool,
    /// Eventos en orden de secuencia (los últimos `MAX_DETAIL_EVENTS`).
    pub events: Vec<RunEventRecord>,
    /// Hay eventos anteriores que no se devuelven.
    pub events_truncated: bool,
}

/// Detalle de un intento del proyecto (otro proyecto o inexistente → «no encontrado»). Se lee
/// en una transacción de lectura: resumen y eventos son coherentes entre sí.
pub async fn get_run(
    store: &Store,
    project_id: &str,
    run_id: &str,
) -> Result<RunDetail, CoreError> {
    validate_id("runId", run_id)?;
    let mut tx = store.reader().begin().await?;
    let detail = load(&mut tx, project_id, run_id).await;
    tx.rollback().await.ok();
    detail
}

async fn load(
    conn: &mut SqliteConnection,
    project_id: &str,
    run_id: &str,
) -> Result<RunDetail, CoreError> {
    let run = require_run(conn, project_id, run_id).await?;
    let snapshot: String = sqlx::query_scalar("SELECT snapshot FROM runs WHERE id = ?")
        .bind(&run.id)
        .fetch_one(&mut *conn)
        .await?;
    let digest: String = Sha256::digest(snapshot.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let rows: Vec<(i64, String, String)> = sqlx::query_as(concat!(
        "SELECT sequence, ",
        iso_from_ms!("at_ms"),
        " AS at, payload FROM run_events WHERE run_id = ? ORDER BY sequence DESC LIMIT ?"
    ))
    .bind(&run.id)
    .bind(MAX_DETAIL_EVENTS)
    .fetch_all(&mut *conn)
    .await?;
    let events = rows
        .into_iter()
        .rev()
        .map(|(sequence, at, payload)| {
            Ok(RunEventRecord {
                sequence,
                at,
                event: serde_json::from_str(&payload)?,
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let events_truncated = events.first().is_some_and(|e| e.sequence > 1);
    Ok(RunDetail {
        snapshot: serde_json::from_str(&snapshot)?,
        snapshot_intact: digest == run.snapshot_sha256,
        run,
        events,
        events_truncated,
    })
}
