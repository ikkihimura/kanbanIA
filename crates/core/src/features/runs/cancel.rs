use serde::{Deserialize, Serialize};
use ts_rs::TS;

use sqlx::SqliteConnection;

use super::run::{RunState, RunSummary, require_run, transition, validate_id};
use crate::error::CoreError;
use crate::platform::actor::Actor;
use crate::platform::idempotency::MutationKey;
use crate::platform::storage::Store;

const OPERATION: &str = "runs.cancel";
const REASON: &str = "Cancelado por una persona.";

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CancelRunInput {
    /// Repítelo al reintentar la misma cancelación.
    pub request_id: String,
    pub project_id: String,
    pub run_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    project_id: &'a str,
    run_id: &'a str,
}

/// Cancela un intento. Solo personas, con `requestId` (reintentar devuelve el resultado
/// original). Una transacción `BEGIN IMMEDIATE`:
///
/// - `queued` (sin proceso) → `cancelled` directamente, con su evento;
/// - `preparing`, `running` o `waiting_approval` → registra una **petición de cancelación**
///   (`cancelRequestedAt`) y el intento sigue en su estado: el supervisor mata el árbol de
///   procesos y registra `cancelled` (`finish_run` con `RunOutcome::Cancelled`). Pedirlo otra
///   vez no cambia nada (conserva la primera petición);
/// - terminado → `invalid_run_transition`.
pub async fn cancel_run(
    store: &Store,
    actor: &Actor,
    input: &CancelRunInput,
) -> Result<RunSummary, CoreError> {
    actor.require_human("cancelar un intento")?;
    validate_id("runId", &input.run_id)?;
    let key = MutationKey::new(actor.receipt_key(), OPERATION, &input.request_id)?;
    let payload = Payload {
        project_id: &input.project_id,
        run_id: &input.run_id,
    };
    let (input, clock) = (input.clone(), store.clone());
    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let run = require_run(conn, &input.project_id, &input.run_id).await?;
                cancel_or_request(conn, &run, REASON, now).await?;
                require_run(conn, &input.project_id, &input.run_id).await
            })
        })
        .await
}

/// Cancela un intento en cola o pide la cancelación de uno con proceso, en la transacción de
/// quien llama. Terminado → `invalid_run_transition`.
pub(super) async fn cancel_or_request(
    conn: &mut SqliteConnection,
    run: &RunSummary,
    reason: &str,
    now: i64,
) -> Result<(), CoreError> {
    match run.state {
        RunState::Queued => transition(conn, run, RunState::Cancelled, Some(reason), now).await,
        RunState::Preparing | RunState::Running | RunState::WaitingApproval => {
            // La primera petición manda: repetirla no cambia el motivo ni la hora.
            sqlx::query(
                "UPDATE runs SET cancel_requested_at_ms = ?, cancel_reason = ?, updated_at_ms = ? \
                 WHERE id = ? AND cancel_requested_at_ms IS NULL",
            )
            .bind(now)
            .bind(reason)
            .bind(now)
            .bind(&run.id)
            .execute(&mut *conn)
            .await?;
            Ok(())
        }
        RunState::Completed | RunState::Failed | RunState::Cancelled | RunState::Interrupted => {
            Err(CoreError::InvalidRunTransition {
                from: run.state.label(),
                to: RunState::Cancelled.label(),
            })
        }
    }
}
