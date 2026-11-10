use tauri::{AppHandle, Runtime, State};

use crate::exit_guard::ExitGuard;

/// La UI informa si hay un borrador sin guardar.
#[tauri::command]
pub fn set_unsaved_changes(guard: State<'_, ExitGuard>, dirty: bool) {
    guard.set_dirty(dirty);
}

/// El usuario confirmó salir descartando los cambios.
#[tauri::command]
pub fn discard_and_close<R: Runtime>(app: AppHandle<R>, guard: State<'_, ExitGuard>) {
    guard.allow_exit();
    app.exit(0);
}
