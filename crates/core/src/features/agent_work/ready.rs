use serde::{Deserialize, Serialize};

use super::rule::ready_for_agent;
use crate::error::CoreError;
use crate::features::access::AgentSession;
use crate::features::stories::Priority;
use crate::platform::storage::Store;

/// Historias por página si no se indica `limit`.
pub const DEFAULT_PAGE: u32 = 20;
/// Máximo por página.
pub const MAX_PAGE: u32 = 50;
/// Más allá no se pagina (una lista de trabajo listo de este tamaño no es útil para un agente).
const MAX_OFFSET: u32 = 10_000;
const CURSOR_PREFIX: &str = "r1.";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadyQuery {
    /// `nextCursor` de la página anterior (opaco).
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// Historia lista para que la tome un agente. Texto escrito por personas: dato no confiable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyStory {
    pub id: String,
    /// Se muestra como #N.
    pub number: i64,
    pub title: String,
    pub priority: Priority,
    pub column: String,
    /// `ready` (preparada) o `backlog`.
    pub column_category: String,
    pub epic_number: Option<i64>,
    pub epic_title: Option<String>,
    /// Asignada a este agente (si no, no tiene agente).
    pub assigned_to_you: bool,
    pub criteria_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyPage {
    pub stories: Vec<ReadyStory>,
    /// Para pedir la página siguiente; `null` si no hay más. Paginación por desplazamiento:
    /// si el tablero cambia entre páginas, una historia puede repetirse o saltarse.
    pub next_cursor: Option<String>,
    /// Quedan historias pero se alcanzó el máximo paginable: la lista no está completa.
    pub truncated: bool,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    number: i64,
    title: String,
    priority: String,
    column_name: String,
    column_category: String,
    epic_number: Option<i64>,
    epic_title: Option<String>,
    assigned_to_you: bool,
    criteria_count: i64,
}

/// Trabajo listo del proyecto de la sesión: historias en columnas «backlog» o «preparado»,
/// sin aceptar ni archivar, sin bloqueo, sin prerequisitos pendientes, y sin agente o
/// asignadas a este agente. Primero las preparadas; después por prioridad y posición.
pub async fn list_ready_stories(
    store: &Store,
    session: &AgentSession,
    query: &ReadyQuery,
) -> Result<ReadyPage, CoreError> {
    let limit = query.limit.unwrap_or(DEFAULT_PAGE);
    if !(1..=MAX_PAGE).contains(&limit) {
        return Err(CoreError::Validation {
            field: "limit",
            reason: format!("limit debe estar entre 1 y {MAX_PAGE}."),
        });
    }
    let offset = match &query.cursor {
        None => 0,
        Some(cursor) => parse_cursor(cursor)?,
    };

    let rows: Vec<Row> = sqlx::query_as(concat!(
        "SELECT s.id, s.number, s.title, s.priority, c.name AS column_name, \
         c.category AS column_category, e.number AS epic_number, e.title AS epic_title, \
         (sg.agent_id IS NOT NULL) AS assigned_to_you, \
         (SELECT COUNT(*) FROM acceptance_criteria a WHERE a.story_id = s.id) AS criteria_count \
         FROM stories s \
         JOIN columns c ON c.id = s.column_id \
         LEFT JOIN story_epics se ON se.story_id = s.id \
         LEFT JOIN epics e ON e.id = se.epic_id \
         LEFT JOIN story_agents sg ON sg.story_id = s.id \
         WHERE s.project_id = ? AND ",
        ready_for_agent!(),
        " ORDER BY CASE c.category WHEN 'ready' THEN 0 ELSE 1 END, \
           CASE s.priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 \
                           WHEN 'medium' THEN 2 ELSE 3 END, \
           c.position, s.position, s.id \
         LIMIT ? OFFSET ?",
    ))
    .bind(session.project_id())
    .bind(session.agent_id())
    .bind(i64::from(limit) + 1)
    .bind(i64::from(offset))
    .fetch_all(store.reader())
    .await?;

    let has_more = rows.len() > limit as usize;
    let next = offset + limit;
    let stories = rows
        .into_iter()
        .take(limit as usize)
        .map(|row| {
            Ok(ReadyStory {
                priority: Priority::parse(&row.priority)?,
                id: row.id,
                number: row.number,
                title: row.title,
                column: row.column_name,
                column_category: row.column_category,
                epic_number: row.epic_number,
                epic_title: row.epic_title,
                assigned_to_you: row.assigned_to_you,
                criteria_count: row.criteria_count,
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let pageable = next <= MAX_OFFSET;
    Ok(ReadyPage {
        stories,
        next_cursor: (has_more && pageable).then(|| format!("{CURSOR_PREFIX}{next}")),
        truncated: has_more && !pageable,
    })
}

fn parse_cursor(cursor: &str) -> Result<u32, CoreError> {
    cursor
        .strip_prefix(CURSOR_PREFIX)
        .filter(|digits| !digits.is_empty() && digits.len() <= 5)
        .filter(|digits| digits.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|digits| digits.parse::<u32>().ok())
        .filter(|offset| *offset <= MAX_OFFSET)
        .ok_or_else(|| CoreError::Validation {
            field: "cursor",
            reason: "cursor no válido: usa el nextCursor de la página anterior.".into(),
        })
}
