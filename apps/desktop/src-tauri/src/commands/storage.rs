use kanbania_core::error::ErrorPayload;
use kanbania_core::platform::storage::StorageStatus;
use tauri::State;

use crate::storage::StorageSlot;

// Tauri exige `Result` en comandos asíncronos con argumentos prestados; el estado de error
// viaja dentro de `StorageStatus::Unavailable`.

#[tauri::command]
pub async fn get_storage_status(
    slot: State<'_, StorageSlot>,
) -> Result<StorageStatus, ErrorPayload> {
    Ok(slot.status().await)
}

#[tauri::command]
pub async fn retry_open_storage(
    slot: State<'_, StorageSlot>,
) -> Result<StorageStatus, ErrorPayload> {
    Ok(slot.retry().await)
}
