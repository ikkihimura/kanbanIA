//! Slice `team`: el usuario local y los agentes de IA, con avatar por piezas.
//! La asignación de agentes a historias vive en `relations` (como épica y padre).

mod agents;
mod avatar;
mod user;

pub use agents::{
    Agent, AgentFields, Capability, CreateAgentInput, UpdateAgentInput, create_agent, list_agents,
    update_agent,
};
pub use avatar::{AVATAR_LIMITS, AvatarSpec};
pub use user::{UpdateUserProfileInput, UserProfile, get_user_profile, update_user_profile};

pub(crate) use agents::load_agent;

use crate::error::CoreError;

/// Texto de una línea: recortado, longitud en caracteres y sin caracteres de control.
fn validate_text(
    field: &'static str,
    raw: &str,
    min: usize,
    max: usize,
    label: &str,
) -> Result<String, CoreError> {
    let text = raw.trim();
    let invalid = |reason: String| CoreError::Validation { field, reason };
    let len = text.chars().count();
    if len < min {
        return Err(invalid(format!("{label} es obligatorio.")));
    }
    if len > max {
        return Err(invalid(format!(
            "{label} admite como máximo {max} caracteres."
        )));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid(format!(
            "{label} no puede contener caracteres de control."
        )));
    }
    Ok(text.to_owned())
}
