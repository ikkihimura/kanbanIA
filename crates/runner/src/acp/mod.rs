//! Cliente ACP v1 propio (HU-019) para lanzar un agente compatible (Claude Code vía
//! `@agentclientprotocol/claude-agent-acp`; desde HU-021, Codex vía
//! `@agentclientprotocol/codex-acp`) como proceso supervisado. Lo propio de cada agente vive en
//! su [`AcpProfile`] ([`CLAUDE_CODE_PROFILE`], [`CODEX_PROFILE`]); el cliente es genérico.
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
//!   ([`AcpProfile`]: `default` en Claude Code, `read-only` en Codex) con
//!   `session/set_mode {sessionId, modeId}`. Cada petición ≤ 60 s;
//! - turno ([`AcpClient::prompt`]): un único bloque `text` con las instrucciones de la
//!   instantánea; `session/update` → salida normalizada (`agent`, [`normalize`]); permisos →
//!   según la [`PermissionPolicy`]: rechazar en el acto (`reject_once`) o preguntar a una
//!   persona (HU-020) y responder `allow_once`/`reject_once` cuando decide (`cancelled` si no
//!   hay `reject_once`, si se cancela el turno o si se cierra sin decisión; nunca
//!   `allow_always` ni `reject_always`; el perfil excluye opciones `allow_once` que valen para
//!   más que una acción y prefiere un `reject_once` que no termine el turno);
//!   `$/cancel_request` del agente retira una pendiente;
//!   otras peticiones del agente → `-32601`; otras notificaciones → solo su
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
mod confine;
pub(crate) mod framing;
#[cfg(all(test, feature = "supervisor"))]
mod guards;
pub(crate) mod jsonrpc;
mod node;
pub(crate) mod normalize;
mod permission;
mod probe;
mod profile;

use std::time::Duration;

pub use client::{AcpClient, AgentDeclaration, OpenedSession, TurnEnd};
pub use confine::{
    CLAUDE_SANDBOX_AVAILABLE, CLAUDE_SANDBOX_NOTE, CLAUDE_UNCONFINED_NOTE, CODEX_CONFINEMENT_NOTE,
    CODEX_PERMISSIONS_PROFILE, DATA_DIR_SANDBOX_PROFILE, DataConfinement, SANDBOX_EXEC,
    codex_config,
};
pub use framing::MAX_LINE_BYTES;
pub use node::{configured_node_unusable, effective_path, missing_node, node_path_env};
pub use permission::{
    MAX_PENDING_PERMISSIONS, MAX_SCOPE_CHARS, PermissionAnswer, PermissionAsk, PermissionBridge,
    PermissionEvent, PermissionPolicy, PermissionReply, permission_bridge,
};
pub use probe::{ProbeError, probe_adapter};
pub use profile::{
    AcpProfile, CLAUDE_CODE_PROFILE, CODEX_CONFIG, CODEX_HOME_DENYLIST, CODEX_HOME_NOTE,
    CODEX_PERSONAL_DENYLIST, CODEX_PERSONAL_HOME, CODEX_PLUGIN_CACHES, CODEX_PROFILE,
    CODEX_REPOSITORY_CONFIG, ISOLATION_NOTE, SessionIsolation, claude_code_isolation_meta,
    clear_codex_plugin_cache, codex_env, codex_home_config, ensure_codex_home,
    has_repository_config,
};

/// Versión de ACP que habla este cliente.
pub const PROTOCOL_VERSION: u16 = 1;

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
    /// (incluido `stopReason: cancelled` sin que la app cancelara ni rechazara un permiso con
    /// una respuesta que termina el turno).
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
