//! Host Tauri: adapta los casos de uso de `kanbania-core` a comandos IPC.
//! No contiene reglas de negocio.

mod commands;

use tauri::{Builder, Runtime};

/// Registra los comandos IPC. Lo usan el arranque real y las pruebas de contrato.
pub fn register_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![commands::app_info::get_app_info])
}

pub fn run() {
    let result = register_commands(Builder::default()).run(tauri::generate_context!());

    if let Err(error) = result {
        eprintln!("Kanban IA no pudo iniciarse: {error}");
        std::process::exit(1);
    }
}
