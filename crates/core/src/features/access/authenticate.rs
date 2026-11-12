use super::token;
use crate::error::CoreError;
use crate::platform::actor::Actor;
use crate::platform::idempotency::MutationKey;
use crate::platform::storage::Store;

/// Sesión autenticada de un agente. Las operaciones del servicio local toman de aquí la
/// identidad y el proyecto; nunca de la petición. Solo `authenticate_agent` la construye
/// (campos privados): ningún transporte puede fabricar una sesión para otro proyecto o agente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
    credential_id: String,
    project_id: String,
    agent_id: String,
}

impl AgentSession {
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Clave de recibos de idempotencia para las mutaciones de esta sesión: por credencial
    /// (que liga agente y proyecto), para no mezclar recibos de un agente entre proyectos.
    /// Toda mutación de agentes la usa en vez de `Actor::receipt_key`.
    pub fn receipt_key(
        &self,
        operation: &'static str,
        request_id: &str,
    ) -> Result<MutationKey, CoreError> {
        MutationKey::new(
            format!("credential:{}", self.credential_id),
            operation,
            request_id,
        )
    }

    pub fn actor(&self) -> Actor {
        Actor::Agent {
            agent_id: self.agent_id.clone(),
        }
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    project_id: String,
    agent_id: String,
    stale: bool,
}

/// Valida un token de agente. Token mal formado, desconocido o revocado → `Unauthorized`
/// (sin distinguir el caso). Registra el último uso con un minuto de resolución, para no
/// escribir en cada petición.
pub async fn authenticate_agent(store: &Store, token: &str) -> Result<AgentSession, CoreError> {
    if !token::well_formed(token) {
        return Err(CoreError::Unauthorized);
    }
    let hash = token::hash(token);
    let row: Option<Row> = sqlx::query_as(
        "SELECT id, project_id, agent_id, \
         (last_used_at IS NULL \
          OR last_used_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-60 seconds')) AS stale \
         FROM agent_credentials WHERE token_hash = ? AND revoked_at IS NULL",
    )
    .bind(&hash)
    .fetch_optional(store.reader())
    .await?;
    let row = row.ok_or(CoreError::Unauthorized)?;

    if row.stale {
        // Informativo: si la base está ocupada, la petición sigue adelante sin registrarlo.
        let id = row.id.clone();
        let touched = store
            .write(move |conn| {
                Box::pin(async move {
                    sqlx::query(
                        "UPDATE agent_credentials \
                         SET last_used_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                         WHERE id = ? AND revoked_at IS NULL",
                    )
                    .bind(&id)
                    .execute(&mut *conn)
                    .await?;
                    Ok(())
                })
            })
            .await;
        if let Err(error) = touched {
            eprintln!("[access] no se registró el último uso: {error}");
        }
    }

    Ok(AgentSession {
        credential_id: row.id,
        project_id: row.project_id,
        agent_id: row.agent_id,
    })
}
