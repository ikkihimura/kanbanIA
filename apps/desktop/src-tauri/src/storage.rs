//! Propiedad del almacenamiento en el host. Si no se puede abrir (otra instancia, migración
//! fallida, base más reciente), la ventana arranca igualmente y muestra el error con opción
//! de reintentar; nunca se abre un segundo escritor.

use std::path::{Path, PathBuf};

use kanbania_core::error::CoreError;
use kanbania_core::platform::storage::{StorageStatus, Store};
use tauri::async_runtime::Mutex;

/// Permite usar otro directorio de datos (perfiles de prueba, depuración).
pub const DATA_DIR_ENV: &str = "KANBANIA_DATA_DIR";

enum SlotState {
    Ready(Store),
    Unavailable(StorageStatus),
    /// Cerrado al salir: no se reabre aunque llegue un reintento tardío.
    Closed,
}

pub struct StorageSlot {
    data_dir: PathBuf,
    state: Mutex<SlotState>,
}

impl StorageSlot {
    pub async fn open(data_dir: PathBuf) -> Self {
        let state = Self::try_open(&data_dir).await;
        Self {
            data_dir,
            state: Mutex::new(state),
        }
    }

    async fn try_open(data_dir: &Path) -> SlotState {
        match Store::open(data_dir).await {
            Ok(store) => SlotState::Ready(store),
            Err(error) => {
                log_open_error(&error);
                SlotState::Unavailable(StorageStatus::from_error(&error))
            }
        }
    }

    pub async fn status(&self) -> StorageStatus {
        Self::describe(&*self.state.lock().await).await
    }

    async fn describe(state: &SlotState) -> StorageStatus {
        match state {
            SlotState::Ready(store) => StorageStatus::of(store).await,
            SlotState::Unavailable(status) => status.clone(),
            SlotState::Closed => StorageStatus::from_error(&CoreError::StorageLocked),
        }
    }

    /// Reintenta abrir si no está disponible; si ya lo está, solo informa.
    pub async fn retry(&self) -> StorageStatus {
        let mut state = self.state.lock().await;
        if let SlotState::Unavailable(_) = &*state {
            *state = Self::try_open(&self.data_dir).await;
        }
        Self::describe(&state).await
    }

    /// Cierra las conexiones (checkpoint del WAL) y libera el bloqueo.
    pub async fn close(&self) {
        let mut state = self.state.lock().await;
        if let SlotState::Ready(store) = &*state {
            store.close().await;
        }
        *state = SlotState::Closed;
    }
}

fn log_open_error(error: &CoreError) {
    // Diagnóstico local en stderr con la cadena de causas; la UI recibe solo el mensaje seguro.
    let mut message = format!("[storage] {error}");
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    eprintln!("{message}");
}
