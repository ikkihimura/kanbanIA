use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use crate::error::CoreError;
use crate::features::access::AgentSession;
use crate::features::team::ClaimOwner;
use crate::platform::storage::Store;

/// Entradas que se devuelven a la UI (las más recientes).
const MAX_ENTRIES: i64 = 50;
/// Filas que se conservan por historia: un agente que alterna su bloqueo no hace crecer la
/// tabla sin límite.
const MAX_STORED_PER_STORY: i64 = 200;

/// Qué reportó el agente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ActivityKind {
    Claimed,
    /// Solo se guarda la última renovación de cada agente en la historia.
    Renewed,
    BlockerReported,
    BlockerCleared,
    SubtaskCreated,
    Delivered,
}

impl ActivityKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::Renewed => "renewed",
            Self::BlockerReported => "blocker_reported",
            Self::BlockerCleared => "blocker_cleared",
            Self::SubtaskCreated => "subtask_created",
            Self::Delivered => "delivered",
        }
    }

    fn parse(value: &str) -> Result<Self, CoreError> {
        Ok(match value {
            "claimed" => Self::Claimed,
            "renewed" => Self::Renewed,
            "blocker_reported" => Self::BlockerReported,
            "blocker_cleared" => Self::BlockerCleared,
            "subtask_created" => Self::SubtaskCreated,
            "delivered" => Self::Delivered,
            other => {
                return Err(CoreError::Database(sqlx::Error::Decode(
                    format!("tipo de actividad desconocido: {other}").into(),
                )));
            }
        })
    }
}

/// Una acción reportada por un agente. Es su afirmación, no algo observado por la app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ActivityEntry {
    pub kind: ActivityKind,
    pub agent: ClaimOwner,
    /// Referencia corta generada por la app («#12», «E2»), nunca texto del agente.
    pub detail: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoryActivity {
    /// De la más reciente a la más antigua (como mucho 50).
    pub entries: Vec<ActivityEntry>,
    /// Última interacción reportada por cualquier agente en la historia. No indica que el
    /// agente siga trabajando.
    pub last_reported_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoryActivityInput {
    pub project_id: String,
    pub story_id: String,
}

/// Registra una acción del agente de la sesión, dentro de la transacción de la operación (un
/// reintento idempotente no la repite). De las renovaciones solo se conserva la última, y de
/// cada historia, las `MAX_STORED_PER_STORY` más recientes.
pub(crate) async fn record_activity(
    conn: &mut SqliteConnection,
    session: &AgentSession,
    story_id: &str,
    kind: ActivityKind,
    detail: Option<&str>,
    now: i64,
) -> Result<(), CoreError> {
    if kind == ActivityKind::Renewed {
        sqlx::query(
            "DELETE FROM agent_activity WHERE story_id = ? AND agent_id = ? AND kind = 'renewed'",
        )
        .bind(story_id)
        .bind(session.agent_id())
        .execute(&mut *conn)
        .await?;
    }
    sqlx::query(
        "INSERT INTO agent_activity (id, project_id, story_id, agent_id, kind, detail, at_ms) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(session.project_id())
    .bind(story_id)
    .bind(session.agent_id())
    .bind(kind.as_str())
    .bind(detail)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM agent_activity WHERE story_id = ? AND id NOT IN \
         (SELECT id FROM agent_activity WHERE story_id = ? ORDER BY at_ms DESC, id DESC LIMIT ?)",
    )
    .bind(story_id)
    .bind(story_id)
    .bind(MAX_STORED_PER_STORY)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Row {
    kind: String,
    agent_id: String,
    agent_number: i64,
    agent_name: String,
    detail: Option<String>,
    at: String,
}

/// Actividad reportada por agentes en una historia (para la app).
pub async fn list_story_activity(
    store: &Store,
    input: &StoryActivityInput,
) -> Result<StoryActivity, CoreError> {
    let mut conn = store.reader().acquire().await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM stories WHERE project_id = ? AND id = ?)")
            .bind(&input.project_id)
            .bind(&input.story_id)
            .fetch_one(&mut *conn)
            .await?;
    if !exists {
        return Err(CoreError::NotFound { entity: "Historia" });
    }
    let rows: Vec<Row> = sqlx::query_as(concat!(
        "SELECT v.kind, a.id AS agent_id, a.number AS agent_number, a.name AS agent_name, \
         v.detail, ",
        crate::platform::clock::iso_from_ms!("v.at_ms"),
        " AS at FROM agent_activity v JOIN agents a ON a.id = v.agent_id \
         WHERE v.project_id = ? AND v.story_id = ? \
         ORDER BY v.at_ms DESC, v.id DESC LIMIT ?"
    ))
    .bind(&input.project_id)
    .bind(&input.story_id)
    .bind(MAX_ENTRIES)
    .fetch_all(&mut *conn)
    .await?;
    let entries = rows
        .into_iter()
        .map(|row| {
            Ok(ActivityEntry {
                kind: ActivityKind::parse(&row.kind)?,
                agent: ClaimOwner {
                    agent_id: row.agent_id,
                    agent_number: row.agent_number,
                    agent_name: row.agent_name,
                },
                detail: row.detail,
                at: row.at,
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    Ok(StoryActivity {
        last_reported_at: entries.first().map(|e| e.at.clone()),
        entries,
    })
}
