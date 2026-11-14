use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::event::RunEvent;
use super::run::{RunState, RunSummary, append_event, require_run, validate_id};
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

/// Cancela un intento activo. Solo personas. El núcleo asigna el siguiente número de la
/// secuencia de eventos. Reintentar con el mismo `requestId` devuelve el resultado original;
/// cancelar un intento ya terminado es `invalid_run_transition`.
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
                let event = RunEvent::StateChanged {
                    to: RunState::Cancelled,
                    reason: Some(REASON.into()),
                };
                append_event(conn, &run, &event, now).await?;
                require_run(conn, &input.project_id, &input.run_id).await
            })
        })
        .await
}
