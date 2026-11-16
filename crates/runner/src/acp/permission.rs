//! Política de permisos del cliente ACP (HU-020).
//!
//! - [`PermissionPolicy::RejectAll`]: responde en el acto `reject_once` (política de HU-019;
//!   la usan las pruebas reales que no tienen a una persona delante).
//! - [`PermissionPolicy::Ask`]: cada `session/request_permission` queda **pendiente** y se
//!   envía a quien decide ([`PermissionEvent::Asked`]); el cliente sigue leyendo al agente y
//!   responde cuando llega la [`PermissionAnswer`]. El agente puede retirarla
//!   (`$/cancel_request`, [`PermissionEvent::Withdrawn`]). Al cancelar el turno, todas las
//!   pendientes se responden `cancelled`.
//!
//! Solo se responde con `allow_once` o `reject_once` del agente (o `cancelled`): nunca
//! `allow_always` ni `reject_always`, que guardarían reglas permanentes.

use tokio::sync::mpsc;

/// Pendientes a la vez por sesión: más → rechazo inmediato sin preguntar (igual que el tope
/// del núcleo, `MAX_PENDING_APPROVALS`).
pub const MAX_PENDING_PERMISSIONS: usize = 8;
/// Caracteres del alcance (comando o rutas) que se envían a quien decide.
pub const MAX_SCOPE_CHARS: usize = 2000;

/// Qué hace el cliente con las peticiones de permiso.
#[derive(Debug, Default)]
pub enum PermissionPolicy {
    /// Rechazar una vez, sin preguntar.
    #[default]
    RejectAll,
    /// Preguntar a quien decide por el puente.
    Ask(PermissionBridge),
}

/// Canales entre el cliente ACP y quien decide (el supervisor).
#[derive(Debug)]
pub struct PermissionBridge {
    pub events: mpsc::Sender<PermissionEvent>,
    pub answers: mpsc::Receiver<PermissionAnswer>,
}

/// Lo que el cliente avisa a quien decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionEvent {
    Asked(PermissionAsk),
    /// El agente retiró la petición: ya no espera respuesta.
    Withdrawn {
        key: u64,
    },
    /// El cliente respondió `cancelled` por su cuenta (cancelación del turno, fin de la sesión,
    /// sin quien decida): ninguna decisión posterior se aplica.
    Closed {
        key: u64,
    },
}

/// Petición del agente, ya limpia (una línea, sin formato invisible, recortada; el alcance con
/// correos y prefijos de secreto ocultos). Datos del agente: no confiables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionAsk {
    /// Clave local de la petición dentro de esta sesión.
    pub key: u64,
    pub title: String,
    /// `kind` de la herramienta (`edit`, `execute`…), si es un nombre ASCII válido.
    pub kind: Option<String>,
    /// Comando (`rawInput.command`) o rutas (`rawInput.file_path`/`path`, `locations`).
    pub scope: Option<String>,
    /// El agente ofreció `allow_once`.
    pub can_allow: bool,
}

/// Respuesta de quien decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionAnswer {
    pub key: u64,
    pub reply: PermissionReply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionReply {
    /// Una persona la permitió una vez → `allow_once`.
    AllowOnce,
    /// Una persona la rechazó → `reject_once`.
    Reject,
    /// Caducó sin decisión → `reject_once`.
    Expired,
    /// Se cerró sin decisión aplicable (cancelación, fin del intento) → `cancelled`.
    Cancelled,
    /// La app no pudo registrar la petición para preguntar → `reject_once` sin preguntar.
    Unrecorded,
}

/// Crea un puente: (extremo del cliente, extremo de quien decide).
pub fn permission_bridge() -> (
    PermissionBridge,
    mpsc::Receiver<PermissionEvent>,
    mpsc::Sender<PermissionAnswer>,
) {
    // Holgura: cada petición genera como mucho un `Asked` y un `Withdrawn`/`Closed`.
    let (events, events_rx) = mpsc::channel(MAX_PENDING_PERMISSIONS * 8);
    let (answers_tx, answers) = mpsc::channel(MAX_PENDING_PERMISSIONS * 2);
    (PermissionBridge { events, answers }, events_rx, answers_tx)
}
