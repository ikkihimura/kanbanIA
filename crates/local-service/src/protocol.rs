//! Protocolo compartido por servidor y cliente. Sin dependencias del núcleo: el cliente (y el
//! adaptador MCP que lo use) no enlaza el código que abre SQLite.

use sha2::{Digest, Sha256};

use crate::descriptor::{Secret, hex};

/// Versión de la API (prefijo de rutas `/v1`). Tipos servidos: `SessionInfo`, `ReadyPage`,
/// `StoryContext` y `StoryClaim`; cuerpos: `ClaimStoryInput` y `{requestId}` al renovar. v1 no se congela hasta publicar M2: hasta entonces, un cambio incompatible
/// se hace en el mismo commit en servidor, cliente y `kanbania-mcp`. Después, retirar o
/// renombrar una ruta o un campo exige subirla.
pub const API_VERSION: u32 = 1;
/// Cabecera con el token de sesión del descriptor.
pub const SESSION_HEADER: &str = "x-kanbania-session";
/// Cabecera del saludo: reto del cliente para que el servidor demuestre conocer el token.
pub const NONCE_HEADER: &str = "x-kanbania-nonce";
/// Cuerpo máximo de una petición. Los cuerpos son JSON pequeños (reclamos; en M2, entregas y
/// bloqueos).
pub const MAX_BODY_BYTES: usize = 64 * 1024;
const PROOF_DOMAIN: &str = "kanbania-service-proof/v1";

/// Respuesta al reto del saludo: `sha256(dominio ‖ token de sesión ‖ reto)`.
pub fn proof(session_token: &Secret, nonce: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(PROOF_DOMAIN.as_bytes());
    hasher.update([0]);
    hasher.update(session_token.expose().as_bytes());
    hasher.update([0]);
    hasher.update(nonce.as_bytes());
    hex(&hasher.finalize())
}
