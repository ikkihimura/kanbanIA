//! Cliente ACP v1 propio (HU-019) para lanzar un agente compatible (hoy, Claude Code vía
//! `@agentclientprotocol/claude-agent-acp`) como proceso supervisado.
//!
//! Especificación: <https://agentclientprotocol.com/protocol/v1/> (initialization,
//! session-setup, session-modes, prompt-turn, tool-calls, cancellation, schema). Se usa un
//! subconjunto pequeño, sin el crate `agent-client-protocol` (decisión de diseño de HU-019):
//!
//! - transporte ([`framing`]): JSON-RPC 2.0 por líneas en stdin/stdout del proceso; línea
//!   entrante ≤ 1 MiB; JSON o JSON-RPC inválido → error de protocolo;
//! - negociación ([`AcpClient::initialize`], [`AcpClient::open_session`]): `initialize`
//!   (versión 1, `clientCapabilities: {}`: sin `fs` ni `terminal`), `session/new` con el
//!   worktree como `cwd` y sin servidores MCP, y **modo obligatorio** del perfil
//!   ([`AcpProfile`]: `default` en Claude Code, que pide permiso antes de actuar) con
//!   `session/set_mode {sessionId, modeId}`. Cada petición ≤ 60 s;
//! - turno ([`AcpClient::prompt`]): un único bloque `text` con las instrucciones de la
//!   instantánea; `session/update` → salida normalizada (`agent`, [`normalize`]); permisos →
//!   siempre la opción `reject_once` (o `cancelled` si no existe; nunca `allow_*` ni
//!   `reject_always`); otras peticiones del agente → `-32601`; otras notificaciones → solo su
//!   nombre en `sys`, una vez (el adaptador real envía el correo de la cuenta en
//!   `_auth/status_update`: nunca se copian parámetros);
//! - cancelación: `session/cancel` (notificación), los permisos que lleguen después se
//!   responden `cancelled`, y se espera la respuesta como mucho [`AcpLimits::cancel_wait`];
//!   antes de tener sesión no se negocia nada: quien llama mata el árbol.
//!
//! Un `current_mode_update` del agente a otro modo que el del perfil detiene el turno (falla
//! cerrado): el agente no puede salir por su cuenta del modo que pide permiso.
//!
//! Todo lo que envía el agente es dato no confiable. Los motivos que ve la persona los decide
//! el núcleo a partir de [`AcpFailure`]; los mensajes del agente van recortados a `sys`.

mod client;
pub(crate) mod framing;
#[cfg(all(test, feature = "supervisor"))]
mod guards;
pub(crate) mod jsonrpc;
pub(crate) mod normalize;
mod probe;

use std::time::Duration;

pub use client::{AcpClient, AgentDeclaration, OpenedSession};
pub use framing::MAX_LINE_BYTES;
pub use probe::{ProbeError, probe_adapter};

/// Versión de ACP que habla este cliente.
pub const PROTOCOL_VERSION: u16 = 1;

/// Perfil de un adaptador ACP: lo que la app exige además del protocolo. HU-021 añadirá otros
/// sin tocar el cliente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcpProfile {
    /// Modo de sesión que se fija con `session/set_mode` y que el agente no puede abandonar
    /// (ni con `current_mode_update` ni con `config_option_update` de la opción de modo).
    /// Si el agente no lo ofrece → [`AcpFailure::ModeUnavailable`].
    pub required_mode: &'static str,
    /// Claude Code: enviar en `session/new` el `_meta` de [`claude_code_isolation_meta`] para
    /// que no cargue la configuración del usuario, del proyecto ni local (sus reglas `allow`,
    /// hooks o `.mcp.json` evitarían `session/request_permission`).
    pub isolate_claude_code_settings: bool,
}

/// `_meta` de `session/new` que aísla Claude Code (adaptador `claude-agent-acp`, comprobado en
/// su `dist/new-session-meta.d.ts` 0.107.0: `_meta.claudeCode.options` son opciones del SDK
/// `@anthropic-ai/claude-agent-sdk`). `settingSources: []` = no cargar ajustes de usuario,
/// proyecto ni local (la política gestionada sí se aplica);
/// `allowDangerouslySkipPermissions: false` quita `bypassPermissions`.
pub fn claude_code_isolation_meta() -> serde_json::Value {
    serde_json::json!({
        "claudeCode": {
            "options": {
                "settingSources": [],
                "allowDangerouslySkipPermissions": false
            }
        }
    })
}

/// Línea `sys` fija cuando se pide el aislamiento.
pub const ISOLATION_NOTE: &str =
    "La app pidió no cargar la configuración de Claude Code (usuario, proyecto ni local).";

/// Claude Code (`@agentclientprotocol/claude-agent-acp`): `default` = «Always ask before making
/// changes» (comprobado con el adaptador real 0.107.0; su modo inicial es `auto`).
pub const CLAUDE_CODE_PROFILE: AcpProfile = AcpProfile {
    required_mode: "default",
    isolate_claude_code_settings: true,
};

/// Límites de tiempo del cliente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcpLimits {
    /// Cada petición de negociación (`initialize`, `session/new`, `session/set_mode`).
    pub request_timeout: Duration,
    /// Espera de la respuesta a `session/prompt` tras `session/cancel`.
    pub cancel_wait: Duration,
    /// Igual, cuando la app se está cerrando (para no bloquear la salida).
    pub shutdown_cancel_wait: Duration,
    /// Tras un fin normal: espera a que el adaptador termine solo al cerrar su stdin.
    pub close_wait: Duration,
    /// Escribir un mensaje en el stdin del agente (también compite con la cancelación y el
    /// tiempo máximo de la fase en curso).
    pub write_timeout: Duration,
}

impl Default for AcpLimits {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(60),
            cancel_wait: Duration::from_secs(10),
            shutdown_cancel_wait: Duration::from_secs(2),
            close_wait: Duration::from_secs(5),
            write_timeout: Duration::from_secs(5),
        }
    }
}

/// Por qué falló la sesión. El supervisor lo traduce a un motivo del núcleo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpFailure {
    /// El proceso cerró su salida (o terminó) antes de responder.
    Disconnected,
    /// JSON o JSON-RPC inválido, línea demasiado larga o respuesta con forma inesperada
    /// (incluido `stopReason: cancelled` sin que la app cancelara).
    Protocol,
    /// `initialize` respondió con otra versión de protocolo.
    VersionUnsupported,
    /// El agente exige autenticarse (la app nunca autentica).
    AuthRequired,
    /// No hay el modo que exige el perfil, no se pudo fijar, o el agente lo abandonó.
    ModeUnavailable,
    /// `stopReason: refusal`.
    Refused,
    /// `stopReason: max_tokens` o `max_turn_requests`.
    LimitReached,
    /// Error JSON-RPC (o sin respuesta a tiempo) a una petición de negociación de la app.
    RequestFailed,
    /// Error JSON-RPC a `session/prompt`: el turno no terminó.
    TurnFailed,
}

/// Por qué se detuvo una fase del cliente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpStop {
    /// La app pidió cancelar.
    Cancelled,
    /// Se agotó el tiempo máximo del intento.
    TimedOut,
    Failed(AcpFailure),
}
