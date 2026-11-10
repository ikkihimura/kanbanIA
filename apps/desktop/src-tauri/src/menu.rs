//! Menú de aplicación de macOS.
//!
//! El «Salir» predefinido llama a `terminate:` y macOS cierra el proceso sin pasar por
//! `ExitRequested`, así que no se podría proteger un borrador sin guardar. Por eso «Salir»
//! es un elemento propio (⌘Q) que pasa por `ExitGuard`. El resto son elementos predefinidos;
//! «Edición» es necesario para que copiar, pegar y deshacer funcionen en los campos de texto.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Runtime};

pub const QUIT_ID: &str = "app-quit";

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let quit = MenuItem::with_id(
        app,
        QUIT_ID,
        "Salir de Kanban IA",
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    let app_menu = Submenu::with_items(
        app,
        "Kanban IA",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("Acerca de Kanban IA"), None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some("Ocultar Kanban IA"))?,
            &PredefinedMenuItem::hide_others(app, Some("Ocultar otras"))?,
            &PredefinedMenuItem::show_all(app, Some("Mostrar todas"))?,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let edit_menu = Submenu::with_items(
        app,
        "Edición",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("Deshacer"))?,
            &PredefinedMenuItem::redo(app, Some("Rehacer"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("Cortar"))?,
            &PredefinedMenuItem::copy(app, Some("Copiar"))?,
            &PredefinedMenuItem::paste(app, Some("Pegar"))?,
            &PredefinedMenuItem::select_all(app, Some("Seleccionar todo"))?,
        ],
    )?;
    let window_menu = Submenu::with_items(
        app,
        "Ventana",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("Minimizar"))?,
            &PredefinedMenuItem::fullscreen(app, Some("Pantalla completa"))?,
            &PredefinedMenuItem::separator(app)?,
            // ⌘W: pasa por `CloseRequested`, que sí se puede cancelar.
            &PredefinedMenuItem::close_window(app, Some("Cerrar ventana"))?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit_menu, &window_menu])
}
