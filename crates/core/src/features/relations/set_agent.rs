use serde::Deserialize;
use ts_rs::TS;

use super::get::{StoryRelations, load_relations};
use super::graph::{require, require_not_archived};
use crate::error::CoreError;
use crate::features::team::load_agent;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetStoryAgentInput {
    pub project_id: String,
    pub story_id: String,
    /// `null` deja la historia sin agente asignado.
    pub agent_id: Option<String>,
}

/// Asigna un agente a la historia (o lo quita). Idempotente.
///
/// Asignar solo registra quién debería encargarse: no crea ejecuciones, no cambia la columna
/// ni la revisión de la historia y no implica actividad del agente.
pub async fn set_story_agent(
    store: &Store,
    input: &SetStoryAgentInput,
) -> Result<StoryRelations, CoreError> {
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let story = require(conn, &input.project_id, &input.story_id).await?;
                require_not_archived(&story, "storyId")?;

                match &input.agent_id {
                    Some(agent_id) => {
                        load_agent(conn, agent_id)
                            .await?
                            .ok_or(CoreError::NotFound { entity: "Agente" })?;
                        // Reasignar el mismo agente no cambia nada (tampoco `assigned_at`).
                        sqlx::query(
                            "INSERT INTO story_agents (story_id, agent_id) VALUES (?, ?) \
                             ON CONFLICT (story_id) DO UPDATE SET \
                                 agent_id = excluded.agent_id, \
                                 assigned_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                             WHERE story_agents.agent_id <> excluded.agent_id",
                        )
                        .bind(&input.story_id)
                        .bind(agent_id)
                        .execute(&mut *conn)
                        .await?;
                    }
                    None => {
                        sqlx::query("DELETE FROM story_agents WHERE story_id = ?")
                            .bind(&input.story_id)
                            .execute(&mut *conn)
                            .await?;
                    }
                }
                load_relations(conn, &input.project_id, &input.story_id).await
            })
        })
        .await
}
