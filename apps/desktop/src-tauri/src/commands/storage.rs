use kanbania_core::error::ErrorPayload;
use kanbania_core::platform::storage::StorageStatus;
use tauri::{AppHandle, Runtime, State};

use crate::run_supervisor::start_run_supervisor;
use crate::service::start_local_service;
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
pub async fn retry_open_storage<R: Runtime>(
    handle: AppHandle<R>,
    slot: State<'_, StorageSlot>,
) -> Result<StorageStatus, ErrorPayload> {
    let status = slot.retry().await;
    // Si ahora esta instancia posee el almacén, publica el servicio local.
    start_local_service(&handle).await;
    start_run_supervisor(&handle).await;
    Ok(status)
}
