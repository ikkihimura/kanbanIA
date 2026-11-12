//! Copias de seguridad y restauración (HU-011). Funcionan aunque el almacenamiento no se haya
//! podido abrir (p. ej. tras una migración fallida), para poder recuperar datos.

use std::path::PathBuf;

use kanbania_core::error::{CoreError, ErrorPayload};
use kanbania_core::platform::storage::{
    self, BackupInfo, BackupKind, RestoreOutcome, RestoreSource, backup_path, save_backup_copy,
};
use tauri::State;

use crate::storage::StorageSlot;

/// Operaciones de disco fuera del hilo asíncrono.
async fn blocking<T: Send + 'static>(
    task: impl FnOnce() -> Result<T, CoreError> + Send + 'static,
) -> Result<T, ErrorPayload> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| {
            CoreError::StorageIo(std::io::Error::other("tarea interrumpida")).to_payload(None)
        })?
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn list_backups(slot: State<'_, StorageSlot>) -> Result<Vec<BackupInfo>, ErrorPayload> {
    let data_dir = slot.data_dir().to_path_buf();
    blocking(move || storage::list_backups(&data_dir)).await
}

#[tauri::command]
pub async fn create_backup(slot: State<'_, StorageSlot>) -> Result<BackupInfo, ErrorPayload> {
    let store = slot.store().await?;
    store
        .create_backup(BackupKind::Manual)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn save_backup_to_folder(
    slot: State<'_, StorageSlot>,
    id: String,
    destination: String,
) -> Result<String, ErrorPayload> {
    let data_dir = slot.data_dir().to_path_buf();
    blocking(move || save_backup_copy(&data_dir, &id, &PathBuf::from(destination))).await
}

#[tauri::command]
pub async fn restore_backup(
    slot: State<'_, StorageSlot>,
    source: RestoreSource,
) -> Result<RestoreOutcome, ErrorPayload> {
    let path = match source {
        RestoreSource::Backup { id } => {
            backup_path(slot.data_dir(), &id).map_err(|e| e.to_payload(None))?
        }
        RestoreSource::Folder { path } => {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(CoreError::Validation {
                    field: "backup",
                    reason: "Elige la carpeta de la copia.".into(),
                }
                .to_payload(None));
            }
            path
        }
    };
    slot.restore(&path).await
}
