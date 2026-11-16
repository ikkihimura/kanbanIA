use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::approval::{RunApproval, approvals_of_run};
use super::event::RunEvent;
use super::run::{RunSummary, require_run, validate_id};
use super::snapshot::RunSnapshot;
use super::start::RunExecutor;
use super::worktree::{RunWorktree, load_worktree};
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

/// Última petición de inicio de un intento (HU-018).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunStartRequest {
    pub repository_id: String,
    /// Ejecutor elegido: proceso de prueba (sin IA, con su escenario) o Claude Code vía ACP.
    pub executor: RunExecutor,
    pub requested_at: String,
    /// El supervisor lo despachó (salió de la cola).
    pub dispatched_at: Option<String>,
    /// Se anuló al reiniciar la app sin haberse despachado: hay que volver a pedirlo.
    pub voided_at: Option<String>,
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
    /// Última petición de inicio (HU-018), si la hubo.
    #[serde(default)]
    pub start_request: Option<RunStartRequest>,
    /// Motivo de la petición de cancelación pendiente o atendida.
    #[serde(default)]
    pub cancel_reason: Option<String>,
    /// Worktree del intento (ruta absoluta local; nunca para agentes).
    #[serde(default)]
    pub worktree: Option<RunWorktree>,
    /// Peticiones de permiso del agente (HU-020), la más reciente primero.
    #[serde(default)]
    pub approvals: Vec<RunApproval>,
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
    let (snapshot, cancel_reason): (String, Option<String>) =
        sqlx::query_as("SELECT snapshot, cancel_reason FROM runs WHERE id = ?")
            .bind(&run.id)
            .fetch_one(&mut *conn)
            .await?;
    let start_request = load_start_request(conn, &run.id).await?;
    let worktree = load_worktree(conn, &run.id).await?;
    let approvals = approvals_of_run(conn, &run.id).await?;
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
        start_request,
        cancel_reason,
        worktree,
        approvals,
    })
}

/// Petición de inicio: repositorio, ejecutor, escenario, pedida, despachada, anulada.
type StartRequestRow = (
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
);

async fn load_start_request(
    conn: &mut SqliteConnection,
    run_id: &str,
) -> Result<Option<RunStartRequest>, CoreError> {
    let row: Option<StartRequestRow> = sqlx::query_as(concat!(
        "SELECT repository_id, executor, scenario, ",
        iso_from_ms!("requested_at_ms"),
        ", ",
        iso_from_ms!("dispatched_at_ms"),
        ", ",
        iso_from_ms!("voided_at_ms"),
        " FROM run_start_requests WHERE run_id = ? ORDER BY id DESC LIMIT 1"
    ))
    .bind(run_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(
        |(repository_id, executor, scenario, requested_at, dispatched_at, voided_at)| {
            Ok(RunStartRequest {
                repository_id,
                executor: RunExecutor::from_columns(&executor, scenario.as_deref())?,
                requested_at,
                dispatched_at,
                voided_at,
            })
        },
    )
    .transpose()
}
