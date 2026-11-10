use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::avatar::AvatarSpec;
use super::validate_text;
use crate::error::CoreError;
use crate::platform::idempotency::MutationKey;
use crate::platform::storage::Store;

pub(crate) const ENTITY: &str = "Agente";
const OPERATION: &str = "agents.create";

/// Qué puede hacer un agente (conjunto fijo). Es informativo hasta que existan sesiones (M2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Capability {
    Plan,
    Implement,
    Review,
    Test,
    Document,
}

/// Agente de IA del equipo. La identidad (id, número, nombre, avatar) no depende del modelo:
/// cambiar proveedor o modelo conserva al mismo agente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Agent {
    pub id: String,
    /// Se muestra como A-n.
    #[ts(type = "number")]
    pub number: i64,
    pub name: String,
    pub role: String,
    /// Texto informativo: no implica conexión con ningún proveedor.
    pub provider: String,
    pub model: String,
    pub capabilities: Vec<Capability>,
    pub avatar: AvatarSpec,
    /// Historias no archivadas asignadas. Asignar no implica actividad.
    #[ts(type = "number")]
    pub assigned_stories: i64,
    #[ts(type = "number")]
    pub revision: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentFields {
    pub name: String,
    pub role: String,
    pub provider: String,
    pub model: String,
    pub capabilities: Vec<Capability>,
    pub avatar: AvatarSpec,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateAgentInput {
    pub request_id: String,
    #[serde(flatten)]
    #[ts(flatten)]
    pub fields: AgentFields,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateAgentInput {
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    #[serde(flatten)]
    #[ts(flatten)]
    pub fields: AgentFields,
}

struct ValidFields {
    name: String,
    name_key: String,
    role: String,
    provider: String,
    model: String,
    capabilities: String,
    avatar: String,
}

fn validate(fields: &AgentFields) -> Result<ValidFields, CoreError> {
    let name = validate_text("name", &fields.name, 1, 40, "El nombre")?;
    let mut capabilities = fields.capabilities.clone();
    capabilities.sort_unstable();
    capabilities.dedup();
    Ok(ValidFields {
        name_key: name.to_lowercase(),
        name,
        role: validate_text("role", &fields.role, 0, 60, "El rol")?,
        provider: validate_text("provider", &fields.provider, 0, 60, "El proveedor")?,
        model: validate_text("model", &fields.model, 0, 80, "El modelo")?,
        capabilities: serde_json::to_string(&capabilities)?,
        avatar: fields.avatar.validate()?.to_json()?,
    })
}

#[derive(sqlx::FromRow)]
struct AgentRow {
    id: String,
    number: i64,
    name: String,
    role: String,
    provider: String,
    model: String,
    capabilities: String,
    avatar: String,
    assigned_stories: i64,
    revision: i64,
}

impl TryFrom<AgentRow> for Agent {
    type Error = CoreError;

    fn try_from(row: AgentRow) -> Result<Self, CoreError> {
        Ok(Self {
            capabilities: serde_json::from_str(&row.capabilities)?,
            avatar: AvatarSpec::from_json(&row.avatar)?,
            id: row.id,
            number: row.number,
            name: row.name,
            role: row.role,
            provider: row.provider,
            model: row.model,
            assigned_stories: row.assigned_stories,
            revision: row.revision,
        })
    }
}

macro_rules! select_agent {
    ($tail:literal) => {
        concat!(
            "SELECT a.id, a.number, a.name, a.role, a.provider, a.model, a.capabilities, \
             a.avatar, a.revision, \
             (SELECT COUNT(*) FROM story_agents sa JOIN stories s ON s.id = sa.story_id \
              WHERE sa.agent_id = a.id AND s.archived_at IS NULL) AS assigned_stories \
             FROM agents a ",
            $tail
        )
    };
}

pub(crate) async fn load_agent(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<Agent>, CoreError> {
    let row: Option<AgentRow> = sqlx::query_as(select_agent!("WHERE a.id = ?"))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(Agent::try_from).transpose()
}

pub async fn list_agents(store: &Store) -> Result<Vec<Agent>, CoreError> {
    let mut conn = store.reader().acquire().await?;
    let rows: Vec<AgentRow> = sqlx::query_as(select_agent!("ORDER BY a.number"))
        .fetch_all(&mut *conn)
        .await?;
    rows.into_iter().map(Agent::try_from).collect()
}

async fn ensure_name_free(
    conn: &mut SqliteConnection,
    name_key: &str,
    except_id: &str,
) -> Result<(), CoreError> {
    let taken: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM agents WHERE name_key = ? AND id <> ?)")
            .bind(name_key)
            .bind(except_id)
            .fetch_one(&mut *conn)
            .await?;
    if taken {
        return Err(CoreError::AlreadyExists {
            entity: "Un agente con ese nombre",
            field: "name",
        });
    }
    Ok(())
}

pub async fn create_agent(
    store: &Store,
    actor: &str,
    input: &CreateAgentInput,
) -> Result<Agent, CoreError> {
    let key = MutationKey::new(actor, OPERATION, &input.request_id)?;
    let valid = validate(&input.fields)?;
    let id = uuid::Uuid::now_v7().to_string();

    store
        .write_idempotent(&key, &input.fields, move |conn| {
            Box::pin(async move {
                ensure_name_free(conn, &valid.name_key, &id).await?;
                let number: i64 =
                    sqlx::query_scalar("SELECT COALESCE(MAX(number), 0) + 1 FROM agents")
                        .fetch_one(&mut *conn)
                        .await?;
                sqlx::query(
                    "INSERT INTO agents (id, number, name, name_key, role, provider, model, \
                     capabilities, avatar) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(number)
                .bind(&valid.name)
                .bind(&valid.name_key)
                .bind(&valid.role)
                .bind(&valid.provider)
                .bind(&valid.model)
                .bind(&valid.capabilities)
                .bind(&valid.avatar)
                .execute(&mut *conn)
                .await?;
                load_agent(conn, &id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}

/// Edita el agente. El id, el número y el avatar (salvo que se cambie explícitamente) se
/// conservan: cambiar de modelo no crea otro agente.
pub async fn update_agent(store: &Store, input: &UpdateAgentInput) -> Result<Agent, CoreError> {
    let valid = validate(&input.fields)?;
    let id = input.id.clone();
    let expected = input.expected_revision;

    store
        .write(move |conn| {
            Box::pin(async move {
                let current = load_agent(conn, &id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })?;
                if current.revision != expected {
                    return Err(CoreError::Conflict {
                        current: serde_json::to_value(&current)?,
                    });
                }
                ensure_name_free(conn, &valid.name_key, &id).await?;
                sqlx::query(
                    "UPDATE agents SET name = ?, name_key = ?, role = ?, provider = ?, model = ?, \
                     capabilities = ?, avatar = ?, revision = revision + 1, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE id = ? AND revision = ?",
                )
                .bind(&valid.name)
                .bind(&valid.name_key)
                .bind(&valid.role)
                .bind(&valid.provider)
                .bind(&valid.model)
                .bind(&valid.capabilities)
                .bind(&valid.avatar)
                .bind(&id)
                .bind(expected)
                .execute(&mut *conn)
                .await?;
                load_agent(conn, &id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: ENTITY })
            })
        })
        .await
}
