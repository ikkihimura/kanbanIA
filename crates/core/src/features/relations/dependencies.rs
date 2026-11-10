use serde::Deserialize;
use ts_rs::TS;

use super::get::{StoryRelations, load_relations};
use super::graph::{reaches_through_dependencies, require, require_not_archived};
use crate::error::CoreError;
use crate::platform::storage::Store;

pub(crate) const MAX_PREREQUISITES: i64 = 50;

/// `story_id` depende de `depends_on_id`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DependencyInput {
    pub project_id: String,
    pub story_id: String,
    pub depends_on_id: String,
}

/// Añade un prerequisito. Idempotente. Rechaza ciclos directos e indirectos y prerequisitos
/// archivados (no podrían aceptarse).
pub async fn add_story_dependency(
    store: &Store,
    input: &DependencyInput,
) -> Result<StoryRelations, CoreError> {
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let story = require(conn, &input.project_id, &input.story_id).await?;
                require_not_archived(&story, "storyId")?;
                let prerequisite = require(conn, &input.project_id, &input.depends_on_id).await?;
                require_not_archived(&prerequisite, "dependsOnId")?;

                // Ciclo si la historia ya es alcanzable desde el prerequisito (o es él mismo).
                if reaches_through_dependencies(conn, &input.depends_on_id, &input.story_id).await? {
                    return Err(CoreError::Cycle {
                        field: "dependsOnId",
                        reason: if prerequisite.id == story.id {
                            format!("La historia #{} no puede depender de sí misma.", story.number)
                        } else {
                            format!(
                                "#{} ya depende (directa o indirectamente) de #{}: \
                                 añadir esta dependencia crearía un ciclo.",
                                prerequisite.number, story.number
                            )
                        },
                    });
                }

                let count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM story_dependencies WHERE project_id = ? AND story_id = ?",
                )
                .bind(&input.project_id)
                .bind(&input.story_id)
                .fetch_one(&mut *conn)
                .await?;
                if count >= MAX_PREREQUISITES {
                    return Err(CoreError::Validation {
                        field: "dependsOnId",
                        reason: "Una historia admite como máximo 50 prerequisitos.".into(),
                    });
                }

                sqlx::query(
                    "INSERT OR IGNORE INTO story_dependencies (story_id, depends_on_id, project_id) \
                     VALUES (?, ?, ?)",
                )
                .bind(&input.story_id)
                .bind(&input.depends_on_id)
                .bind(&input.project_id)
                .execute(&mut *conn)
                .await?;
                load_relations(conn, &input.project_id, &input.story_id).await
            })
        })
        .await
}

/// Quita un prerequisito si existe. Idempotente; permitido aunque alguna esté archivada,
/// para poder desbloquear historias que dependían de trabajo descartado.
pub async fn remove_story_dependency(
    store: &Store,
    input: &DependencyInput,
) -> Result<StoryRelations, CoreError> {
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                require(conn, &input.project_id, &input.story_id).await?;
                sqlx::query(
                    "DELETE FROM story_dependencies \
                     WHERE project_id = ? AND story_id = ? AND depends_on_id = ?",
                )
                .bind(&input.project_id)
                .bind(&input.story_id)
                .bind(&input.depends_on_id)
                .execute(&mut *conn)
                .await?;
                load_relations(conn, &input.project_id, &input.story_id).await
            })
        })
        .await
}
