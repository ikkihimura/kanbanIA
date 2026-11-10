//! Quién ejecuta una operación. Las reglas que dependen de la identidad (solo una persona
//! acepta, reabre o devuelve entregas) se aplican en el núcleo, para todos los transportes:
//! la UI de escritorio actúa como `Human`; las sesiones MCP (M2) actuarán como `Agent`.

use crate::error::CoreError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// El usuario local, desde la aplicación de escritorio.
    Human,
    /// Un agente del equipo (sesión MCP o adaptador).
    Agent { agent_id: String },
}

impl Actor {
    /// Clave para recibos de idempotencia (compatible con el actor "ui" ya existente).
    pub fn receipt_key(&self) -> String {
        match self {
            Self::Human => "ui".to_owned(),
            Self::Agent { agent_id } => format!("agent:{agent_id}"),
        }
    }

    /// Falla con `Forbidden` si no es una persona.
    pub fn require_human(&self, action: &str) -> Result<(), CoreError> {
        match self {
            Self::Human => Ok(()),
            Self::Agent { .. } => Err(CoreError::Forbidden {
                reason: format!("Solo una persona puede {action}; un agente no puede hacerlo."),
            }),
        }
    }
}
