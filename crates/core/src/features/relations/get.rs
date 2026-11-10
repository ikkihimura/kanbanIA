use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::graph::{
    RelationState, StoryRef, children, dependents, prerequisites, require, story_ref,
};
use crate::error::CoreError;
use crate::features::team::{AvatarSpec, load_agent};
use crate::platform::storage::Store;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EpicRef {
    pub id: String,
    #[ts(type = "number")]
    pub number: i64,
    pub title: String,
}

/// Agente asignado (identidad, sin estado de actividad: asignar no implica trabajo en curso).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentRef {
    pub id: String,
    #[ts(type = "number")]
    pub number: i64,
    pub name: String,
    pub role: String,
    pub avatar: AvatarSpec,
}

/// Relaciones de una historia, para el panel del editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoryRelations {
    pub story_id: String,
    pub epic: Option<EpicRef>,
    pub agent: Option<AgentRef>,
    pub parent: Option<StoryRef>,
    pub children: Vec<StoryRef>,
    /// De las que depende esta historia.
    pub prerequisites: Vec<StoryRef>,
    /// Las que dependen de esta historia.
    pub dependents: Vec<StoryRef>,
    /// Prerequisitos sin aceptar (pendientes o archivados): la historia está bloqueada si > 0.
    #[ts(type = "number")]
    pub pending_prerequisites: i64,
}

pub async fn get_story_relations(
    store: &Store,
    project_id: &str,
    story_id: &str,
) -> Result<StoryRelations, CoreError> {
    let mut conn = store.reader().acquire().await?;
    load_relations(&mut conn, project_id, story_id).await
}

pub(super) async fn load_relations(
    conn: &mut SqliteConnection,
    project_id: &str,
    story_id: &str,
) -> Result<StoryRelations, CoreError> {
    require(conn, project_id, story_id).await?;

    let epic: Option<(String, i64, String)> = sqlx::query_as(
        "SELECT e.id, e.number, e.title FROM story_epics se \
         JOIN epics e ON e.id = se.epic_id \
         WHERE se.project_id = ? AND se.story_id = ?",
    )
    .bind(project_id)
    .bind(story_id)
    .fetch_optional(&mut *conn)
    .await?;

    let parent_id: Option<String> = sqlx::query_scalar(
        "SELECT parent_id FROM story_parents WHERE project_id = ? AND story_id = ?",
    )
    .bind(project_id)
    .bind(story_id)
    .fetch_optional(&mut *conn)
    .await?;
    let parent = match parent_id {
        Some(id) => story_ref(conn, project_id, &id).await?,
        None => None,
    };

    let agent_id: Option<String> =
        sqlx::query_scalar("SELECT agent_id FROM story_agents WHERE story_id = ?")
            .bind(story_id)
            .fetch_optional(&mut *conn)
            .await?;
    let agent = match agent_id {
        Some(id) => load_agent(conn, &id).await?.map(|a| AgentRef {
            id: a.id,
            number: a.number,
            name: a.name,
            role: a.role,
            avatar: a.avatar,
        }),
        None => None,
    };

    let prerequisites = prerequisites(conn, project_id, story_id).await?;
    let pending_prerequisites = i64::try_from(
        prerequisites
            .iter()
            .filter(|p| p.state != RelationState::Accepted)
            .count(),
    )
    .unwrap_or(i64::MAX);

    Ok(StoryRelations {
        story_id: story_id.to_owned(),
        epic: epic.map(|(id, number, title)| EpicRef { id, number, title }),
        agent,
        parent,
        children: children(conn, project_id, story_id).await?,
        prerequisites,
        dependents: dependents(conn, project_id, story_id).await?,
        pending_prerequisites,
    })
}
