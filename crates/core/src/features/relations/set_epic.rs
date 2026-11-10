use serde::Deserialize;
use ts_rs::TS;

use super::get::{StoryRelations, load_relations};
use super::graph::{require, require_not_archived};
use crate::error::CoreError;
use crate::features::epics::load_epic;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetStoryEpicInput {
    pub project_id: String,
    pub story_id: String,
    /// `null` saca la historia de su épica.
    pub epic_id: Option<String>,
}

/// Asocia la historia a una épica del mismo proyecto (o la desasocia). Idempotente.
pub async fn set_story_epic(
    store: &Store,
    input: &SetStoryEpicInput,
) -> Result<StoryRelations, CoreError> {
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let story = require(conn, &input.project_id, &input.story_id).await?;
                require_not_archived(&story, "storyId")?;

                sqlx::query("DELETE FROM story_epics WHERE project_id = ? AND story_id = ?")
                    .bind(&input.project_id)
                    .bind(&input.story_id)
                    .execute(&mut *conn)
                    .await?;
                if let Some(epic_id) = &input.epic_id {
                    load_epic(conn, &input.project_id, epic_id)
                        .await?
                        .ok_or(CoreError::NotFound { entity: "Épica" })?;
                    sqlx::query(
                        "INSERT INTO story_epics (story_id, project_id, epic_id) VALUES (?, ?, ?)",
                    )
                    .bind(&input.story_id)
                    .bind(&input.project_id)
                    .bind(epic_id)
                    .execute(&mut *conn)
                    .await?;
                }
                load_relations(conn, &input.project_id, &input.story_id).await
            })
        })
        .await
}
