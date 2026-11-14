//! Intentos de ejecución desde la app (HU-017). El actor es siempre la persona. Registrar
//! eventos de un intento (`record_run_event`) no tiene comando: solo lo usará el supervisor.

use kanbania_core::error::ErrorPayload;
use kanbania_core::features::runs::{self, CancelRunInput, CreateRunInput, RunDetail, RunSummary};
use kanbania_core::platform::actor::Actor;
use tauri::State;

use crate::storage::StorageSlot;

#[tauri::command]
pub async fn create_run(
    slot: State<'_, StorageSlot>,
    input: CreateRunInput,
) -> Result<RunSummary, ErrorPayload> {
    let store = slot.store().await?;
    runs::create_run(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))
}

#[tauri::command]
pub async fn cancel_run(
    slot: State<'_, StorageSlot>,
    input: CancelRunInput,
) -> Result<RunSummary, ErrorPayload> {
    let store = slot.store().await?;
    runs::cancel_run(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))
}

#[tauri::command]
pub async fn list_story_runs(
    slot: State<'_, StorageSlot>,
    project_id: String,
    story_id: String,
) -> Result<Vec<RunSummary>, ErrorPayload> {
    let store = slot.store().await?;
    runs::list_story_runs(&store, &project_id, &story_id)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn get_run(
    slot: State<'_, StorageSlot>,
    project_id: String,
    run_id: String,
) -> Result<RunDetail, ErrorPayload> {
    let store = slot.store().await?;
    runs::get_run(&store, &project_id, &run_id)
        .await
        .map_err(|e| e.to_payload(None))
}
