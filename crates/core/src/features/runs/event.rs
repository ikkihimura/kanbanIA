use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::run::{RunState, RunSummary, RunUsage, append_event, require_run, validate_id};
use super::supervisor::{SupervisorToken, ensure_token};
use crate::error::CoreError;
use crate::platform::storage::Store;
use crate::platform::text::{contains_secret_like, has_invisible_formatting};

/// Tope del JSON de un evento (también lo impone la base).
pub const MAX_EVENT_PAYLOAD_BYTES: usize = 8 * 1024;
const MAX_REASON_CHARS: usize = 200;

/// Evento de un intento. Solo cambios de estado y uso: la salida voluminosa de procesos irá a
/// JSONL rotado (HU-018), nunca aquí.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum RunEvent {
    /// Cambio de estado validado por la máquina de estados. `reason` es un texto corto.
    StateChanged {
        to: RunState,
        reason: Option<String>,
    },
    /// Totales acumulados que informa el proveedor; lo que no informa queda `null`.
    UsageReported(RunUsage),
}

impl RunEvent {
    pub(super) fn kind(&self) -> &'static str {
        match self {
            Self::StateChanged { .. } => "state_changed",
            Self::UsageReported(_) => "usage_reported",
        }
    }

    /// JSON que se guarda (y que se compara para detectar duplicados).
    pub(super) fn to_payload(&self) -> Result<String, CoreError> {
        let payload = serde_json::to_string(self)?;
        if payload.len() > MAX_EVENT_PAYLOAD_BYTES {
            return Err(invalid("event", "El evento es demasiado grande.".into()));
        }
        Ok(payload)
    }

    /// Normaliza y valida. Lo usan tanto la fuente externa (supervisor) como el núcleo.
    pub(super) fn normalized(&self) -> Result<Self, CoreError> {
        match self {
            Self::StateChanged { to, reason } => Ok(Self::StateChanged {
                to: *to,
                reason: normalize_reason(reason.as_deref())?,
            }),
            Self::UsageReported(usage) => {
                let values = [
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cost_usd_micros,
                ];
                if values.iter().all(Option::is_none) {
                    return Err(invalid(
                        "event",
                        "Un evento de uso debe informar al menos un valor.".into(),
                    ));
                }
                if values.iter().flatten().any(|v| *v < 0) {
                    return Err(invalid(
                        "event",
                        "El uso y el coste no pueden ser negativos.".into(),
                    ));
                }
                Ok(self.clone())
            }
        }
    }
}

fn normalize_reason(reason: Option<&str>) -> Result<Option<String>, CoreError> {
    let Some(reason) = reason.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    if reason.chars().count() > MAX_REASON_CHARS
        || reason.chars().any(char::is_control)
        || has_invisible_formatting(reason)
        || contains_secret_like(reason)
    {
        return Err(invalid(
            "reason",
            format!("El motivo admite una línea de hasta {MAX_REASON_CHARS} caracteres."),
        ));
    }
    Ok(Some(reason.to_owned()))
}

fn invalid(field: &'static str, reason: String) -> CoreError {
    CoreError::Validation { field, reason }
}

/// Evento numerado que llega de la fuente de un intento (el supervisor de HU-018).
///
/// `sequence` es la siguiente posición esperada de un **contador compartido** por intento: el
/// núcleo también ocupa posiciones (crear = 1, cancelar, aceptar o archivar la historia,
/// restaurar una copia). Tras un `conflict`, el emisor debe releer el intento
/// (`lastSequence`, estado) y resincronizarse antes de volver a emitir; nunca reintentar a
/// ciegas con otra secuencia.
#[derive(Debug, Clone)]
pub struct RecordRunEventInput {
    pub project_id: String,
    pub run_id: String,
    /// Debe ser `lastSequence + 1`; repetir uno ya aplicado con el mismo contenido no hace nada.
    pub sequence: i64,
    pub event: RunEvent,
}

/// Resultado de registrar un evento.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedRunEvent {
    /// `false` si era un duplicado exacto de uno ya aplicado (no se aplicó dos veces).
    pub applied: bool,
    /// Estado vigente del intento tras el evento.
    pub run: RunSummary,
}

/// Registra el evento `sequence` de un intento, en una transacción `BEGIN IMMEDIATE`:
///
/// - `sequence == lastSequence + 1` → se valida (transición incluida) y se aplica;
/// - una secuencia ya aplicada con el mismo contenido → duplicado: no se aplica otra vez y
///   devuelve el estado vigente (`applied = false`);
/// - una secuencia ya aplicada con otro contenido → `conflict` con el intento vigente (p. ej.
///   una persona canceló mientras tanto y ocupó esa secuencia);
/// - un hueco (`sequence > lastSequence + 1`) o una secuencia < 1 → error de validación;
/// - un `StateChanged` → `validation` (los cambios de estado los registra el núcleo).
///
/// API del núcleo para la fuente de eventos de ejecución: exige el [`SupervisorToken`] del
/// almacén (HU-018; token de otro almacén → `forbidden`). No tiene comando Tauri ni ruta del
/// servicio local o MCP: un agente nunca la invoca. Para despachar, terminar o cancelar,
/// el supervisor usa las funciones de `supervisor` (`begin_dispatch`, `finish_run`…), que
/// aplican además las reglas de HU-018; esta queda para eventos de la fuente (uso, HU-019).
pub async fn record_run_event(
    store: &Store,
    token: &SupervisorToken,
    input: &RecordRunEventInput,
) -> Result<RecordedRunEvent, CoreError> {
    ensure_token(store, token)?;
    validate_id("runId", &input.run_id)?;
    if input.sequence < 1 {
        return Err(invalid("sequence", "La secuencia empieza en 1.".into()));
    }
    // Los cambios de estado solo los aplican las funciones del núcleo (crear, cancelar,
    // `begin_dispatch`, `finish_run`…), con sus reglas: la fuente externa solo informa uso.
    if matches!(input.event, RunEvent::StateChanged { .. }) {
        return Err(invalid(
            "event",
            "Los cambios de estado los registra el núcleo; aquí solo se admite uso.".into(),
        ));
    }
    let event = input.event.normalized()?;
    let payload = event.to_payload()?;
    let (input, clock) = (input.clone(), store.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let run = require_run(conn, &input.project_id, &input.run_id).await?;
                if input.sequence <= run.last_sequence {
                    let stored: Option<String> = sqlx::query_scalar(
                        "SELECT payload FROM run_events WHERE run_id = ? AND sequence = ?",
                    )
                    .bind(&run.id)
                    .bind(input.sequence)
                    .fetch_optional(&mut *conn)
                    .await?;
                    return if stored.as_deref() == Some(payload.as_str()) {
                        Ok(RecordedRunEvent {
                            applied: false,
                            run,
                        })
                    } else {
                        Err(CoreError::Conflict {
                            current: serde_json::to_value(&run)?,
                        })
                    };
                }
                if input.sequence > run.last_sequence + 1 {
                    return Err(invalid(
                        "sequence",
                        format!(
                            "Falta el evento {} del intento: los eventos van en orden y sin huecos.",
                            run.last_sequence + 1
                        ),
                    ));
                }
                append_event(conn, &run, &event, now).await?;
                let run = require_run(conn, &input.project_id, &input.run_id).await?;
                Ok(RecordedRunEvent { applied: true, run })
            })
        })
        .await
}
