//! Exportar e importar paquetes Markdown (HU-010). Las rutas llegan del selector de carpetas
//! de la propia app; el actor es siempre la persona.

use kanbania_core::error::ErrorPayload;
use kanbania_core::features::portability::{
    self, ApplyImportInput, ExportProjectInput, ExportResult, ImportPreview, ImportResult,
};
use kanbania_core::platform::actor::Actor;
use tauri::{AppHandle, Runtime, State};

use crate::storage::StorageSlot;

#[tauri::command]
pub async fn export_project<R: Runtime>(
    app: AppHandle<R>,
    slot: State<'_, StorageSlot>,
    input: ExportProjectInput,
) -> Result<ExportResult, ErrorPayload> {
    let store = slot.store().await?;
    let version = app.package_info().version.to_string();
    portability::export_project(&store, &Actor::Human, &input, &version)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn preview_import(
    slot: State<'_, StorageSlot>,
    path: String,
) -> Result<ImportPreview, ErrorPayload> {
    let store = slot.store().await?;
    portability::preview_import(&store, &Actor::Human, &path)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn apply_import(
    slot: State<'_, StorageSlot>,
    input: ApplyImportInput,
) -> Result<ImportResult, ErrorPayload> {
    let store = slot.store().await?;
    portability::apply_import(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(None))
}
