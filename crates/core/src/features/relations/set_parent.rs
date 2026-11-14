use serde::Deserialize;
use ts_rs::TS;

use super::get::{StoryRelations, load_relations};
use super::graph::{is_self_or_ancestor, require, require_not_archived};
use crate::error::CoreError;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetStoryParentInput {
    pub project_id: String,
    pub story_id: String,
    /// `null` convierte la subtarea en historia independiente.
    pub parent_id: Option<String>,
}

/// Convierte la historia en subtarea de `parent_id` (o la independiza). Idempotente.
/// Rechaza ciclos directos e indirectos: el padre no puede ser la propia historia ni una de
/// sus subtareas a cualquier profundidad.
pub async fn set_story_parent(
    store: &Store,
    input: &SetStoryParentInput,
) -> Result<StoryRelations, CoreError> {
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let story = require(conn, &input.project_id, &input.story_id).await?;
                require_not_archived(&story, "storyId")?;

                if let Some(parent_id) = &input.parent_id {
                    let parent = require(conn, &input.project_id, parent_id).await?;
                    require_not_archived(&parent, "parentId")?;
                    // Ciclo si la historia ya es el padre propuesto o uno de sus antecesores.
                    if is_self_or_ancestor(conn, parent_id, &input.story_id).await? {
                        return Err(CoreError::Cycle {
                            field: "parentId",
                            reason: if parent.id == story.id {
                                format!("La historia #{} no puede ser su propia subtarea.", story.number)
                            } else {
                                format!(
                                    "#{} ya es subtarea (directa o indirecta) de #{}: \
                                     no puede ser su padre.",
                                    parent.number, story.number
                                )
                            },
                        });
                    }
                }

                sqlx::query("DELETE FROM story_parents WHERE project_id = ? AND story_id = ?")
                    .bind(&input.project_id)
                    .bind(&input.story_id)
                    .execute(&mut *conn)
                    .await?;
                if let Some(parent_id) = &input.parent_id {
                    sqlx::query(
                        "INSERT INTO story_parents (story_id, project_id, parent_id) VALUES (?, ?, ?)",
                    )
                    .bind(&input.story_id)
                    .bind(&input.project_id)
                    .bind(parent_id)
                    .execute(&mut *conn)
                    .await?;
                }
                load_relations(conn, &input.project_id, &input.story_id).await
            })
        })
        .await
}

/// Enlaza como subtarea de `parent_id` una historia recién creada en la misma transacción (no
/// puede formar ciclo ni tener padre). Toda regla nueva de parentesco va aquí también.
pub(crate) async fn link_new_child(
    conn: &mut sqlx::SqliteConnection,
    project_id: &str,
    child_id: &str,
    parent_id: &str,
) -> Result<(), CoreError> {
    let parent = require(conn, project_id, parent_id).await?;
    require_not_archived(&parent, "parentId")?;
    sqlx::query("INSERT INTO story_parents (story_id, project_id, parent_id) VALUES (?, ?, ?)")
        .bind(child_id)
        .bind(project_id)
        .bind(parent_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}
