//! Peticiones de permiso de un agente durante un intento (HU-020, migración 0014).
//!
//! Recorrido: el supervisor registra la petición (`request_approval`) → el intento pasa a
//! `waiting_approval` → una persona decide una sola vez (`decide_approval`: permitir una vez o
//! rechazar) → el supervisor entrega la decisión al agente (`resolve_approval`), en una
//! transacción que vuelve a comprobar que el intento sigue activo y sin cancelación. Cuando no
//! queda ninguna pendiente, el intento vuelve a `running`.
//!
//! Reglas (decisiones del propietario):
//! - caducidad de [`APPROVAL_TTL_MS`] (10 min): la caducada se entrega como rechazo;
//! - solo «permitir una vez» y «rechazar»: nunca reglas permanentes en el agente;
//! - cancelar el intento (o que termine) cierra las pendientes y las decididas sin entregar
//!   (`cancelled`): una aprobación nunca se aplica después de cancelar;
//! - el agente puede retirar una petición (`withdraw_approval`, `$/cancel_request` en ACP).
//!
//! Título y alcance son datos del agente: se recortan, se quitan controles y formato invisible
//! y se ocultan si tienen forma de secreto. Nada de esto se expone a agentes.

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::event::RunEvent;
use super::run::{RunState, RunSummary, append_event, require_run, require_run_any, validate_id};
use super::supervisor::{SupervisorToken, ensure_token};
use crate::error::CoreError;
use crate::platform::actor::Actor;
use crate::platform::clock::iso_from_ms;
use crate::platform::idempotency::MutationKey;
use crate::platform::storage::Store;
use crate::platform::text::{contains_secret_like, has_invisible_formatting};

/// Tiempo que una petición espera una decisión (decisión del propietario: 10 min).
pub const APPROVAL_TTL_MS: i64 = 10 * 60 * 1000;
/// Peticiones pendientes a la vez por intento; más → el supervisor las rechaza sin preguntar.
pub const MAX_PENDING_APPROVALS: i64 = 8;
/// Peticiones en total por intento (también retiradas o cerradas): más → rechazo sin preguntar.
pub const MAX_APPROVALS_PER_RUN: i64 = 200;
pub const MAX_APPROVAL_TITLE_CHARS: usize = 300;
pub const MAX_APPROVAL_SCOPE_CHARS: usize = 2000;
const MAX_KIND_CHARS: usize = 32;
const HIDDEN_SECRET: &str = "[oculto: parecía contener un secreto]";
const OPERATION: &str = "runs.decide_approval";

/// Estado de una petición de permiso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ApprovalState {
    /// Espera la decisión de una persona.
    Pending,
    /// Una persona la permitió una vez.
    Allowed,
    /// Una persona la rechazó.
    Rejected,
    /// Caducó sin decisión: se entregó al agente como rechazo.
    Expired,
    /// El intento terminó o se pidió cancelarlo antes de entregar la decisión.
    Cancelled,
    /// El agente la retiró.
    Withdrawn,
}

impl ApprovalState {
    const ALL: [Self; 6] = [
        Self::Pending,
        Self::Allowed,
        Self::Rejected,
        Self::Expired,
        Self::Cancelled,
        Self::Withdrawn,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Allowed => "allowed",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
            Self::Withdrawn => "withdrawn",
        }
    }

    fn parse(value: &str) -> Result<Self, CoreError> {
        Self::ALL
            .into_iter()
            .find(|s| s.as_str() == value)
            .ok_or_else(|| {
                CoreError::Database(sqlx::Error::Decode(
                    format!("estado de aprobación desconocido: {value}").into(),
                ))
            })
    }
}

/// Decisión de una persona.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ApprovalDecision {
    /// Permitir esta acción una vez (`allow_once`).
    AllowOnce,
    /// Rechazar esta acción (`reject_once`).
    Reject,
}

/// Petición de permiso tal como se muestra.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunApproval {
    pub id: String,
    pub run_id: String,
    /// «Permiso #n» dentro del intento.
    #[ts(type = "number")]
    pub sequence: i64,
    /// Acción según el agente (p. ej. «Write recetas.py»). Dato no confiable: texto plano.
    pub tool_title: String,
    /// Tipo de herramienta según el agente (`edit`, `execute`…), si lo informó.
    pub tool_kind: Option<String>,
    /// Alcance según el agente: comando o rutas. Dato no confiable: texto plano.
    pub scope: Option<String>,
    /// El agente ofreció «permitir una vez»; si no, solo se puede rechazar.
    pub can_allow: bool,
    pub state: ApprovalState,
    pub requested_at: String,
    pub expires_at: String,
    /// Decisión o cierre.
    pub decided_at: Option<String>,
    /// La decisión salió hacia la sesión del agente (se marca en la misma transacción que
    /// comprueba el intento; si el turno se cancela a la vez, el cliente responde «cancelado»
    /// y la decisión no se aplica).
    pub delivered: bool,
    /// Motivo del cierre, generado por el núcleo.
    pub end_reason: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ApprovalRow {
    id: String,
    run_id: String,
    sequence: i64,
    tool_title: String,
    tool_kind: Option<String>,
    scope: Option<String>,
    can_allow: bool,
    state: String,
    requested_at: String,
    expires_at: String,
    decided_at: Option<String>,
    delivered: bool,
    end_reason: Option<String>,
    expires_at_ms: i64,
}

impl ApprovalRow {
    fn into_approval(self) -> Result<(RunApproval, i64), CoreError> {
        Ok((
            RunApproval {
                state: ApprovalState::parse(&self.state)?,
                id: self.id,
                run_id: self.run_id,
                sequence: self.sequence,
                tool_title: self.tool_title,
                tool_kind: self.tool_kind,
                scope: self.scope,
                can_allow: self.can_allow,
                requested_at: self.requested_at,
                expires_at: self.expires_at,
                decided_at: self.decided_at,
                delivered: self.delivered,
                end_reason: self.end_reason,
            },
            self.expires_at_ms,
        ))
    }
}

macro_rules! select_approval {
    ($tail:literal) => {
        concat!(
            "SELECT p.id, p.run_id, p.sequence, p.tool_title, p.tool_kind, p.scope, \
             p.can_allow, p.state, ",
            iso_from_ms!("p.requested_at_ms"),
            " AS requested_at, ",
            iso_from_ms!("p.expires_at_ms"),
            " AS expires_at, ",
            iso_from_ms!("p.decided_at_ms"),
            " AS decided_at, p.delivered_at_ms IS NOT NULL AS delivered, p.end_reason, \
             p.expires_at_ms FROM run_approvals p ",
            $tail
        )
    };
}

/// Peticiones de un intento (más reciente primero), para su detalle.
pub(super) async fn approvals_of_run(
    conn: &mut SqliteConnection,
    run_id: &str,
) -> Result<Vec<RunApproval>, CoreError> {
    let rows: Vec<ApprovalRow> = sqlx::query_as(select_approval!(
        "WHERE p.run_id = ? ORDER BY p.sequence DESC"
    ))
    .bind(run_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter()
        .map(|row| row.into_approval().map(|(approval, _)| approval))
        .collect()
}

async fn load_approval(
    conn: &mut SqliteConnection,
    approval_id: &str,
) -> Result<(RunApproval, i64), CoreError> {
    let row: Option<ApprovalRow> = sqlx::query_as(select_approval!("WHERE p.id = ?"))
        .bind(approval_id)
        .fetch_optional(&mut *conn)
        .await?;
    row.ok_or(CoreError::NotFound {
        entity: "Petición de permiso",
    })?
    .into_approval()
}

/// Por qué se cierran en bloque las peticiones abiertas de un intento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ApprovalCloseCause {
    /// El intento terminó (cualquier estado terminal).
    RunEnded,
    /// Una persona (o aceptar/archivar la historia) pidió cancelar el intento.
    CancelRequested,
    /// Al abrir los datos: peticiones abiertas de intentos ya terminados (copias restauradas).
    Orphaned,
}

impl ApprovalCloseCause {
    fn reason(self) -> &'static str {
        match self {
            Self::RunEnded => "El intento terminó antes de aplicar una decisión.",
            Self::CancelRequested => "Se pidió cancelar el intento.",
            Self::Orphaned => "El intento ya no está activo.",
        }
    }
}

/// Cierra como `cancelled` las peticiones pendientes y las decididas sin entregar de un intento,
/// en la transacción de quien llama. Sin eventos (el cambio de estado del intento ya lo
/// explica).
pub(super) async fn close_open_approvals(
    conn: &mut SqliteConnection,
    run_id: &str,
    cause: ApprovalCloseCause,
    now: i64,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE run_approvals SET state = 'cancelled', decided_at_ms = COALESCE(decided_at_ms, ?), end_reason = ? \
         WHERE run_id = ? AND (state = 'pending' \
           OR (state IN ('allowed', 'rejected') AND delivered_at_ms IS NULL))",
    )
    .bind(now)
    .bind(cause.reason())
    .bind(run_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Cierra las peticiones abiertas de intentos que ya no están activos (p. ej. de una copia
/// restaurada). Lo llama `interrupt_unverifiable_runs` al abrir los datos.
pub(super) async fn close_orphaned_approvals(
    conn: &mut SqliteConnection,
    now: i64,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE run_approvals SET state = 'cancelled', decided_at_ms = COALESCE(decided_at_ms, ?), end_reason = ? \
         WHERE (state = 'pending' \
           OR (state IN ('allowed', 'rejected') AND delivered_at_ms IS NULL)) \
         AND run_id IN (SELECT id FROM runs \
           WHERE state NOT IN ('running', 'waiting_approval'))",
    )
    .bind(now)
    .bind(ApprovalCloseCause::Orphaned.reason())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn pending_count(conn: &mut SqliteConnection, run_id: &str) -> Result<i64, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM run_approvals WHERE run_id = ? AND state = 'pending'",
    )
    .bind(run_id)
    .fetch_one(&mut *conn)
    .await?)
}

/// `waiting_approval → running` si ya no queda ninguna pendiente.
async fn resume_if_none_pending(
    conn: &mut SqliteConnection,
    run_id: &str,
    now: i64,
) -> Result<(), CoreError> {
    let run = require_run_any(conn, run_id).await?;
    if run.state == RunState::WaitingApproval && pending_count(conn, run_id).await? == 0 {
        append_event(
            conn,
            &run,
            &RunEvent::StateChanged {
                to: RunState::Running,
                reason: None,
            },
            now,
        )
        .await?;
    }
    Ok(())
}

/// Cierra una petición (`pending` o decidida) con evento, en la transacción de quien llama.
async fn resolve_row(
    conn: &mut SqliteConnection,
    approval: &RunApproval,
    to: ApprovalState,
    reason: Option<&str>,
    delivered: bool,
    now: i64,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE run_approvals SET state = ?, decided_at_ms = COALESCE(decided_at_ms, ?), \
         delivered_at_ms = ?, end_reason = COALESCE(?, end_reason) WHERE id = ?",
    )
    .bind(to.as_str())
    .bind(now)
    .bind(delivered.then_some(now))
    .bind(reason)
    .bind(&approval.id)
    .execute(&mut *conn)
    .await?;
    if to != approval.state {
        let run = require_run_any(conn, &approval.run_id).await?;
        append_event(
            conn,
            &run,
            &RunEvent::ApprovalResolved {
                approval_id: approval.id.clone(),
                outcome: to,
            },
            now,
        )
        .await?;
    }
    Ok(())
}

fn run_accepts_decisions(run: &RunSummary) -> bool {
    matches!(run.state, RunState::Running | RunState::WaitingApproval)
        && run.cancel_requested_at.is_none()
}

// ---------------------------------------------------------------------------------------------
// Personas
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DecideApprovalInput {
    /// Repítelo al reintentar la misma decisión.
    pub request_id: String,
    pub project_id: String,
    pub approval_id: String,
    pub decision: ApprovalDecision,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    project_id: &'a str,
    approval_id: &'a str,
    decision: ApprovalDecision,
}

/// Decide una petición de permiso. Solo personas, con `requestId` (reintentar devuelve el
/// resultado original). Una transacción `BEGIN IMMEDIATE`: solo se decide una `pending`,
/// antes de caducar, con el intento en `running`/`waiting_approval` y sin petición de
/// cancelación; si no → `approval_closed` (nada cambia). Permitir sin que el agente lo ofreciera
/// → `validation`. Después, si no queda ninguna pendiente, el intento vuelve a `running`. Quien
/// llama despierta al supervisor para que entregue la decisión.
///
/// Errores: petición o intento de otro proyecto o inexistente → `not_found`.
pub async fn decide_approval(
    store: &Store,
    actor: &Actor,
    input: &DecideApprovalInput,
) -> Result<RunApproval, CoreError> {
    actor.require_human("decidir un permiso de un agente")?;
    validate_id("approvalId", &input.approval_id)?;
    let key = MutationKey::new(actor.receipt_key(), OPERATION, &input.request_id)?;
    let payload = Payload {
        project_id: &input.project_id,
        approval_id: &input.approval_id,
        decision: input.decision,
    };
    let (input, clock) = (input.clone(), store.clone());
    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let (approval, expires_at_ms) = load_approval(conn, &input.approval_id).await?;
                // Pertenece al proyecto (si no, no existe para quien pregunta).
                let run = require_run(conn, &input.project_id, &approval.run_id).await?;
                if approval.state != ApprovalState::Pending {
                    return Err(CoreError::ApprovalClosed {
                        reason: "Esta petición ya no espera respuesta: ya se decidió, caducó, \
                                 la retiró el agente o terminó el intento.",
                    });
                }
                if now >= expires_at_ms {
                    return Err(CoreError::ApprovalClosed {
                        reason: "La petición caducó: el agente recibe un rechazo.",
                    });
                }
                if !run_accepts_decisions(&run) {
                    return Err(CoreError::ApprovalClosed {
                        reason: "El intento ya no está en marcha o se está cancelando: la \
                                 petición no se puede decidir.",
                    });
                }
                let to = match input.decision {
                    ApprovalDecision::AllowOnce if !approval.can_allow => {
                        return Err(CoreError::Validation {
                            field: "decision",
                            reason: "El agente no ofreció «permitir una vez» para esta acción: \
                                     solo se puede rechazar."
                                .into(),
                        });
                    }
                    ApprovalDecision::AllowOnce => ApprovalState::Allowed,
                    ApprovalDecision::Reject => ApprovalState::Rejected,
                };
                let reason = match to {
                    ApprovalState::Allowed => "Permitido una vez por una persona.",
                    _ => "Rechazado por una persona.",
                };
                resolve_row(conn, &approval, to, Some(reason), false, now).await?;
                resume_if_none_pending(conn, &run.id, now).await?;
                Ok(load_approval(conn, &approval.id).await?.0)
            })
        })
        .await
}

/// Intento con peticiones pendientes (bandeja y tablero).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WaitingApproval {
    pub story_id: String,
    #[ts(type = "number")]
    pub story_number: i64,
    pub story_title: String,
    pub run_id: String,
    #[ts(type = "number")]
    pub run_number: i64,
    pub agent_id: String,
    /// Peticiones pendientes sin caducar.
    #[ts(type = "number")]
    pub pending: i64,
    /// La más antigua (para ordenar).
    pub since: String,
}

/// Intentos del proyecto que esperan una decisión: activos, sin petición de cancelación y con
/// alguna petición pendiente sin caducar (lectura). Señal real: sale de `run_approvals`, no
/// de lo que diga el agente.
pub async fn list_waiting_approvals(
    store: &Store,
    project_id: &str,
) -> Result<Vec<WaitingApproval>, CoreError> {
    let now = store.now_ms();
    let mut conn = store.reader().acquire().await?;
    crate::features::projects::ensure_project_exists(&mut conn, project_id).await?;
    Ok(sqlx::query_as(concat!(
        "SELECT r.story_id, s.number AS story_number, s.title AS story_title, r.id AS run_id, r.number AS run_number, r.agent_id, \
         COUNT(*) AS pending, ",
        iso_from_ms!("MIN(p.requested_at_ms)"),
        " AS since FROM run_approvals p JOIN runs r ON r.id = p.run_id \
         JOIN stories s ON s.id = r.story_id WHERE r.project_id = ? AND p.state = 'pending' AND p.expires_at_ms > ? \
         AND r.state IN ('running', 'waiting_approval') AND r.cancel_requested_at_ms IS NULL \
         GROUP BY r.id ORDER BY MIN(p.requested_at_ms)"
    ))
    .bind(project_id)
    .bind(now)
    .fetch_all(&mut *conn)
    .await?)
}

// ---------------------------------------------------------------------------------------------
// Supervisor
// ---------------------------------------------------------------------------------------------

/// Lo que el agente pide (datos no confiables; el núcleo los limpia).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalAsk {
    pub tool_title: String,
    pub tool_kind: Option<String>,
    pub scope: Option<String>,
    pub can_allow: bool,
}

/// Petición registrada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedApproval {
    pub approval: RunApproval,
    pub expires_at_ms: i64,
    /// Número de la historia (para la notificación: «… en #N»).
    pub story_number: i64,
    /// El intento pasó de `running` a `waiting_approval` con esta petición (la notificación
    /// solo se envía entonces, no por cada petición).
    pub started_waiting: bool,
    pub story_id: String,
}

/// Qué hacer con una petición (ver [`resolve_approval`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalResolution {
    /// Sigue esperando hasta `expires_at_ms`.
    Pending { expires_at_ms: i64 },
    /// Responder al agente con esta decisión (ya registrada como entregada).
    Deliver(ApprovalDecision),
    /// Caducó ahora: responder al agente con un rechazo (ya registrado como entregado).
    Expired,
    /// Cerrada (cancelada, retirada o ya entregada): responder `cancelled` si el agente aún
    /// espera; nunca aplicar una decisión.
    Closed,
}

/// Limpia texto del agente: sin controles ni formato invisible, espacios colapsados, recortado.
fn clean(text: &str, max_chars: usize) -> Option<String> {
    let visible: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| !has_invisible_formatting(c.encode_utf8(&mut [0; 4])))
        .collect();
    let collapsed = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if contains_secret_like(&collapsed) {
        return Some(HIDDEN_SECRET.to_owned());
    }
    if collapsed.chars().count() <= max_chars {
        return Some(collapsed);
    }
    let mut cut: String = collapsed.chars().take(max_chars - 1).collect();
    cut.push('…');
    Some(cut)
}

fn clean_kind(kind: Option<&str>) -> Option<String> {
    kind.filter(|k| {
        (1..=MAX_KIND_CHARS).contains(&k.len())
            && k.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
    })
    .map(str::to_owned)
}

/// Registra una petición de permiso del agente, en una transacción `BEGIN IMMEDIATE`:
/// `running → waiting_approval` (o sigue en `waiting_approval`), evento `approval_requested`,
/// caducidad = ahora + [`APPROVAL_TTL_MS`].
///
/// Errores: intento que no está en `running`/`waiting_approval` o con petición de cancelación →
/// `approval_closed` (el supervisor responde `cancelled` al agente); más de
/// [`MAX_PENDING_APPROVALS`] pendientes → `validation` (el supervisor rechaza sin preguntar).
pub async fn request_approval(
    store: &Store,
    token: &SupervisorToken,
    run_id: &str,
    ask: &ApprovalAsk,
) -> Result<RequestedApproval, CoreError> {
    ensure_token(store, token)?;
    validate_id("runId", run_id)?;
    let tool_title = clean(&ask.tool_title, MAX_APPROVAL_TITLE_CHARS)
        .unwrap_or_else(|| "(sin título)".to_owned());
    let tool_kind = clean_kind(ask.tool_kind.as_deref());
    let scope = ask
        .scope
        .as_deref()
        .and_then(|s| clean(s, MAX_APPROVAL_SCOPE_CHARS));
    let (run_id, can_allow, clock) = (run_id.to_owned(), ask.can_allow, store.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let run = require_run_any(conn, &run_id).await?;
                if !run_accepts_decisions(&run) {
                    return Err(CoreError::ApprovalClosed {
                        reason: "El intento ya no está en marcha o se está cancelando.",
                    });
                }
                if pending_count(conn, &run.id).await? >= MAX_PENDING_APPROVALS {
                    return Err(CoreError::Validation {
                        field: "approval",
                        reason: format!(
                            "El agente tiene ya {MAX_PENDING_APPROVALS} peticiones de permiso sin \
                             responder."
                        ),
                    });
                }
                let sequence: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(sequence), 0) + 1 FROM run_approvals WHERE run_id = ?",
                )
                .bind(&run.id)
                .fetch_one(&mut *conn)
                .await?;
                if sequence > MAX_APPROVALS_PER_RUN {
                    return Err(CoreError::Validation {
                        field: "approval",
                        reason: format!(
                            "El agente ya pidió {MAX_APPROVALS_PER_RUN} permisos en este intento."
                        ),
                    });
                }
                let id = uuid::Uuid::now_v7().to_string();
                let expires_at_ms = now + APPROVAL_TTL_MS;
                sqlx::query(
                    "INSERT INTO run_approvals (id, run_id, sequence, tool_title, tool_kind, \
                     scope, can_allow, state, requested_at_ms, expires_at_ms) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)",
                )
                .bind(&id)
                .bind(&run.id)
                .bind(sequence)
                .bind(&tool_title)
                .bind(&tool_kind)
                .bind(&scope)
                .bind(can_allow)
                .bind(now)
                .bind(expires_at_ms)
                .execute(&mut *conn)
                .await?;
                append_event(
                    conn,
                    &run,
                    &RunEvent::ApprovalRequested {
                        approval_id: id.clone(),
                        sequence,
                    },
                    now,
                )
                .await?;
                let started_waiting = run.state == RunState::Running;
                if started_waiting {
                    let run = require_run_any(conn, &run.id).await?;
                    append_event(
                        conn,
                        &run,
                        &RunEvent::StateChanged {
                            to: RunState::WaitingApproval,
                            reason: None,
                        },
                        now,
                    )
                    .await?;
                }
                let story_number: i64 =
                    sqlx::query_scalar("SELECT number FROM stories WHERE id = ?")
                        .bind(&run.story_id)
                        .fetch_one(&mut *conn)
                        .await?;
                let (approval, expires_at_ms) = load_approval(conn, &id).await?;
                Ok(RequestedApproval {
                    approval,
                    expires_at_ms,
                    story_number,
                    started_waiting,
                    story_id: run.story_id,
                })
            })
        })
        .await
}

/// Qué responder al agente sobre una petición, en una transacción `BEGIN IMMEDIATE`:
///
/// - decidida y sin entregar, con el intento activo y sin cancelación → la marca entregada y
///   devuelve `Deliver` (una sola vez: la segunda llamada devuelve `Closed`);
/// - pendiente y vencida → `expired` + entregada, el intento vuelve a `running` si no quedan
///   más, y devuelve `Expired` (responder con rechazo);
/// - pendiente sin vencer → `Pending`;
/// - cualquier otro caso → `Closed`.
pub async fn resolve_approval(
    store: &Store,
    token: &SupervisorToken,
    approval_id: &str,
) -> Result<ApprovalResolution, CoreError> {
    ensure_token(store, token)?;
    validate_id("approvalId", approval_id)?;
    // Primero una lectura: cada `wake` del supervisor llega aquí por cada petición abierta, y
    // casi siempre no hay nada que hacer. Solo se abre la escritura si hay que entregar,
    // caducar o cerrar; dentro se vuelve a comprobar todo.
    {
        let mut conn = store.reader().acquire().await?;
        let (approval, expires_at_ms) = load_approval(&mut conn, approval_id).await?;
        match approval.state {
            ApprovalState::Pending if store.now_ms() < expires_at_ms => {
                return Ok(ApprovalResolution::Pending { expires_at_ms });
            }
            ApprovalState::Pending => {}
            ApprovalState::Allowed | ApprovalState::Rejected if !approval.delivered => {}
            _ => return Ok(ApprovalResolution::Closed),
        }
    }
    let (approval_id, clock) = (approval_id.to_owned(), store.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let (approval, expires_at_ms) = load_approval(conn, &approval_id).await?;
                let run = require_run_any(conn, &approval.run_id).await?;
                match approval.state {
                    ApprovalState::Allowed | ApprovalState::Rejected if !approval.delivered => {
                        if !run_accepts_decisions(&run) {
                            // Defensa: cancelar ya las cierra en su transacción.
                            resolve_row(
                                conn,
                                &approval,
                                ApprovalState::Cancelled,
                                Some(ApprovalCloseCause::CancelRequested.reason()),
                                false,
                                now,
                            )
                            .await?;
                            return Ok(ApprovalResolution::Closed);
                        }
                        let decision = if approval.state == ApprovalState::Allowed {
                            ApprovalDecision::AllowOnce
                        } else {
                            ApprovalDecision::Reject
                        };
                        sqlx::query("UPDATE run_approvals SET delivered_at_ms = ? WHERE id = ?")
                            .bind(now)
                            .bind(&approval.id)
                            .execute(&mut *conn)
                            .await?;
                        Ok(ApprovalResolution::Deliver(decision))
                    }
                    ApprovalState::Pending if now >= expires_at_ms => {
                        resolve_row(
                            conn,
                            &approval,
                            ApprovalState::Expired,
                            Some("Caducó sin respuesta: se responde rechazar al agente."),
                            true,
                            now,
                        )
                        .await?;
                        resume_if_none_pending(conn, &run.id, now).await?;
                        Ok(ApprovalResolution::Expired)
                    }
                    ApprovalState::Pending => Ok(ApprovalResolution::Pending { expires_at_ms }),
                    _ => Ok(ApprovalResolution::Closed),
                }
            })
        })
        .await
}

/// El agente retiró la petición (`$/cancel_request`): `pending` o decidida sin entregar →
/// `withdrawn`; el intento vuelve a `running` si no quedan pendientes. Ya cerrada → nada.
pub async fn withdraw_approval(
    store: &Store,
    token: &SupervisorToken,
    approval_id: &str,
) -> Result<RunApproval, CoreError> {
    close_from_session(
        store,
        token,
        approval_id,
        ApprovalState::Withdrawn,
        "El agente retiró la petición.",
    )
    .await
}

/// El cliente ACP ya respondió `cancelled` al agente por su cuenta (se canceló el turno por
/// tiempo máximo o apagado, o terminó la sesión): `pending` o decidida sin entregar →
/// `cancelled`, para que ninguna decisión posterior conste como entregada.
pub async fn cancel_approval(
    store: &Store,
    token: &SupervisorToken,
    approval_id: &str,
) -> Result<RunApproval, CoreError> {
    close_from_session(
        store,
        token,
        approval_id,
        ApprovalState::Cancelled,
        "La sesión del agente respondió «cancelado» (se canceló el turno o terminó).",
    )
    .await
}

async fn close_from_session(
    store: &Store,
    token: &SupervisorToken,
    approval_id: &str,
    to: ApprovalState,
    reason: &'static str,
) -> Result<RunApproval, CoreError> {
    ensure_token(store, token)?;
    validate_id("approvalId", approval_id)?;
    let (approval_id, clock) = (approval_id.to_owned(), store.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let (approval, _) = load_approval(conn, &approval_id).await?;
                let open = approval.state == ApprovalState::Pending
                    || (matches!(
                        approval.state,
                        ApprovalState::Allowed | ApprovalState::Rejected
                    ) && !approval.delivered);
                if open {
                    resolve_row(conn, &approval, to, Some(reason), false, now).await?;
                    let run = require_run_any(conn, &approval.run_id).await?;
                    if !run.state.is_terminal() {
                        resume_if_none_pending(conn, &run.id, now).await?;
                    }
                }
                Ok(load_approval(conn, &approval.id).await?.0)
            })
        })
        .await
}
