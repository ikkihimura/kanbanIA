//! Host Tauri: adapta los casos de uso de `kanbania-core` a comandos IPC.
//! No contiene reglas de negocio.

mod commands;
mod exit_guard;
#[cfg(target_os = "macos")]
mod menu;
mod run_supervisor;
mod service;
mod storage;

use std::path::PathBuf;

pub use exit_guard::{CONFIRM_CLOSE_EVENT, ExitGuard};
pub use run_supervisor::{
    RUN_CHANGE_EVENT, RunSupervisorSlot, TestProcessProgram, start_run_supervisor,
};
pub use service::{ServiceSlot, start_local_service};
pub use storage::StorageSlot;
use tauri::{App, AppHandle, Builder, Emitter, Manager, RunEvent, Runtime, WindowEvent};

/// Registra los comandos IPC. Lo usan el arranque real y las pruebas de contrato.
pub fn register_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder
        .manage(ExitGuard::default())
        .manage(ServiceSlot::default())
        .manage(RunSupervisorSlot::default())
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
            commands::access::list_agent_credentials,
            commands::access::create_agent_credential,
            commands::access::revoke_agent_credential,
            commands::access::get_local_service_status,
            commands::claims::list_active_claims,
            commands::claims::release_claim,
            commands::agent_activity::list_story_activity,
            commands::runs::create_run,
            commands::runs::cancel_run,
            commands::runs::decide_approval,
            commands::runs::list_waiting_approvals,
            commands::runs::list_story_runs,
            commands::runs::get_run,
            commands::runs::start_run,
            commands::runs::get_run_output,
            commands::runs::get_worktree_changes,
            commands::runs::remove_run_worktree,
            commands::runs::get_run_settings,
            commands::runs::update_run_settings,
            commands::runs::check_acp_adapter,
        ])
}

fn resolve_data_dir<R: Runtime>(app: &App<R>) -> tauri::Result<PathBuf> {
    match std::env::var_os(storage::DATA_DIR_ENV) {
        Some(dir) => {
            // Las rutas de worktrees y salida de intentos se derivan de aquí: una relativa
            // dependería del directorio actual. Se rechaza en vez de adivinar.
            let dir = PathBuf::from(dir);
            if dir.is_absolute() {
                Ok(dir)
            } else {
                Err(tauri::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    DATA_DIR_NOT_ABSOLUTE,
                )))
            }
        }
        None => app.path().app_data_dir(),
    }
}

/// Mensaje si `KANBANIA_DATA_DIR` no es absoluta (la app no arranca).
const DATA_DIR_NOT_ABSOLUTE: &str =
    "KANBANIA_DATA_DIR debe ser una ruta absoluta (p. ej. /Users/tu-usuario/kanbania-pruebas).";

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
        .plugin(tauri_plugin_dialog::init())
        // Solo se usa desde Rust (aviso de permisos, HU-020): la capability no da permisos JS.
        .plugin(tauri_plugin_notification::init());

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
            // Solo si esta instancia posee el almacén (una segunda instancia no lo arranca).
            tauri::async_runtime::block_on(start_local_service(app.handle()));
            // Igual: solo con el almacén propio. Antes del primer despacho interrumpe lo que
            // quedó activo y anula las peticiones de inicio pendientes.
            tauri::async_runtime::block_on(start_run_supervisor(app.handle()));
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
            // Primero los procesos (y marcar `interrupted`), con el almacén aún abierto.
            if let Some(runs) = handle.try_state::<RunSupervisorSlot>() {
                tauri::async_runtime::block_on(runs.stop());
            }
            if let Some(service) = handle.try_state::<ServiceSlot>() {
                tauri::async_runtime::block_on(service.stop());
            }
            if let Some(slot) = handle.try_state::<StorageSlot>() {
                tauri::async_runtime::block_on(slot.close());
            }
        }
        _ => {}
    });
}
