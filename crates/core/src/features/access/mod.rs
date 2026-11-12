//! Slice `access`: credenciales de agentes para el servicio local (HU-012).
//!
//! Una credencial liga un agente a un proyecto. El token se muestra una sola vez al crearlo y
//! solo se guarda su hash. Autenticar un token devuelve una [`AgentSession`]: de ella (nunca
//! de un campo de la petición) salen la identidad del agente y el proyecto permitido.
//! Crear y revocar credenciales es cosa de personas: nunca se expone a agentes.

mod authenticate;
mod create;
mod credential;
mod list;
mod restore;
mod revoke;
mod session;
mod token;

pub use authenticate::{AgentSession, authenticate_agent};
pub use create::{CreateAgentCredentialInput, CreatedAgentCredential, create_agent_credential};
pub use credential::{AgentCredential, MAX_ACTIVE_PER_PROJECT};
pub use list::list_agent_credentials;
pub(crate) use restore::{current_revocations, reconcile_restored};
pub use revoke::{RevokeAgentCredentialInput, revoke_agent_credential};
pub use session::{LocalServiceStatus, SessionInfo, describe_session};
