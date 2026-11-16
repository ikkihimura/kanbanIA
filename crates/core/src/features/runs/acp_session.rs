//! Registro de la sesión ACP de un intento (HU-019, CA-1): lo que el agente **declaró** al
//! negociar (protocolo, nombre, versión, capacidades), el modo que fijó la app y la huella del
//! prompt enviado. Son datos reportados por el agente, no verificados: la UI los muestra como
//! «reportado por el agente».
//!
//! No confundir con `access::AgentSession` (credencial de un agente en el servicio local/MCP):
//! aquí «sesión» es la sesión ACP de un intento.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use super::event::RunEvent;
use super::run::{RunState, RunSummary, append_event, require_run_any, validate_id};
use super::supervisor::{SupervisorToken, ensure_token};
use crate::error::CoreError;
use crate::platform::storage::Store;
use crate::platform::text::{contains_secret_like, has_invisible_formatting};

/// Caracteres máximos del nombre y del título del agente.
pub const MAX_ACP_NAME_CHARS: usize = 100;
/// Caracteres máximos (ASCII imprimible) de la versión del agente y del modo.
pub const MAX_ACP_VERSION_CHARS: usize = 50;
/// Capacidades declaradas como mucho.
pub const MAX_ACP_CAPABILITIES: usize = 16;
/// Capacidades ACP v1 que se registran (lista permitida; `_meta` y extensiones nunca).
pub const ACP_CAPABILITIES: [&str; 7] = [
    "loadSession",
    "image",
    "audio",
    "embeddedContext",
    "mcpHttp",
    "mcpSse",
    "modes",
];

/// Sesión ACP negociada para un intento, tal como la reportó el agente. Sin `_meta`, sin datos
/// de cuenta ni rutas. Constrúyela con [`AcpSessionInfo::from_reported`] (y
/// [`AcpSessionInfo::with_prompt`]): así nunca falla la validación de `record_acp_session`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcpSessionInfo {
    /// Versión de ACP negociada (hoy, siempre 1).
    pub protocol_version: u16,
    /// `agentInfo.name` (≤ 100 caracteres). `null` si el agente no lo informó (en ACP v1
    /// `agentInfo` es opcional) o si la identidad no cumplía los límites.
    pub agent_name: Option<String>,
    /// `agentInfo.title` (≤ 100 caracteres).
    pub agent_title: Option<String>,
    /// `agentInfo.version` (≤ 50 caracteres ASCII imprimibles).
    pub agent_version: Option<String>,
    /// Capacidades declaradas, de [`ACP_CAPABILITIES`] (sin repetir, como mucho 16).
    pub capabilities: Vec<String>,
    /// Modo de sesión que fijó la app (p. ej. `default`); `null` si no se fijó ninguno.
    pub mode: Option<String>,
    /// SHA-256 (64 hex minúsculas) del texto del prompt realmente enviado; `null` si aún no se
    /// envió. Va junto con `promptChars`.
    #[serde(default)]
    pub prompt_sha256: Option<String>,
    /// Caracteres del prompt enviado.
    #[serde(default)]
    pub prompt_chars: Option<u32>,
    /// Modelo que el agente **declaró** al crear la sesión (`models.currentModelId` de ACP, ≤ 50
    /// caracteres ASCII imprimibles; HU-021). La app no lo elige ni lo verifica; `null` si no
    /// lo informó o no cumplía los límites.
    #[serde(default)]
    pub model: Option<String>,
}

/// Resultado de [`AcpSessionInfo::from_reported`]: la sesión normalizada y lo que se omitió.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedSession {
    pub info: AcpSessionInfo,
    /// La identidad (nombre, título o versión) no cumplía los límites: se omitió entera
    /// (nunca se recorta en silencio).
    pub identity_dropped: bool,
    /// El modo no cumplía los límites y se omitió.
    pub mode_dropped: bool,
    /// Capacidades desconocidas (o repetidas) que se filtraron.
    pub dropped_capabilities: usize,
}

/// SHA-256 en hex minúsculas y número de caracteres del texto de un prompt (para
/// `AcpSessionInfo::prompt_sha256` y `prompt_chars`). Más de `u32::MAX` caracteres →
/// `u32::MAX`.
pub fn prompt_digest(text: &str) -> (String, u32) {
    let digest: String = Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let chars = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    (digest, chars)
}

fn invalid(reason: String) -> CoreError {
    CoreError::Validation {
        field: "acpSession",
        reason,
    }
}

fn is_unsafe_text(text: &str) -> bool {
    text.chars().any(char::is_control)
        || has_invisible_formatting(text)
        || contains_secret_like(text)
}

fn name_like(label: &str, value: Option<&str>) -> Result<Option<String>, CoreError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_ACP_NAME_CHARS || is_unsafe_text(value) {
        return Err(invalid(format!(
            "El {label} del agente admite una línea de 1 a {MAX_ACP_NAME_CHARS} caracteres \
             sin formato invisible."
        )));
    }
    Ok(Some(value.to_owned()))
}

fn ascii_token(label: &str, value: Option<&str>) -> Result<Option<String>, CoreError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_ACP_VERSION_CHARS
        || !value.bytes().all(|b| (0x20..=0x7e).contains(&b))
        || contains_secret_like(value)
    {
        return Err(invalid(format!(
            "{label} admite de 1 a {MAX_ACP_VERSION_CHARS} caracteres ASCII imprimibles."
        )));
    }
    Ok(Some(value.to_owned()))
}

fn prompt_fields(
    sha256: Option<&str>,
    chars: Option<u32>,
) -> Result<(Option<String>, Option<u32>), CoreError> {
    match (sha256, chars) {
        (None, None) => Ok((None, None)),
        (Some(sha), Some(chars))
            if sha.len() == 64 && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) =>
        {
            Ok((Some(sha.to_owned()), Some(chars)))
        }
        _ => Err(invalid(
            "La huella del prompt son 64 caracteres hex en minúsculas junto con su número de \
             caracteres."
                .into(),
        )),
    }
}

/// Nombre, título y versión validados; cualquiera fuera de los límites → `Err`.
type Identity = (Option<String>, Option<String>, Option<String>);

fn identity(
    name: Option<&str>,
    title: Option<&str>,
    version: Option<&str>,
) -> Result<Identity, CoreError> {
    Ok((
        name_like("nombre", name)?,
        name_like("título", title)?,
        ascii_token("La versión del agente", version)?,
    ))
}

impl AcpSessionInfo {
    /// Único constructor de lo que reporta un agente (también para [`AcpAdapterCheck`]):
    /// normaliza los textos; si la identidad (nombre, título o versión) no cumple los límites,
    /// la omite entera (`identity_dropped`); un modo no válido se omite (`mode_dropped`); las
    /// capacidades desconocidas o repetidas se filtran (`dropped_capabilities`). Sin huella de
    /// prompt (ver [`Self::with_prompt`]). El resultado solo puede fallar en
    /// `record_acp_session` si `protocol_version` es 0.
    pub fn from_reported(
        protocol_version: u16,
        agent_name: Option<String>,
        agent_title: Option<String>,
        agent_version: Option<String>,
        capabilities: Vec<String>,
        mode: Option<String>,
    ) -> ReportedSession {
        let (agent_name, agent_title, agent_version, identity_dropped) = match identity(
            agent_name.as_deref(),
            agent_title.as_deref(),
            agent_version.as_deref(),
        ) {
            Ok((name, title, version)) => (name, title, version, false),
            Err(_) => (None, None, None, true),
        };
        let (mode, mode_dropped) = match ascii_token("El modo", mode.as_deref()) {
            Ok(mode) => (mode, false),
            Err(_) => (None, true),
        };
        let total = capabilities.len();
        let mut kept: Vec<String> = Vec::new();
        for capability in capabilities {
            if ACP_CAPABILITIES.contains(&capability.as_str())
                && !kept.contains(&capability)
                && kept.len() < MAX_ACP_CAPABILITIES
            {
                kept.push(capability);
            }
        }
        ReportedSession {
            dropped_capabilities: total - kept.len(),
            info: Self {
                protocol_version,
                agent_name,
                agent_title,
                agent_version,
                capabilities: kept,
                mode,
                prompt_sha256: None,
                prompt_chars: None,
                model: None,
            },
            identity_dropped,
            mode_dropped,
        }
    }

    /// Añade la huella ([`prompt_digest`]) del texto del prompt realmente enviado.
    #[must_use]
    pub fn with_prompt(mut self, text: &str) -> Self {
        let (sha256, chars) = prompt_digest(text);
        self.prompt_sha256 = Some(sha256);
        self.prompt_chars = Some(chars);
        self
    }

    /// Añade el modelo que declaró el agente; si no cumple los límites, se omite.
    #[must_use]
    pub fn with_model(mut self, model: Option<&str>) -> Self {
        self.model = ascii_token("El modelo", model).ok().flatten();
        self
    }

    /// Valida y normaliza (recorta textos, quita capacidades repetidas conservando el orden).
    /// Cualquier valor fuera de los límites → `validation` (`acpSession`): nunca se recorta
    /// en silencio un dato reportado. Defensa para quien no use `from_reported`.
    pub(super) fn normalized(&self) -> Result<Self, CoreError> {
        if self.protocol_version == 0 {
            return Err(invalid("La versión de protocolo no es válida.".into()));
        }
        if self.capabilities.len() > MAX_ACP_CAPABILITIES {
            return Err(invalid(format!(
                "Se admiten como mucho {MAX_ACP_CAPABILITIES} capacidades."
            )));
        }
        let mut capabilities: Vec<String> = Vec::with_capacity(self.capabilities.len());
        for capability in &self.capabilities {
            if !ACP_CAPABILITIES.contains(&capability.as_str()) {
                return Err(invalid("Capacidad de agente desconocida.".into()));
            }
            if !capabilities.contains(capability) {
                capabilities.push(capability.clone());
            }
        }
        let (agent_name, agent_title, agent_version) = identity(
            self.agent_name.as_deref(),
            self.agent_title.as_deref(),
            self.agent_version.as_deref(),
        )?;
        let (prompt_sha256, prompt_chars) =
            prompt_fields(self.prompt_sha256.as_deref(), self.prompt_chars)?;
        Ok(Self {
            protocol_version: self.protocol_version,
            agent_name,
            agent_title,
            agent_version,
            capabilities,
            mode: ascii_token("El modo", self.mode.as_deref())?,
            prompt_sha256,
            prompt_chars,
            model: ascii_token("El modelo", self.model.as_deref())?,
        })
    }
}

/// Resultado de «Comprobar adaptador» (HU-019): lo que el adaptador **declaró** al responder a
/// `initialize`, con las mismas reglas que [`AcpSessionInfo`] (sin `_meta`, sin datos de cuenta
/// ni rutas). Solo lo construye [`AcpAdapterCheck::from_reported`]. Nunca para agentes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcpAdapterCheck {
    /// Versión de ACP que respondió el adaptador (solo se devuelve si es la 1).
    pub protocol_version: u16,
    pub agent_name: Option<String>,
    pub agent_title: Option<String>,
    pub agent_version: Option<String>,
    /// Capacidades declaradas, de [`ACP_CAPABILITIES`] (`modes` nunca: solo se sabe al crear
    /// una sesión).
    pub capabilities: Vec<String>,
}

impl AcpAdapterCheck {
    /// Deriva de [`AcpSessionInfo::from_reported`]: identidad fuera de los límites → omitida;
    /// capacidades desconocidas → filtradas. Solo falla con protocolo 0 (`validation`,
    /// `acpSession`).
    pub fn from_reported(
        protocol_version: u16,
        agent_name: Option<String>,
        agent_title: Option<String>,
        agent_version: Option<String>,
        capabilities: Vec<String>,
    ) -> Result<Self, CoreError> {
        if protocol_version == 0 {
            return Err(invalid("La versión de protocolo no es válida.".into()));
        }
        let info = AcpSessionInfo::from_reported(
            protocol_version,
            agent_name,
            agent_title,
            agent_version,
            capabilities,
            None,
        )
        .info;
        Ok(Self {
            protocol_version: info.protocol_version,
            agent_name: info.agent_name,
            agent_title: info.agent_title,
            agent_version: info.agent_version,
            capabilities: info.capabilities,
        })
    }
}

/// Añade un evento `acp_session` al intento con la siguiente secuencia del contador
/// compartido, en una transacción `BEGIN IMMEDIATE`. Solo en `preparing` o `running` (la sesión
/// se negocia tras lanzar el adaptador). No cambia el estado. Se puede llamar más de una vez
/// (p. ej. tras negociar y tras enviar el prompt): cada llamada es un evento nuevo.
///
/// API del supervisor (exige el [`SupervisorToken`] del almacén): sin comando Tauri ni ruta del
/// servicio local o MCP.
///
/// Errores: token de otro almacén → `forbidden`; intento inexistente → `not_found`; datos
/// fuera de los límites (protocolo 0, capacidad desconocida, formato invisible, algo que parece
/// un secreto, huella del prompt mal formada) → `validation` (`acpSession`); el intento no está
/// en `preparing` ni en `running` → `validation` (`runId`).
pub async fn record_acp_session(
    store: &Store,
    token: &SupervisorToken,
    run_id: &str,
    info: &AcpSessionInfo,
) -> Result<RunSummary, CoreError> {
    ensure_token(store, token)?;
    validate_id("runId", run_id)?;
    let event = RunEvent::AcpSession(info.normalized()?);
    // Comprueba el tamaño antes de abrir la transacción.
    event.to_payload()?;
    let (run_id, clock) = (run_id.to_owned(), store.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let run = require_run_any(conn, &run_id).await?;
                if !matches!(run.state, RunState::Preparing | RunState::Running) {
                    return Err(CoreError::Validation {
                        field: "runId",
                        reason: format!(
                            "La sesión ACP solo se registra mientras el intento se prepara o se \
                             ejecuta (está {}).",
                            run.state.label()
                        ),
                    });
                }
                append_event(conn, &run, &event, now).await?;
                require_run_any(conn, &run.id).await
            })
        })
        .await
}
