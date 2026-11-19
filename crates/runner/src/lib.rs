//! Ejecución local de intentos (HU-018). Las piezas de bajo nivel no dependen del núcleo; solo
//! [`supervisor`] lo usa:
//!
//! - [`acp`]: cliente ACP v1 propio (HU-019) para agentes compatibles (Claude Code) lanzados
//!   en modo protocolo;
//! - [`process`]: lanza un programa con argumentos explícitos (nunca un shell), en su propio
//!   grupo de procesos (Unix) o Job Object (Windows), con entorno limpio, límite de tiempo y
//!   cancelación del árbol completo;
//! - [`output`]: salida del proceso en JSONL acotado y rotado, y lectura de la cola para la UI;
//! - [`worktree`]: crear, inspeccionar y eliminar worktrees con la CLI de Git, sin tocar nunca
//!   el árbol de trabajo del repositorio original;
//! - [`guard`]: guardián que termina el árbol de un intento si la app muere de golpe (Unix);
//! - [`test_process`]: el proceso de prueba incluido («Proceso de prueba (sin IA)»);
//! - [`supervisor`]: orquesta el núcleo (`runs::supervisor`) y los procesos.
//!
//! Todo lo que produce un proceso o Git es dato no confiable: los mensajes de error que se
//! muestran son fijos y seguros; los detalles (rutas, salida de Git) van aparte, para el log.

pub mod acp;
mod env;
pub mod guard;
pub mod output;
pub mod process;
mod redact;
#[cfg(feature = "supervisor")]
pub mod supervisor;
pub mod test_process;
mod tree;
pub mod worktree;

pub use env::{adapter_env_names, is_adapter_forbidden, is_reserved_env_name};
