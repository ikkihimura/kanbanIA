//! Servicio local de Kanban IA (HU-012): API HTTP solo en loopback, autenticada, que el host
//! de escritorio mantiene mientras la app está abierta. Llama a los mismos casos de uso del
//! núcleo que la UI y no abre su propia conexión SQLite: el almacén lo presta el host.
//!
//! El servidor va tras la feature `server` (por defecto). El cliente, que usarán las
//! integraciones (MCP en HU-013), se compila sin ella (`default-features = false`) y entonces
//! no enlaza ni el núcleo ni SQLite: no puede convertirse en otro propietario de los datos.

pub mod client;
pub mod descriptor;
pub mod protocol;
#[cfg(feature = "server")]
mod server;

pub use client::{ClientError, ServiceClient};
pub use protocol::{API_VERSION, MAX_BODY_BYTES, NONCE_HEADER, SESSION_HEADER};
#[cfg(feature = "server")]
pub use server::{LocalService, StoreSource};
