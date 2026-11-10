//! Host Tauri: adapta los casos de uso de `kanbania-core` a comandos IPC.
//! No contiene reglas de negocio.

mod commands;
mod storage;

use std::path::PathBuf;

pub use storage::StorageSlot;
use tauri::{App, Builder, Manager, RunEvent, Runtime};

/// Registra los comandos IPC. Lo usan el arranque real y las pruebas de contrato.
pub fn register_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        commands::app_info::get_app_info,
        commands::storage::get_storage_status,
        commands::storage::retry_open_storage,
        commands::projects::list_projects,
        commands::projects::create_project,
        commands::projects::rename_project,
        commands::projects::open_project,
        commands::projects::get_last_opened_project,
        commands::repositories::list_repositories,
        commands::repositories::link_repository,
        commands::repositories::relocate_repository,
        commands::repositories::unlink_repository,
    ])
}

fn resolve_data_dir<R: Runtime>(app: &App<R>) -> tauri::Result<PathBuf> {
    match std::env::var_os(storage::DATA_DIR_ENV) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => app.path().app_data_dir(),
    }
}

pub fn run() {
    let app = register_commands(Builder::default())
        // Solo el selector de carpetas: capability `dialog:allow-open`.
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = resolve_data_dir(app)?;
            let slot = tauri::async_runtime::block_on(StorageSlot::open(data_dir));
            app.manage(slot);
            Ok(())
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(error) => {
            eprintln!("Kanban IA no pudo iniciarse: {error}");
            std::process::exit(1);
        }
    };

    app.run(|handle, event| {
        if let RunEvent::Exit = event
            && let Some(slot) = handle.try_state::<StorageSlot>()
        {
            tauri::async_runtime::block_on(slot.close());
        }
    });
}
