//! Host Tauri: adapta los casos de uso de `kanbania-core` a comandos IPC.
//! No contiene reglas de negocio.

mod commands;
mod exit_guard;
#[cfg(target_os = "macos")]
mod menu;
mod storage;

use std::path::PathBuf;

pub use exit_guard::{CONFIRM_CLOSE_EVENT, ExitGuard};
pub use storage::StorageSlot;
use tauri::{App, AppHandle, Builder, Emitter, Manager, RunEvent, Runtime, WindowEvent};

/// Registra los comandos IPC. Lo usan el arranque real y las pruebas de contrato.
pub fn register_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder
        .manage(ExitGuard::default())
        .invoke_handler(tauri::generate_handler![
            commands::exit::set_unsaved_changes,
            commands::exit::discard_and_close,
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
            commands::columns::list_columns,
            commands::columns::create_column,
            commands::columns::update_column,
            commands::columns::reorder_column,
            commands::columns::delete_column,
            commands::stories::list_stories,
            commands::stories::get_story,
            commands::stories::create_story,
            commands::stories::update_story,
            commands::stories::archive_story,
            commands::stories::restore_story,
            commands::stories::move_story,
            commands::epics::list_epics,
            commands::epics::create_epic,
            commands::epics::update_epic,
            commands::relations::get_story_relations,
            commands::relations::set_story_epic,
            commands::relations::set_story_parent,
            commands::relations::add_story_dependency,
            commands::relations::remove_story_dependency,
            commands::relations::set_story_agent,
            commands::team::get_user_profile,
            commands::team::update_user_profile,
            commands::team::list_agents,
            commands::team::create_agent,
            commands::team::update_agent,
            commands::review::get_story_review,
            commands::review::list_attention,
            commands::review::submit_delivery,
            commands::review::return_delivery,
            commands::review::accept_story,
            commands::review::reopen_story,
            commands::review::set_story_blocker,
            commands::portability::export_project,
            commands::portability::preview_import,
            commands::portability::apply_import,
            commands::backup::list_backups,
            commands::backup::create_backup,
            commands::backup::save_backup_to_folder,
            commands::backup::restore_backup,
        ])
}

fn resolve_data_dir<R: Runtime>(app: &App<R>) -> tauri::Result<PathBuf> {
    match std::env::var_os(storage::DATA_DIR_ENV) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => app.path().app_data_dir(),
    }
}

/// Cancela el cierre y pide confirmación a la UI si hay cambios sin guardar.
fn should_block_and_ask<R: Runtime>(handle: &AppHandle<R>) -> bool {
    let blocked = handle
        .try_state::<ExitGuard>()
        .is_some_and(|guard| guard.should_block());
    if blocked && let Err(error) = handle.emit_to("main", CONFIRM_CLOSE_EVENT, ()) {
        eprintln!("No se pudo pedir confirmación de cierre: {error}");
    }
    blocked
}

pub fn run() {
    let builder = register_commands(Builder::default())
        // Solo el selector de carpetas: capability `dialog:allow-open`.
        .plugin(tauri_plugin_dialog::init());

    // macOS: «Salir» (⌘Q) propio para que pase por `ExitGuard` (ver `menu.rs`).
    #[cfg(target_os = "macos")]
    let builder = builder.menu(menu::build).on_menu_event(|handle, event| {
        if event.id() == menu::QUIT_ID && !should_block_and_ask(handle) {
            handle.exit(0);
        }
    });

    let app = builder
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

    app.run(|handle, event| match event {
        // Botón de cerrar, ⌘W, Alt+F4.
        RunEvent::WindowEvent {
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } => {
            if should_block_and_ask(handle) {
                api.prevent_close();
            }
        }
        // Salida pedida por el sistema o el usuario (no por `app.exit`, que trae código).
        RunEvent::ExitRequested {
            code: None, api, ..
        } => {
            if should_block_and_ask(handle) {
                api.prevent_exit();
            }
        }
        RunEvent::Exit => {
            if let Some(slot) = handle.try_state::<StorageSlot>() {
                tauri::async_runtime::block_on(slot.close());
            }
        }
        _ => {}
    });
}
