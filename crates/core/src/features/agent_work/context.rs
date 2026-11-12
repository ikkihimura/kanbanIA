use serde::Serialize;
use sqlx::SqliteConnection;

use super::rule::ready_for_agent;
use crate::error::CoreError;
use crate::features::access::AgentSession;
use crate::features::stories::{AcceptanceCriterion, Priority};
use crate::platform::storage::Store;

const ENTITY: &str = "Historia";
/// Subtareas que se devuelven como mucho (las dependencias ya están limitadas a 50).
const MAX_SUBTASKS: i64 = 50;

/// Cómo nombra un agente una historia: por id o por su número `#N` en el proyecto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoryKey {
    Id(String),
    Number(i64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextColumn {
    pub name: String,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextEpic {
    pub number: i64,
    pub title: String,
    pub description: String,
}

/// Otra historia relacionada (padre, subtarea o prerequisito).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ContextRef {
    pub id: String,
    pub number: i64,
    pub title: String,
    pub accepted: bool,
    pub archived: bool,
}

/// Contexto de una historia para un agente. Todo el texto es dato no confiable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryContext {
    pub id: String,
    pub number: i64,
    pub title: String,
    /// Markdown tal como se escribió (como mucho 20 000 caracteres).
    pub description: String,
    pub priority: Priority,
    pub column: ContextColumn,
    pub accepted_at: Option<String>,
    pub archived_at: Option<String>,
    pub blocker_reason: Option<String>,
    /// `you` (este agente), `other` (otro agente) o `none`.
    pub assignment: String,
    /// La historia aparecería en `list_ready_stories` para este agente.
    pub ready: bool,
    pub criteria: Vec<AcceptanceCriterion>,
    pub epic: Option<ContextEpic>,
    pub parent: Option<ContextRef>,
    pub subtasks: Vec<ContextRef>,
    /// Hay más subtareas de las devueltas.
    pub subtasks_truncated: bool,
    /// Historias de las que depende; pendiente = sin aceptar.
    pub prerequisites: Vec<ContextRef>,
    pub revision: i64,
    pub updated_at: String,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    number: i64,
    title: String,
    description: String,
    priority: String,
    column_name: String,
    column_category: String,
    accepted_at: Option<String>,
    archived_at: Option<String>,
    blocker_reason: Option<String>,
    agent_id: Option<String>,
    epic_number: Option<i64>,
    epic_title: Option<String>,
    epic_description: Option<String>,
    ready: bool,
    revision: i64,
    updated_at: String,
}

macro_rules! select_context {
    ($tail:literal) => {
        concat!(
            "SELECT s.id, s.number, s.title, s.description, s.priority, \
             c.name AS column_name, c.category AS column_category, s.accepted_at, \
             s.archived_at, b.reason AS blocker_reason, sg.agent_id, \
             e.number AS epic_number, e.title AS epic_title, \
             e.description AS epic_description, ",
            ready_for_agent!(),
            " AS ready, s.revision, s.updated_at \
             FROM stories s \
             JOIN columns c ON c.id = s.column_id \
             LEFT JOIN story_blockers b ON b.story_id = s.id \
             LEFT JOIN story_agents sg ON sg.story_id = s.id \
             LEFT JOIN story_epics se ON se.story_id = s.id \
             LEFT JOIN epics e ON e.id = se.epic_id \
             WHERE s.project_id = ? ",
            $tail
        )
    };
}

macro_rules! select_ref {
    ($tail:literal) => {
        concat!(
            "SELECT r.id, r.number, r.title, (r.accepted_at IS NOT NULL) AS accepted, \
             (r.archived_at IS NOT NULL) AS archived FROM stories r ",
            $tail
        )
    };
}

/// Historia del proyecto de la sesión con criterios, épica y relaciones. Otra historia
/// (de otro proyecto o inexistente) → «no encontrada», sin distinguir. Se lee en una sola
/// transacción de lectura: historia y relaciones son coherentes entre sí.
pub async fn get_story_context(
    store: &Store,
    session: &AgentSession,
    key: &StoryKey,
) -> Result<StoryContext, CoreError> {
    let mut tx = store.reader().begin().await?;
    let context = load(&mut tx, session, key).await;
    tx.rollback().await.ok();
    context
}

async fn load(
    conn: &mut SqliteConnection,
    session: &AgentSession,
    key: &StoryKey,
) -> Result<StoryContext, CoreError> {
    let project = session.project_id();
    let row: Option<Row> = match key {
        StoryKey::Id(id) => {
            sqlx::query_as(select_context!("AND s.id = ?"))
                .bind(session.agent_id())
                .bind(project)
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        StoryKey::Number(number) => {
            sqlx::query_as(select_context!("AND s.number = ?"))
                .bind(session.agent_id())
                .bind(project)
                .bind(number)
                .fetch_optional(&mut *conn)
                .await?
        }
    };
    let row = row.ok_or(CoreError::NotFound { entity: ENTITY })?;

    let criteria: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT id, number, text FROM acceptance_criteria WHERE story_id = ? ORDER BY position",
    )
    .bind(&row.id)
    .fetch_all(&mut *conn)
    .await?;
    let parent: Option<ContextRef> = sqlx::query_as(select_ref!(
        "JOIN story_parents sp ON sp.parent_id = r.id WHERE sp.story_id = ?"
    ))
    .bind(&row.id)
    .fetch_optional(&mut *conn)
    .await?;
    let mut subtasks: Vec<ContextRef> = sqlx::query_as(select_ref!(
        "JOIN story_parents sp ON sp.story_id = r.id WHERE sp.parent_id = ? \
         ORDER BY r.number LIMIT ?"
    ))
    .bind(&row.id)
    .bind(MAX_SUBTASKS + 1)
    .fetch_all(&mut *conn)
    .await?;
    let subtasks_truncated = subtasks.len() as i64 > MAX_SUBTASKS;
    subtasks.truncate(MAX_SUBTASKS as usize);
    // Las dependencias ya están limitadas a 50 al escribirlas; el límite es defensivo.
    let prerequisites: Vec<ContextRef> = sqlx::query_as(select_ref!(
        "JOIN story_dependencies d ON d.depends_on_id = r.id WHERE d.story_id = ? \
         ORDER BY r.number LIMIT ?"
    ))
    .bind(&row.id)
    .bind(MAX_SUBTASKS)
    .fetch_all(&mut *conn)
    .await?;

    let assignment = match row.agent_id.as_deref() {
        None => "none",
        Some(agent) if agent == session.agent_id() => "you",
        Some(_) => "other",
    };

    Ok(StoryContext {
        priority: Priority::parse(&row.priority)?,
        id: row.id,
        number: row.number,
        title: row.title,
        description: row.description,
        column: ContextColumn {
            name: row.column_name,
            category: row.column_category,
        },
        accepted_at: row.accepted_at,
        archived_at: row.archived_at,
        blocker_reason: row.blocker_reason,
        assignment: assignment.into(),
        // Misma regla SQL que `list_ready_stories` (`rule.rs`).
        ready: row.ready,
        criteria: criteria
            .into_iter()
            .map(|(id, number, text)| AcceptanceCriterion { id, number, text })
            .collect(),
        epic: match (row.epic_number, row.epic_title) {
            (Some(number), Some(title)) => Some(ContextEpic {
                number,
                title,
                description: row.epic_description.unwrap_or_default(),
            }),
            _ => None,
        },
        parent,
        subtasks,
        subtasks_truncated,
        prerequisites,
        revision: row.revision,
        updated_at: row.updated_at,
    })
}
