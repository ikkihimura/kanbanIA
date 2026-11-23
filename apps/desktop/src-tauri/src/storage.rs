//! Propiedad del almacenamiento en el host. Si no se puede abrir (otra instancia, migración
//! fallida, base más reciente), la ventana arranca igualmente y muestra el error con opción
//! de reintentar; nunca se abre un segundo escritor.

use std::path::{Path, PathBuf};

use kanbania_core::error::{CoreError, ErrorPayload};
use std::time::Duration;

use kanbania_core::platform::storage::{
    BACKUP_DIR, RestoreOutcome, StorageStatus, Store, restore_backup_waiting,
};
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
            Ok(store) => {
                // Retención de los recibos de idempotencia (HU-025): fuera de su ventana. Si
                // falla, se intenta en el próximo arranque; la app sigue.
                if let Err(error) = store.prune_mutation_receipts().await {
                    eprintln!("[almacén] no se pudieron borrar recibos antiguos: {error:?}");
                }
                SlotState::Ready(store)
            }
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

    /// Almacén listo para un comando, o el error que la UI debe mostrar.
    /// `Store` es un manejador barato (pool compartido); no debe guardarse más allá del comando.
    pub async fn store(&self) -> Result<Store, ErrorPayload> {
        match &*self.state.lock().await {
            SlotState::Ready(store) => Ok(store.clone()),
            SlotState::Unavailable(StorageStatus::Unavailable { error }) => Err(error.clone()),
            SlotState::Unavailable(StorageStatus::Ready { .. }) | SlotState::Closed => {
                Err(CoreError::StorageLocked.to_payload(None))
            }
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

    /// `true` si esta instancia posee el almacén abierto.
    pub async fn is_ready(&self) -> bool {
        matches!(&*self.state.lock().await, SlotState::Ready(_))
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Restaura una copia (HU-011). Mientras dura, el estado queda bloqueado: los comandos que
    /// piden el almacén esperan y las conexiones antiguas se cierran, así que nadie escribe
    /// durante la sustitución. Si la base restaurada no abre (p. ej. falla su migración), se
    /// vuelve a la copia previa automáticamente y se dice. Ante cualquier error, se reabre lo
    /// que hubiera.
    pub async fn restore(&self, source: &Path) -> Result<RestoreOutcome, ErrorPayload> {
        let mut state = self.state.lock().await;
        if let SlotState::Closed = &*state {
            return Err(CoreError::StorageLocked.to_payload(None));
        }
        if let SlotState::Ready(store) = &*state {
            store.close().await;
        }
        *state = SlotState::Unavailable(StorageStatus::from_error(&CoreError::StorageLocked));

        // Un comando en curso aún puede tener su copia del almacén (y del bloqueo): con el pool
        // cerrado termina enseguida. Se espera un plazo corto antes de dar por bloqueado.
        let outcome = match restore_waiting(&self.data_dir, source).await {
            Ok(outcome) => outcome,
            Err(error) => {
                *state = Self::try_open(&self.data_dir).await;
                return Err(error.to_payload(None));
            }
        };
        *state = Self::try_open(&self.data_dir).await;
        if let SlotState::Unavailable(StorageStatus::Unavailable { error }) = &*state {
            let reason = error.message.clone();
            let Some(safety) = &outcome.safety_backup else {
                return Err(error.clone());
            };
            let undo = self.data_dir.join(BACKUP_DIR).join(&safety.id);
            let undone = restore_waiting(&self.data_dir, &undo).await.is_ok();
            if undone {
                *state = Self::try_open(&self.data_dir).await;
            }
            let message = if undone && matches!(&*state, SlotState::Ready(_)) {
                format!(
                    "La copia se restauró pero no se pudo abrir ({reason}). Se volvió \
                     automáticamente al estado anterior; no se perdió nada."
                )
            } else {
                format!(
                    "La copia se restauró pero no se pudo abrir ({reason}) y no se pudo volver \
                     al estado anterior. Tus datos anteriores están en la copia «{}»: \
                     restáurala desde la lista.",
                    safety.id
                )
            };
            return Err(CoreError::Validation {
                field: "backup",
                reason: message,
            }
            .to_payload(None));
        }
        Ok(outcome)
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

/// Plazo para que un comando en curso suelte su manejador del almacén antes de restaurar.
const LOCK_WAIT: Duration = Duration::from_secs(3);

async fn restore_waiting(data_dir: &Path, source: &Path) -> Result<RestoreOutcome, CoreError> {
    restore_backup_waiting(data_dir, source, LOCK_WAIT).await
}
