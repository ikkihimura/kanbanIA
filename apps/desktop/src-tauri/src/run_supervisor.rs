//! Supervisor de ejecuciones (HU-018) mantenido por el host, igual que el servicio local: solo
//! arranca si esta instancia posee el almacén (setup, reintentar, restaurar). Al arrancar,
//! `kanbania_runner::supervisor::Supervisor::start` marca `interrupted` los intentos que
//! quedaron activos y anula las peticiones de inicio pendientes antes del primer despacho.
//! Se para (matando los árboles de procesos y marcando `interrupted` lo que quedara) antes de
//! restaurar una copia y al salir.
//!
//! El ejecutor es el proceso de prueba (sin IA): la propia app con `--kanbania-test-process`.

use std::path::PathBuf;
use std::sync::Arc;

use kanbania_core::features::runs::{InterruptCause, RunChange, SupervisorToken};
use kanbania_runner::supervisor::{
    ChangeSink, Supervisor, SupervisorConfig, SupervisorWaker, recover_previous_session,
};
use tauri::async_runtime::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::storage::StorageSlot;

/// Evento hacia la ventana principal tras un cambio del supervisor (carga: `RunChange`). La UI
/// recarga; el núcleo sigue siendo la autoridad.
pub const RUN_CHANGE_EVENT: &str = "run-change";

/// Ejecutable del proceso de prueba. Por defecto, la propia app (`current_exe`); las pruebas del
/// host lo sustituyen registrando este estado antes de arrancar. `None` simula que no se pudo
/// localizar el ejecutable (solo pruebas).
pub struct TestProcessProgram(pub Option<PathBuf>);

#[derive(Default)]
pub struct RunSupervisorSlot {
    running: Mutex<Option<Supervisor>>,
    waker: std::sync::Mutex<Option<SupervisorWaker>>,
    /// Tras salir no se vuelve a arrancar.
    stopped: Mutex<bool>,
    /// Mientras se restaura una copia no arranca (ni por un reintento ni por otra vía).
    suspended: Mutex<bool>,
}

impl RunSupervisorSlot {
    /// Arranca el supervisor si aún no lo está y el almacén está listo. Idempotente.
    pub async fn ensure_started<R: Runtime>(&self, handle: &AppHandle<R>) {
        let mut running = self.running.lock().await;
        if running.is_some() || *self.stopped.lock().await || *self.suspended.lock().await {
            return;
        }
        let Some(slot) = handle.try_state::<StorageSlot>() else {
            return;
        };
        let Ok(store) = slot.store().await else {
            return;
        };
        // El token primero: con él se recupera la sesión anterior aunque después no se pueda
        // arrancar el supervisor (sin ejecutable, por ejemplo).
        let token = match SupervisorToken::claim(&store) {
            Ok(token) => token,
            Err(error) => {
                eprintln!("[runs] supervisor no disponible: {:?}", error.code());
                return;
            }
        };
        let emitter = handle.clone();
        let sink: ChangeSink = Arc::new(move |change: RunChange| {
            // Solo un aviso para recargar: si la ventana no está, no pasa nada.
            if emitter.emit_to("main", RUN_CHANGE_EVENT, change).is_err() {
                eprintln!("[runs] no se pudo avisar a la ventana");
            }
        });
        let program = match handle.try_state::<TestProcessProgram>() {
            Some(program) => program.0.clone(),
            None => std::env::current_exe().ok(),
        };
        let Some(program) = program else {
            eprintln!("[runs] no se pudo localizar el ejecutable de la app");
            // Sin supervisor: aun así, los intentos de la sesión anterior pasan a
            // `interrupted` y sus peticiones se anulan; después se suelta el token.
            if let Err(error) = recover_previous_session(&store, &token, &sink).await {
                eprintln!(
                    "[runs] no se pudo recuperar la sesión anterior: {:?}",
                    error.code()
                );
            }
            return;
        };
        let config = SupervisorConfig::new(slot.data_dir().to_path_buf(), program);
        match Supervisor::start(store, token, config, sink).await {
            Ok(supervisor) => {
                if let Ok(mut waker) = self.waker.lock() {
                    *waker = Some(supervisor.waker());
                }
                *running = Some(supervisor);
            }
            Err(error) => eprintln!(
                "[runs] no se pudo arrancar el supervisor: {:?}",
                error.code()
            ),
        }
    }

    /// Despierta al supervisor (tras iniciar, cancelar, aceptar, archivar o cambiar la
    /// configuración). Sin supervisor no hace nada.
    pub fn wake(&self) {
        if let Ok(waker) = self.waker.lock()
            && let Some(waker) = waker.as_ref()
        {
            waker.wake();
        }
    }

    /// `true` si hay un supervisor en marcha (sin él, iniciar un intento no tendría efecto).
    pub async fn is_running(&self) -> bool {
        self.running.lock().await.is_some()
    }

    /// Antes de restaurar una copia: impide arrancar hasta [`Self::resume`], mata los árboles,
    /// marca `interrupted` («Se restauró una copia…») y suelta el almacén.
    pub async fn pause_for_restore(&self) {
        *self.suspended.lock().await = true;
        self.take_and_shutdown(InterruptCause::Restoring).await;
    }

    /// Tras restaurar (haya ido bien o mal): vuelve a permitir el arranque y arranca si el
    /// almacén está listo.
    pub async fn resume<R: Runtime>(&self, handle: &AppHandle<R>) {
        *self.suspended.lock().await = false;
        self.ensure_started(handle).await;
    }

    /// Para el supervisor definitivamente (al salir).
    pub async fn stop(&self) {
        *self.stopped.lock().await = true;
        self.take_and_shutdown(InterruptCause::AppExiting).await;
    }

    async fn take_and_shutdown(&self, cause: InterruptCause) {
        let supervisor = self.running.lock().await.take();
        if let Ok(mut waker) = self.waker.lock() {
            *waker = None;
        }
        if let Some(supervisor) = supervisor {
            supervisor.shutdown_for(cause).await;
        }
    }
}

/// Arranca el supervisor si procede (tras abrir, reintentar o restaurar el almacén).
pub async fn start_run_supervisor<R: Runtime>(handle: &AppHandle<R>) {
    if let Some(slot) = handle.try_state::<RunSupervisorSlot>() {
        slot.ensure_started(handle).await;
    }
}
