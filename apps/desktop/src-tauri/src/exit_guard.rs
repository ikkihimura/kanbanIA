//! Protección de cambios sin guardar al cerrar la ventana o salir de la app.
//!
//! La UI informa si hay un borrador sin guardar (`set_unsaved_changes`). Mientras lo haya,
//! el host cancela el cierre (botón de cerrar, ⌘W, Alt+F4) y la salida (⌘Q vía el menú
//! propio) y emite [`CONFIRM_CLOSE_EVENT`] para que la UI pregunte. Si el usuario decide
//! descartar, la UI llama a `discard_and_close` y la salida continúa.

use std::sync::atomic::{AtomicBool, Ordering};

/// Evento hacia la ventana principal: "se pidió cerrar con cambios sin guardar".
pub const CONFIRM_CLOSE_EVENT: &str = "unsaved-changes:confirm-close";

#[derive(Debug, Default)]
pub struct ExitGuard {
    dirty: AtomicBool,
    allowed: AtomicBool,
}

impl ExitGuard {
    pub fn set_dirty(&self, dirty: bool) {
        self.dirty.store(dirty, Ordering::SeqCst);
    }

    /// `true` si hay que cancelar el cierre y preguntar.
    pub fn should_block(&self) -> bool {
        self.dirty.load(Ordering::SeqCst) && !self.allowed.load(Ordering::SeqCst)
    }

    /// El usuario confirmó descartar: los siguientes cierres no se bloquean.
    pub fn allow_exit(&self) {
        self.allowed.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_only_with_unsaved_changes_until_confirmed() {
        let guard = ExitGuard::default();
        assert!(!guard.should_block(), "sin cambios se cierra normalmente");

        guard.set_dirty(true);
        assert!(guard.should_block());

        guard.set_dirty(false);
        assert!(!guard.should_block(), "tras guardar ya no bloquea");

        guard.set_dirty(true);
        guard.allow_exit();
        assert!(!guard.should_block(), "el usuario confirmó descartar");
    }
}
