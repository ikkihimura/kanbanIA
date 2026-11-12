use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

use super::story_claim::{CLAIM_DURATION_MS, MAX_OPEN_CLAIMS, StoryClaim, load_claim, validate_id};
use crate::error::CoreError;
use crate::features::access::AgentSession;
use crate::features::agent_work::ready_for_agent;
use crate::platform::storage::Store;

const OPERATION: &str = "claims.claim";

/// Historia a reclamar: por `storyId` o por `number` (#N), no ambos.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimStoryInput {
    /// Repítelo al reintentar la misma reclamación.
    pub request_id: String,
    pub story_id: Option<String>,
    pub number: Option<i64>,
}

/// Lo que identifica la operación para la idempotencia (sin el `requestId`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    story_id: Option<&'a str>,
    number: Option<i64>,
}

enum Target {
    Id(String),
    Number(i64),
}

/// Reclama una historia del proyecto de la sesión. En una sola transacción: cierra reclamos
/// vencidos, comprueba que nadie más la tenga, evalúa la regla de «lista», crea el reclamo
/// y asigna la historia al agente si no tenía a nadie.
///
/// Si el agente ya tiene un reclamo vigente sobre la historia, lo devuelve sin cambiarlo (sin
/// volver a evaluar la regla; `renew_claim` sí rechaza historias archivadas o aceptadas).
pub async fn claim_story(
    store: &Store,
    session: &AgentSession,
    input: &ClaimStoryInput,
) -> Result<StoryClaim, CoreError> {
    let target = match (&input.story_id, input.number) {
        (Some(id), None) => {
            validate_id("storyId", id)?;
            Target::Id(id.clone())
        }
        (None, Some(number)) if number >= 1 => Target::Number(number),
        (None, Some(_)) => {
            return Err(CoreError::Validation {
                field: "number",
                reason: "number debe ser 1 o mayor.".into(),
            });
        }
        _ => {
            return Err(CoreError::Validation {
                field: "storyId",
                reason: "Indica storyId o number (solo uno de los dos).".into(),
            });
        }
    };
    let key = session.receipt_key(OPERATION, &input.request_id)?;
    let payload = Payload {
        story_id: input.story_id.as_deref(),
        number: input.number,
    };
    let (session, clock) = (session.clone(), store.clone());
    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                // Tras `BEGIN IMMEDIATE`: la hora no se queda atrás esperando el bloqueo.
                let now = clock.now_ms();
                claim_in_tx(conn, &session, &target, now).await
            })
        })
        .await
}

async fn claim_in_tx(
    conn: &mut SqliteConnection,
    session: &AgentSession,
    target: &Target,
    now: i64,
) -> Result<StoryClaim, CoreError> {
    let story_id: Option<String> = match target {
        Target::Id(id) => {
            sqlx::query_scalar("SELECT id FROM stories WHERE project_id = ? AND id = ?")
                .bind(session.project_id())
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        Target::Number(number) => {
            sqlx::query_scalar("SELECT id FROM stories WHERE project_id = ? AND number = ?")
                .bind(session.project_id())
                .bind(number)
                .fetch_optional(&mut *conn)
                .await?
        }
    };
    // Otro proyecto o inexistente: sin distinguir.
    let story_id = story_id.ok_or(CoreError::NotFound { entity: "Historia" })?;

    close_expired(conn, &story_id, now).await?;

    let open: Option<(String, String)> = sqlx::query_as(
        "SELECT id, agent_id FROM story_claims WHERE story_id = ? AND ended_at_ms IS NULL",
    )
    .bind(&story_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((claim_id, agent_id)) = open {
        return if agent_id == session.agent_id() {
            load_claim(conn, &claim_id).await
        } else {
            Err(CoreError::Claimed)
        };
    }

    let ready: bool = sqlx::query_scalar(concat!(
        "SELECT ",
        ready_for_agent!(),
        " FROM stories s JOIN columns c ON c.id = s.column_id WHERE s.id = ?"
    ))
    .bind(session.agent_id())
    .bind(now)
    .bind(session.agent_id())
    .bind(&story_id)
    .fetch_one(&mut *conn)
    .await?;
    if !ready {
        return Err(CoreError::NotReady);
    }
    let open: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM story_claims k JOIN stories s ON s.id = k.story_id \
         WHERE k.project_id = ? AND k.agent_id = ? AND k.ended_at_ms IS NULL \
         AND k.expires_at_ms > ? AND s.archived_at IS NULL AND s.accepted_at IS NULL",
    )
    .bind(session.project_id())
    .bind(session.agent_id())
    .bind(now)
    .fetch_one(&mut *conn)
    .await?;
    if open >= MAX_OPEN_CLAIMS {
        return Err(CoreError::ClaimLimit {
            limit: MAX_OPEN_CLAIMS,
        });
    }

    let claim_id = uuid::Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO story_claims \
         (id, project_id, story_id, agent_id, credential_id, claimed_at_ms, expires_at_ms) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&claim_id)
    .bind(session.project_id())
    .bind(&story_id)
    .bind(session.agent_id())
    .bind(session.credential_id())
    .bind(now)
    .bind(now + CLAIM_DURATION_MS)
    .execute(&mut *conn)
    .await?;
    // La regla ya garantiza que no está asignada a otro agente.
    sqlx::query(
        "INSERT INTO story_agents (story_id, agent_id) VALUES (?, ?) \
         ON CONFLICT (story_id) DO NOTHING",
    )
    .bind(&story_id)
    .bind(session.agent_id())
    .execute(&mut *conn)
    .await?;

    load_claim(conn, &claim_id).await
}

/// Cierra como vencido el reclamo abierto de la historia cuyo plazo ya pasó.
async fn close_expired(
    conn: &mut SqliteConnection,
    story_id: &str,
    now: i64,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE story_claims SET ended_at_ms = expires_at_ms, end_reason = 'expired' \
         WHERE story_id = ? AND ended_at_ms IS NULL AND expires_at_ms <= ?",
    )
    .bind(story_id)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
