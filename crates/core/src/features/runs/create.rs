use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::ensure_runnable;
use super::event::RunEvent;
use super::git_head::read_heads;
use super::run::{RunState, RunSummary, active_run_number, insert_event, require_run};
use super::snapshot::{SnapshotSources, build_snapshot};
use crate::error::CoreError;
use crate::features::projects::ensure_project_exists;
use crate::features::stories::{load_detail, load_story};
use crate::features::team::load_agent;
use crate::platform::actor::Actor;
use crate::platform::idempotency::MutationKey;
use crate::platform::storage::Store;
use crate::platform::text::{contains_secret_like, has_invisible_formatting};

const OPERATION: &str = "runs.create";
/// Longitud máxima de la nota de la persona, en caracteres.
pub const MAX_NOTE_CHARS: usize = 4_000;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateRunInput {
    /// Repítelo al reintentar el mismo envío.
    pub request_id: String,
    pub project_id: String,
    pub story_id: String,
    /// Agente del intento; por defecto, el asignado a la historia.
    #[ts(optional)]
    pub agent_id: Option<String>,
    /// Nota opcional para el agente (Markdown, hasta 4 000 caracteres), añadida a las
    /// instrucciones generadas.
    #[ts(optional)]
    pub note: Option<String>,
}

/// Lo que identifica la operación para la idempotencia (sin el `requestId`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    project_id: &'a str,
    story_id: &'a str,
    agent_id: Option<&'a str>,
    note: Option<&'a str>,
}

/// Prepara un intento: crea el Run en `queued` con su instantánea inmutable y su primer
/// evento, en una transacción. Solo personas. Sin ejecutor (HU-018), se queda en cola hasta
/// cancelarse.
///
/// Se rechaza si no hay agente (ni indicado ni asignado) o no existe, si la historia no cumple
/// `ensure_runnable` (archivada, aceptada, bloqueada, prerequisitos o entrega pendientes,
/// reclamo vigente de otro agente) o si ya tiene un intento activo (`run_active`).
pub async fn create_run(
    store: &Store,
    actor: &Actor,
    input: &CreateRunInput,
) -> Result<RunSummary, CoreError> {
    actor.require_human("preparar un intento")?;
    let key = MutationKey::new(actor.receipt_key(), OPERATION, &input.request_id)?;
    let note = normalize_note(input.note.as_deref())?;
    let payload = Payload {
        project_id: &input.project_id,
        story_id: &input.story_id,
        agent_id: input.agent_id.as_deref(),
        note: note.as_deref(),
    };
    // Un reintento devuelve el resultado original sin volver a leer el disco.
    if let Some(previous) = store.replay(&key, &payload).await? {
        return Ok(previous);
    }

    // HEAD de los repositorios: disco, antes de la transacción y fuera del runtime.
    let links: Vec<(String, String)> = {
        let mut conn = store.reader().acquire().await?;
        ensure_project_exists(&mut conn, &input.project_id).await?;
        sqlx::query_as("SELECT id, path FROM repository_links WHERE project_id = ?")
            .bind(&input.project_id)
            .fetch_all(&mut *conn)
            .await?
    };
    let paths: HashMap<String, String> = links.iter().cloned().collect();
    let heads: HashMap<_, _> = read_heads(links)
        .await
        .into_iter()
        .filter_map(|(id, head)| paths.get(&id).map(|path| (id, (path.clone(), head))))
        .collect();

    let (input, clock, frozen_note) = (input.clone(), store.clone(), note.clone());
    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let story = load_story(conn, &input.project_id, &input.story_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Historia" })?;
                let agent_id = input
                    .agent_id
                    .clone()
                    .or_else(|| story.agent_id.clone())
                    .ok_or_else(|| CoreError::Validation {
                        field: "agentId",
                        reason: "Asigna un agente a la historia o elige uno para el intento."
                            .into(),
                    })?;
                let agent = load_agent(conn, &agent_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Agente" })?;
                ensure_runnable(conn, &story, &agent.id, now).await?;
                if let Some(number) = active_run_number(conn, &story.id).await? {
                    return Err(CoreError::RunActive { number });
                }

                let detail = load_detail(conn, &input.project_id, &story.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Historia" })?;
                let snapshot = build_snapshot(
                    conn,
                    &SnapshotSources {
                        project_id: &input.project_id,
                        story: &detail,
                        agent: &agent,
                        note: frozen_note.as_deref(),
                        heads: &heads,
                        now_ms: now,
                    },
                )
                .await?;
                let frozen = snapshot.freeze()?;

                let number: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(number), 0) + 1 FROM runs WHERE story_id = ?",
                )
                .bind(&story.id)
                .fetch_one(&mut *conn)
                .await?;
                let id = uuid::Uuid::now_v7().to_string();
                sqlx::query(
                    "INSERT INTO runs (id, project_id, story_id, number, agent_id, state, \
                     snapshot, snapshot_sha256, last_sequence, created_at_ms, updated_at_ms) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
                )
                .bind(&id)
                .bind(&input.project_id)
                .bind(&story.id)
                .bind(number)
                .bind(&agent.id)
                .bind(RunState::Queued.as_str())
                .bind(&frozen.json)
                .bind(&frozen.sha256)
                .bind(now)
                .bind(now)
                .execute(&mut *conn)
                .await?;
                let first = RunEvent::StateChanged {
                    to: RunState::Queued,
                    reason: None,
                };
                insert_event(conn, &id, 1, first.kind(), &first.to_payload()?, now).await?;

                require_run(conn, &input.project_id, &id).await
            })
        })
        .await
}

/// Nota de la persona: recortada, finales de línea `\n`, sin controles salvo saltos y
/// tabuladores, sin formato invisible y sin nada con forma de secreto. Vacía → `None`.
fn normalize_note(raw: Option<&str>) -> Result<Option<String>, CoreError> {
    let Some(note) = raw.map(|n| n.replace("\r\n", "\n")) else {
        return Ok(None);
    };
    let note = note.trim();
    if note.is_empty() {
        return Ok(None);
    }
    let fail = |reason: &str| {
        Err(CoreError::Validation {
            field: "note",
            reason: reason.to_owned(),
        })
    };
    if note.chars().count() > MAX_NOTE_CHARS {
        return fail("La nota admite como máximo 4 000 caracteres.");
    }
    if note
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
        || has_invisible_formatting(note)
    {
        return fail("La nota contiene caracteres de control o de formato invisibles.");
    }
    if contains_secret_like(note) {
        return fail(
            "La nota contiene algo con forma de secreto (token, clave de API o clave privada). \
             Quítalo antes de preparar el intento.",
        );
    }
    Ok(Some(note.to_owned()))
}
