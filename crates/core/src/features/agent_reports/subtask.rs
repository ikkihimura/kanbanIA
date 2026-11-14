use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::features::access::AgentSession;
use crate::features::agent_activity::{ActivityKind, record_activity};
use crate::features::claims::require_active_claim;
use crate::features::relations::link_new_child;
use crate::features::stories::{
    NewStory, Priority, insert_story, validate_story_description, validate_story_title,
};
use crate::platform::storage::Store;
use crate::platform::text::has_invisible_formatting;

const OPERATION: &str = "agent_reports.subtask";
/// Subtareas sin archivar que puede tener una historia para que un agente cree otra: un
/// agente (con fallos o manipulado por el texto de una historia) no inunda el backlog.
pub const MAX_SUBTASKS_PER_STORY: i64 = 20;
/// Subtareas creadas por agentes sin aceptar ni archivar en un proyecto (decisión del
/// propietario): junto con el nivel único, frena que un agente inunde el backlog encadenando
/// subtareas.
pub const MAX_PENDING_AGENT_SUBTASKS: i64 = 30;
/// Descripción de una subtarea creada por un agente, en caracteres. Menor que la de la app
/// (20 000) para que el cuerpo siempre quepa en `MAX_BODY_BYTES` del servicio (64 KB) aunque
/// todos los caracteres ocupen 4 bytes.
pub const MAX_SUBTASK_DESCRIPTION_CHARS: usize = 10_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSubtaskInput {
    /// Repítelo al reintentar la misma creación.
    pub request_id: String,
    /// Reclamo vigente de la historia padre.
    pub claim_id: String,
    pub title: String,
    /// Markdown (hasta `MAX_SUBTASK_DESCRIPTION_CHARS` caracteres).
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    claim_id: &'a str,
    title: &'a str,
    description: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedSubtask {
    pub story_id: String,
    pub number: i64,
    pub title: String,
    pub parent_story_id: String,
    pub parent_number: i64,
}

/// Crea una subtarea de la historia reclamada: en la columna por defecto (backlog), sin agente
/// y con la prioridad del padre. Aplica WIP, el tope de subtareas por historia, el de subtareas
/// de agentes pendientes en el proyecto y un solo nivel (no en historias creadas por agentes).
pub async fn create_subtask(
    store: &Store,
    session: &AgentSession,
    input: &CreateSubtaskInput,
) -> Result<CreatedSubtask, CoreError> {
    let title = validate_story_title(&input.title)?;
    if has_invisible_formatting(&title) {
        return Err(CoreError::Validation {
            field: "title",
            reason: "El título contiene caracteres de formato invisibles.".into(),
        });
    }
    let description = validate_story_description(input.description.as_deref().unwrap_or(""))?;
    if description.chars().count() > MAX_SUBTASK_DESCRIPTION_CHARS {
        return Err(CoreError::Validation {
            field: "description",
            reason: "La descripción de una subtarea admite como máximo 10 000 caracteres.".into(),
        });
    }
    if has_invisible_formatting(&description) {
        return Err(CoreError::Validation {
            field: "description",
            reason: "La descripción contiene caracteres de formato invisibles.".into(),
        });
    }
    let key = session.receipt_key(OPERATION, &input.request_id)?;
    let payload = Payload {
        claim_id: &input.claim_id,
        title: &title,
        description: &description,
    };
    let (session, claim_id, clock) = (session.clone(), input.claim_id.clone(), store.clone());
    let id = uuid::Uuid::now_v7().to_string();
    let (title_tx, description_tx) = (title.clone(), description.clone());
    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let claim = require_active_claim(conn, &session, &claim_id, now).await?;
                let children: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM story_parents p JOIN stories s ON s.id = p.story_id \
                     WHERE p.parent_id = ? AND s.archived_at IS NULL",
                )
                .bind(&claim.story_id)
                .fetch_one(&mut *conn)
                .await?;
                // Un solo nivel: una historia creada por un agente no tiene subtareas de agentes.
                let parent_by_agent: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM agent_created_stories WHERE story_id = ?)",
                )
                .bind(&claim.story_id)
                .fetch_one(&mut *conn)
                .await?;
                if parent_by_agent {
                    return Err(CoreError::Validation {
                        field: "claimId",
                        reason: format!(
                            "La historia #{} la creó un agente: no admite subtareas de agentes. \
                             Pide a una persona que la divida.",
                            claim.story_number
                        ),
                    });
                }
                let pending: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM agent_created_stories o \
                     JOIN stories s ON s.id = o.story_id \
                     WHERE o.project_id = ? AND s.archived_at IS NULL AND s.accepted_at IS NULL",
                )
                .bind(session.project_id())
                .fetch_one(&mut *conn)
                .await?;
                if pending >= MAX_PENDING_AGENT_SUBTASKS {
                    return Err(CoreError::Validation {
                        field: "claimId",
                        reason: format!(
                            "El proyecto ya tiene {MAX_PENDING_AGENT_SUBTASKS} subtareas creadas \
                             por agentes sin aceptar ni archivar."
                        ),
                    });
                }
                if children >= MAX_SUBTASKS_PER_STORY {
                    return Err(CoreError::Validation {
                        field: "claimId",
                        reason: format!(
                            "La historia #{} ya tiene {MAX_SUBTASKS_PER_STORY} subtareas.",
                            claim.story_number
                        ),
                    });
                }
                let priority: String =
                    sqlx::query_scalar("SELECT priority FROM stories WHERE id = ?")
                        .bind(&claim.story_id)
                        .fetch_one(&mut *conn)
                        .await?;
                let subtask = insert_story(
                    conn,
                    &NewStory {
                        id: &id,
                        project_id: session.project_id(),
                        column_id: None,
                        title: &title_tx,
                        description: &description_tx,
                        priority: Priority::parse(&priority)?,
                    },
                )
                .await?;
                link_new_child(conn, session.project_id(), &subtask.id, &claim.story_id).await?;
                sqlx::query(
                    "INSERT INTO agent_created_stories \
                     (story_id, project_id, agent_id, credential_id, created_at_ms) \
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&subtask.id)
                .bind(session.project_id())
                .bind(session.agent_id())
                .bind(session.credential_id())
                .bind(now)
                .execute(&mut *conn)
                .await?;
                record_activity(
                    conn,
                    &session,
                    &claim.story_id,
                    ActivityKind::SubtaskCreated,
                    Some(&format!("#{}", subtask.number)),
                    now,
                )
                .await?;
                Ok(CreatedSubtask {
                    story_id: subtask.id,
                    number: subtask.number,
                    title: subtask.title,
                    parent_story_id: claim.story_id,
                    parent_number: claim.story_number,
                })
            })
        })
        .await
}

impl CreatedSubtask {
    /// Historias que cambió crear la subtarea: el padre y la nueva (para `AgentChange`).
    pub fn story_ids(&self) -> Vec<String> {
        vec![self.parent_story_id.clone(), self.story_id.clone()]
    }
}
