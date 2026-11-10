//! Lecturas y comprobaciones del grafo de relaciones. Todas filtran por proyecto.

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use crate::error::CoreError;

/// Estado de una historia relacionada, para decidir si bloquea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RelationState {
    /// Aceptada (HU-009): satisface a quien depende de ella.
    Accepted,
    /// Sin aceptar: bloquea a quien depende de ella.
    Pending,
    /// Archivada sin aceptar: sigue sin satisfacer la dependencia (conviene quitarla).
    Archived,
}

/// Referencia ligera a una historia relacionada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoryRef {
    pub id: String,
    #[ts(type = "number")]
    pub number: i64,
    pub title: String,
    pub state: RelationState,
    /// Archivada, independientemente de si está aceptada (`state` prioriza la aceptación).
    pub archived: bool,
}

#[derive(sqlx::FromRow)]
struct RefRow {
    id: String,
    number: i64,
    title: String,
    accepted: bool,
    archived: bool,
}

impl From<RefRow> for StoryRef {
    fn from(row: RefRow) -> Self {
        let state = if row.accepted {
            RelationState::Accepted
        } else if row.archived {
            RelationState::Archived
        } else {
            RelationState::Pending
        };
        Self {
            id: row.id,
            number: row.number,
            title: row.title,
            state,
            archived: row.archived,
        }
    }
}

macro_rules! select_ref {
    ($tail:literal) => {
        concat!(
            "SELECT s.id, s.number, s.title, s.accepted_at IS NOT NULL AS accepted, \
             s.archived_at IS NOT NULL AS archived FROM stories s ",
            $tail
        )
    };
}

pub(super) async fn story_ref(
    conn: &mut SqliteConnection,
    project_id: &str,
    id: &str,
) -> Result<Option<StoryRef>, CoreError> {
    let row: Option<RefRow> = sqlx::query_as(select_ref!("WHERE s.project_id = ? AND s.id = ?"))
        .bind(project_id)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row.map(StoryRef::from))
}

/// Historia existente en el proyecto (NotFound si no, también si es de otro proyecto).
pub(super) async fn require(
    conn: &mut SqliteConnection,
    project_id: &str,
    id: &str,
) -> Result<StoryRef, CoreError> {
    story_ref(conn, project_id, id)
        .await?
        .ok_or(CoreError::NotFound { entity: "Historia" })
}

pub(super) fn require_not_archived(story: &StoryRef, field: &'static str) -> Result<(), CoreError> {
    // Se mira `archived`, no `state`: una historia aceptada y archivada también está archivada.
    if story.archived {
        return Err(CoreError::Validation {
            field,
            reason: format!("La historia #{} está archivada.", story.number),
        });
    }
    Ok(())
}

pub(super) async fn children(
    conn: &mut SqliteConnection,
    project_id: &str,
    parent_id: &str,
) -> Result<Vec<StoryRef>, CoreError> {
    let rows: Vec<RefRow> = sqlx::query_as(select_ref!(
        "JOIN story_parents p ON p.story_id = s.id \
         WHERE p.project_id = ? AND p.parent_id = ? ORDER BY s.number"
    ))
    .bind(project_id)
    .bind(parent_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(StoryRef::from).collect())
}

pub(super) async fn prerequisites(
    conn: &mut SqliteConnection,
    project_id: &str,
    story_id: &str,
) -> Result<Vec<StoryRef>, CoreError> {
    let rows: Vec<RefRow> = sqlx::query_as(select_ref!(
        "JOIN story_dependencies d ON d.depends_on_id = s.id \
         WHERE d.project_id = ? AND d.story_id = ? ORDER BY s.number"
    ))
    .bind(project_id)
    .bind(story_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(StoryRef::from).collect())
}

pub(super) async fn dependents(
    conn: &mut SqliteConnection,
    project_id: &str,
    story_id: &str,
) -> Result<Vec<StoryRef>, CoreError> {
    let rows: Vec<RefRow> = sqlx::query_as(select_ref!(
        "JOIN story_dependencies d ON d.story_id = s.id \
         WHERE d.project_id = ? AND d.depends_on_id = ? ORDER BY s.number"
    ))
    .bind(project_id)
    .bind(story_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(StoryRef::from).collect())
}

/// `true` si `target` es `start` o un antecesor suyo (siguiendo padres).
/// `UNION` (no `UNION ALL`) garantiza que termina aunque hubiera datos cíclicos.
pub(crate) async fn is_self_or_ancestor(
    conn: &mut SqliteConnection,
    start: &str,
    target: &str,
) -> Result<bool, CoreError> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE chain(id) AS ( \
             SELECT ? \
             UNION \
             SELECT p.parent_id FROM story_parents p JOIN chain ON p.story_id = chain.id \
         ) SELECT EXISTS (SELECT 1 FROM chain WHERE id = ?)",
    )
    .bind(start)
    .bind(target)
    .fetch_one(&mut *conn)
    .await?)
}

/// `true` si `target` es alcanzable desde `start` siguiendo «depende de» (incluido `start`).
pub(crate) async fn reaches_through_dependencies(
    conn: &mut SqliteConnection,
    start: &str,
    target: &str,
) -> Result<bool, CoreError> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE reach(id) AS ( \
             SELECT ? \
             UNION \
             SELECT d.depends_on_id FROM story_dependencies d JOIN reach ON d.story_id = reach.id \
         ) SELECT EXISTS (SELECT 1 FROM reach WHERE id = ?)",
    )
    .bind(start)
    .bind(target)
    .fetch_one(&mut *conn)
    .await?)
}
