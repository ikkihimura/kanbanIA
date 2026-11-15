use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::event::RunEvent;
use crate::error::CoreError;
use crate::platform::clock::iso_from_ms;

pub(super) const ENTITY: &str = "Intento";
const MAX_ID_LEN: usize = 64;

/// Estado de un intento (ver `docs/ARQUITECTURA.md`, «Ejecución y observación»).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunState {
    Queued,
    Preparing,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl RunState {
    pub const ALL: [Self; 8] = [
        Self::Queued,
        Self::Preparing,
        Self::Running,
        Self::WaitingApproval,
        Self::Completed,
        Self::Failed,
        Self::Cancelled,
        Self::Interrupted,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self, CoreError> {
        Self::ALL
            .into_iter()
            .find(|s| s.as_str() == value)
            .ok_or_else(|| {
                CoreError::Database(sqlx::Error::Decode(
                    format!("estado de intento desconocido: {value}").into(),
                ))
            })
    }

    /// Etiqueta en español para los mensajes de error.
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "en cola",
            Self::Preparing => "preparando",
            Self::Running => "en ejecución",
            Self::WaitingApproval => "esperando aprobación",
            Self::Completed => "completado",
            Self::Failed => "fallido",
            Self::Cancelled => "cancelado",
            Self::Interrupted => "interrumpido",
        }
    }

    /// Completado, fallido, cancelado o interrumpido: ya no cambia de estado.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    /// Máquina de estados. Cualquier otro cambio (incluido quedarse igual) se rechaza.
    pub fn can_transition_to(self, to: Self) -> bool {
        use RunState::{
            Cancelled, Completed, Failed, Interrupted, Preparing, Queued, Running, WaitingApproval,
        };
        matches!(
            (self, to),
            (Queued, Preparing | Cancelled | Failed | Interrupted)
                | (Preparing, Running | Failed | Cancelled | Interrupted)
                | (
                    Running,
                    WaitingApproval | Completed | Failed | Cancelled | Interrupted
                )
                | (WaitingApproval, Running | Failed | Cancelled | Interrupted)
        )
    }

    pub(super) fn ensure_transition(self, to: Self) -> Result<(), CoreError> {
        if self.can_transition_to(to) {
            Ok(())
        } else {
            Err(CoreError::InvalidRunTransition {
                from: self.label(),
                to: to.label(),
            })
        }
    }
}

/// Uso y coste informados por el proveedor. `null` = no informado (se muestra «no
/// disponible», nunca 0).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunUsage {
    #[ts(type = "number | null")]
    pub input_tokens: Option<i64>,
    #[ts(type = "number | null")]
    pub output_tokens: Option<i64>,
    /// Millonésimas de dólar estadounidense.
    #[ts(type = "number | null")]
    pub cost_usd_micros: Option<i64>,
}

/// Intento en listas y en la cabecera del detalle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunSummary {
    pub id: String,
    pub project_id: String,
    pub story_id: String,
    /// Se muestra como «Intento #n»; no se reutiliza.
    #[ts(type = "number")]
    pub number: i64,
    pub state: RunState,
    /// Agente del intento (nombre y número actuales; los congelados están en la instantánea).
    pub agent_id: String,
    #[ts(type = "number")]
    pub agent_number: i64,
    pub agent_name: String,
    pub created_at: String,
    pub updated_at: String,
    /// Solo en estados terminales.
    pub ended_at: Option<String>,
    /// Motivo corto del final, generado por el núcleo.
    pub end_reason: Option<String>,
    pub usage: RunUsage,
    /// Secuencia del último evento aplicado.
    #[ts(type = "number")]
    pub last_sequence: i64,
    /// SHA-256 de la instantánea guardada (permite comprobar que no cambió).
    pub snapshot_sha256: String,
    /// Hora de la petición de inicio pendiente (HU-018): el intento espera hueco en la cola.
    /// `null` si no hay petición o ya se despachó o anuló.
    #[serde(default)]
    pub start_requested_at: Option<String>,
    /// Hora de la petición de cancelación (HU-018): el supervisor aún no ha parado el proceso.
    /// Se conserva cuando el intento termina.
    #[serde(default)]
    pub cancel_requested_at: Option<String>,
}

#[derive(sqlx::FromRow)]
struct RunRow {
    id: String,
    project_id: String,
    story_id: String,
    number: i64,
    state: String,
    agent_id: String,
    agent_number: i64,
    agent_name: String,
    created_at: String,
    updated_at: String,
    ended_at: Option<String>,
    end_reason: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cost_usd_micros: Option<i64>,
    last_sequence: i64,
    snapshot_sha256: String,
    start_requested_at: Option<String>,
    cancel_requested_at: Option<String>,
}

impl TryFrom<RunRow> for RunSummary {
    type Error = CoreError;

    fn try_from(row: RunRow) -> Result<Self, CoreError> {
        Ok(Self {
            state: RunState::parse(&row.state)?,
            id: row.id,
            project_id: row.project_id,
            story_id: row.story_id,
            number: row.number,
            agent_id: row.agent_id,
            agent_number: row.agent_number,
            agent_name: row.agent_name,
            created_at: row.created_at,
            updated_at: row.updated_at,
            ended_at: row.ended_at,
            end_reason: row.end_reason,
            usage: RunUsage {
                input_tokens: row.input_tokens,
                output_tokens: row.output_tokens,
                cost_usd_micros: row.cost_usd_micros,
            },
            last_sequence: row.last_sequence,
            snapshot_sha256: row.snapshot_sha256,
            start_requested_at: row.start_requested_at,
            cancel_requested_at: row.cancel_requested_at,
        })
    }
}

/// SQL estático (sqlx 0.9 rechaza cadenas dinámicas).
macro_rules! select_run {
    ($tail:literal) => {
        concat!(
            "SELECT r.id, r.project_id, r.story_id, r.number, r.state, r.agent_id, \
             a.number AS agent_number, a.name AS agent_name, ",
            iso_from_ms!("r.created_at_ms"),
            " AS created_at, ",
            iso_from_ms!("r.updated_at_ms"),
            " AS updated_at, ",
            iso_from_ms!("r.ended_at_ms"),
            " AS ended_at, r.end_reason, r.input_tokens, r.output_tokens, r.cost_usd_micros, \
             r.last_sequence, r.snapshot_sha256, (SELECT ",
            iso_from_ms!("q.requested_at_ms"),
            " FROM run_start_requests q WHERE q.run_id = r.id AND q.voided_at_ms IS NULL \
             AND q.dispatched_at_ms IS NULL AND r.state = 'queued') AS start_requested_at, ",
            iso_from_ms!("r.cancel_requested_at_ms"),
            " AS cancel_requested_at \
             FROM runs r JOIN agents a ON a.id = r.agent_id ",
            $tail
        )
    };
}

/// Intento del proyecto; de otro proyecto o inexistente → `None`, sin distinguir.
pub(super) async fn load_run(
    conn: &mut SqliteConnection,
    project_id: &str,
    run_id: &str,
) -> Result<Option<RunSummary>, CoreError> {
    let row: Option<RunRow> = sqlx::query_as(select_run!("WHERE r.project_id = ? AND r.id = ?"))
        .bind(project_id)
        .bind(run_id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(RunSummary::try_from).transpose()
}

pub(super) async fn require_run(
    conn: &mut SqliteConnection,
    project_id: &str,
    run_id: &str,
) -> Result<RunSummary, CoreError> {
    load_run(conn, project_id, run_id)
        .await?
        .ok_or(CoreError::NotFound { entity: ENTITY })
}

pub(super) async fn runs_of_story(
    conn: &mut SqliteConnection,
    project_id: &str,
    story_id: &str,
) -> Result<Vec<RunSummary>, CoreError> {
    let rows: Vec<RunRow> = sqlx::query_as(select_run!(
        "WHERE r.project_id = ? AND r.story_id = ? ORDER BY r.number DESC"
    ))
    .bind(project_id)
    .bind(story_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter().map(RunSummary::try_from).collect()
}

/// Número del intento activo de la historia, si lo hay.
pub(super) async fn active_run_number(
    conn: &mut SqliteConnection,
    story_id: &str,
) -> Result<Option<i64>, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT number FROM runs WHERE story_id = ? \
         AND state IN ('queued', 'preparing', 'running', 'waiting_approval')",
    )
    .bind(story_id)
    .fetch_optional(&mut *conn)
    .await?)
}

/// Intento por id, sin acotar al proyecto. Solo para el supervisor y el núcleo: las entradas de
/// personas usan `require_run` con su proyecto.
pub(super) async fn require_run_any(
    conn: &mut SqliteConnection,
    run_id: &str,
) -> Result<RunSummary, CoreError> {
    let row: Option<RunRow> = sqlx::query_as(select_run!("WHERE r.id = ?"))
        .bind(run_id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(RunSummary::try_from)
        .transpose()?
        .ok_or(CoreError::NotFound { entity: ENTITY })
}

/// Cambia de estado con su evento (siguiente secuencia), en la transacción de quien llama.
pub(super) async fn transition(
    conn: &mut SqliteConnection,
    run: &RunSummary,
    to: RunState,
    reason: Option<&str>,
    now: i64,
) -> Result<(), CoreError> {
    let event = RunEvent::StateChanged {
        to,
        reason: reason.map(short_reason),
    };
    append_event(conn, run, &event, now).await
}

/// Motivo de como mucho `MAX_REASON_CHARS` caracteres (lo exige la base).
pub(super) fn short_reason(reason: &str) -> String {
    const MAX_REASON_CHARS: usize = 200;
    if reason.chars().count() <= MAX_REASON_CHARS {
        return reason.to_owned();
    }
    let mut cut: String = reason.chars().take(MAX_REASON_CHARS - 1).collect();
    cut.push('…');
    cut
}

pub(super) fn validate_id(field: &'static str, id: &str) -> Result<(), CoreError> {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return Err(CoreError::Validation {
            field,
            reason: format!("{field} no válido."),
        });
    }
    Ok(())
}

/// Añade el evento como siguiente de la secuencia y aplica su efecto sobre el intento: valida
/// la transición o que los totales de uso no disminuyan. Dentro de la transacción de quien
/// llama.
pub(super) async fn append_event(
    conn: &mut SqliteConnection,
    run: &RunSummary,
    event: &RunEvent,
    now: i64,
) -> Result<(), CoreError> {
    let sequence = run.last_sequence + 1;
    let payload = event.to_payload()?;
    let updated = match event {
        RunEvent::StateChanged { to, reason } => {
            run.state.ensure_transition(*to)?;
            let terminal = to.is_terminal();
            sqlx::query(
                "UPDATE runs SET state = ?, last_sequence = ?, updated_at_ms = ?, \
                 ended_at_ms = ?, end_reason = ? WHERE id = ? AND last_sequence = ?",
            )
            .bind(to.as_str())
            .bind(sequence)
            .bind(now)
            .bind(terminal.then_some(now))
            .bind(if terminal { reason.as_deref() } else { None })
            .bind(&run.id)
            .bind(run.last_sequence)
            .execute(&mut *conn)
            .await
        }
        // Totales acumulados del proveedor: lo que no informa se queda como estaba. Se admite
        // también en un intento terminado (el coste puede llegar después).
        RunEvent::UsageReported(usage) => {
            ensure_not_decreasing(&run.usage, usage)?;
            sqlx::query(
                "UPDATE runs SET input_tokens = COALESCE(?, input_tokens), \
                 output_tokens = COALESCE(?, output_tokens), \
                 cost_usd_micros = COALESCE(?, cost_usd_micros), \
                 last_sequence = ?, updated_at_ms = ? WHERE id = ? AND last_sequence = ?",
            )
            .bind(usage.input_tokens)
            .bind(usage.output_tokens)
            .bind(usage.cost_usd_micros)
            .bind(sequence)
            .bind(now)
            .bind(&run.id)
            .bind(run.last_sequence)
            .execute(&mut *conn)
            .await
        }
        // Dato reportado por el agente: no cambia estado ni uso, solo ocupa la secuencia.
        RunEvent::AcpSession(_) => {
            sqlx::query(
                "UPDATE runs SET last_sequence = ?, updated_at_ms = ? \
                 WHERE id = ? AND last_sequence = ?",
            )
            .bind(sequence)
            .bind(now)
            .bind(&run.id)
            .bind(run.last_sequence)
            .execute(&mut *conn)
            .await
        }
    };
    let updated = updated?.rows_affected();
    if updated == 0 {
        // Dentro de `BEGIN IMMEDIATE` nadie más escribe: sería un error de programación.
        return Err(CoreError::Conflict {
            current: serde_json::to_value(run)?,
        });
    }
    insert_event(conn, &run.id, sequence, event.kind(), &payload, now).await
}

/// Los totales son acumulados: un valor menor que el ya registrado es un error del emisor.
fn ensure_not_decreasing(current: &RunUsage, reported: &RunUsage) -> Result<(), CoreError> {
    let pairs = [
        (current.input_tokens, reported.input_tokens),
        (current.output_tokens, reported.output_tokens),
        (current.cost_usd_micros, reported.cost_usd_micros),
    ];
    if pairs
        .iter()
        .any(|(before, now)| matches!((before, now), (Some(b), Some(n)) if n < b))
    {
        return Err(CoreError::Validation {
            field: "event",
            reason: "Los totales de uso y coste son acumulados: no pueden disminuir.".into(),
        });
    }
    Ok(())
}

/// Tipo de evento guardado: nombre corto en minúsculas (la base solo limita su longitud).
fn valid_kind(kind: &str) -> bool {
    (1..=40).contains(&kind.len()) && kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

pub(super) async fn insert_event(
    conn: &mut SqliteConnection,
    run_id: &str,
    sequence: i64,
    kind: &str,
    payload: &str,
    now: i64,
) -> Result<(), CoreError> {
    if !valid_kind(kind) {
        return Err(CoreError::Validation {
            field: "event",
            reason: "Tipo de evento no válido.".into(),
        });
    }
    sqlx::query(
        "INSERT INTO run_events (run_id, sequence, kind, payload, at_ms) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(run_id)
    .bind(sequence)
    .bind(kind)
    .bind(payload)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
