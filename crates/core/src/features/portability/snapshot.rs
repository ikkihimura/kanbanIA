//! Estado local de un proyecto, leído con pocas consultas, para exportar y para comparar con
//! un paquete. Incluye archivadas.

use std::collections::HashMap;

use sqlx::SqliteConnection;

use super::package::{EpicContent, StoryContent};
use crate::error::CoreError;
use crate::features::stories::Priority;

pub(super) struct LocalEpic {
    pub id: String,
    pub number: i64,
    pub title: String,
    pub description: String,
    pub revision: i64,
}

impl LocalEpic {
    pub fn content(&self) -> EpicContent {
        EpicContent {
            title: self.title.clone(),
            description: self.description.clone(),
        }
    }
}

pub(super) struct LocalCriterion {
    pub id: String,
    pub number: i64,
    pub text: String,
}

pub(super) struct LocalStory {
    pub id: String,
    pub number: i64,
    pub column_id: String,
    pub position: Option<i64>,
    pub title: String,
    pub description: String,
    pub priority: Priority,
    pub archived: bool,
    pub accepted_at: Option<String>,
    pub next_criterion_number: i64,
    pub revision: i64,
    pub criteria: Vec<LocalCriterion>,
    pub epic: Option<String>,
    pub parent: Option<String>,
    pub depends_on: Vec<String>,
    pub agent: Option<(String, String)>,
    pub blocker: Option<String>,
}

impl LocalStory {
    pub fn content(&self) -> StoryContent {
        StoryContent {
            title: self.title.clone(),
            description: self.description.clone(),
            priority: self.priority,
            criteria: self
                .criteria
                .iter()
                .map(|c| (Some(c.id.clone()), c.text.clone()))
                .collect(),
            epic: self.epic.clone(),
            parent: self.parent.clone(),
            depends_on: self.depends_on.clone(),
        }
    }
}

pub(super) struct LocalProject {
    /// En orden de número.
    pub epics: Vec<LocalEpic>,
    /// En orden de número.
    pub stories: Vec<LocalStory>,
}

impl LocalProject {
    pub fn epic(&self, id: &str) -> Option<&LocalEpic> {
        self.epics.iter().find(|e| e.id == id)
    }

    pub fn story_index(&self) -> HashMap<&str, &LocalStory> {
        self.stories.iter().map(|s| (s.id.as_str(), s)).collect()
    }
}

type StoryRow = (
    String,
    i64,
    String,
    Option<i64>,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    i64,
);

pub(super) async fn load_local(
    conn: &mut SqliteConnection,
    project_id: &str,
) -> Result<LocalProject, CoreError> {
    let epics: Vec<(String, i64, String, String, i64)> = sqlx::query_as(
        "SELECT id, number, title, description, revision FROM epics \
         WHERE project_id = ? ORDER BY number",
    )
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?;
    let epics = epics
        .into_iter()
        .map(|(id, number, title, description, revision)| LocalEpic {
            id,
            number,
            title,
            description,
            revision,
        })
        .collect();

    let rows: Vec<StoryRow> = sqlx::query_as(
        "SELECT id, number, column_id, position, title, description, priority, archived_at, \
         accepted_at, next_criterion_number, revision FROM stories \
         WHERE project_id = ? ORDER BY number",
    )
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?;

    let criteria: Vec<(String, String, i64, String)> = sqlx::query_as(
        "SELECT c.story_id, c.id, c.number, c.text FROM acceptance_criteria c \
         JOIN stories s ON s.id = c.story_id WHERE s.project_id = ? \
         ORDER BY c.story_id, c.position",
    )
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut criteria_by: HashMap<String, Vec<LocalCriterion>> = HashMap::new();
    for (story, id, number, text) in criteria {
        criteria_by
            .entry(story)
            .or_default()
            .push(LocalCriterion { id, number, text });
    }

    let epic_of: HashMap<String, String> =
        sqlx::query_as("SELECT story_id, epic_id FROM story_epics WHERE project_id = ?")
            .bind(project_id)
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .collect();
    let parent_of: HashMap<String, String> =
        sqlx::query_as("SELECT story_id, parent_id FROM story_parents WHERE project_id = ?")
            .bind(project_id)
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .collect();
    let deps: Vec<(String, String)> = sqlx::query_as(
        "SELECT story_id, depends_on_id FROM story_dependencies WHERE project_id = ? \
         ORDER BY story_id, depends_on_id",
    )
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut deps_of: HashMap<String, Vec<String>> = HashMap::new();
    for (story, dep) in deps {
        deps_of.entry(story).or_default().push(dep);
    }
    let agents: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT sa.story_id, a.id, a.name FROM story_agents sa \
         JOIN agents a ON a.id = sa.agent_id JOIN stories s ON s.id = sa.story_id \
         WHERE s.project_id = ?",
    )
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut agent_of: HashMap<String, (String, String)> = agents
        .into_iter()
        .map(|(story, id, name)| (story, (id, name)))
        .collect();
    let mut blocker_of: HashMap<String, String> =
        sqlx::query_as("SELECT story_id, reason FROM story_blockers WHERE project_id = ?")
            .bind(project_id)
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .collect();

    let mut stories = Vec::with_capacity(rows.len());
    for row in rows {
        let (
            id,
            number,
            column_id,
            position,
            title,
            description,
            priority,
            archived_at,
            accepted_at,
            next_criterion_number,
            revision,
        ) = row;
        stories.push(LocalStory {
            criteria: criteria_by.remove(&id).unwrap_or_default(),
            epic: epic_of.get(&id).cloned(),
            parent: parent_of.get(&id).cloned(),
            depends_on: deps_of.remove(&id).unwrap_or_default(),
            agent: agent_of.remove(&id),
            blocker: blocker_of.remove(&id),
            id,
            number,
            column_id,
            position,
            title,
            description,
            priority: Priority::parse(&priority)?,
            archived: archived_at.is_some(),
            accepted_at,
            next_criterion_number,
            revision,
        });
    }
    Ok(LocalProject { epics, stories })
}
