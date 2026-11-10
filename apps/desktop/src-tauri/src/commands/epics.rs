use kanbania_core::error::ErrorPayload;
use kanbania_core::features::epics::{self, CreateEpicInput, Epic, UpdateEpicInput};
use tauri::State;

use super::UI_ACTOR;
use crate::storage::StorageSlot;

#[tauri::command]
pub async fn list_epics(
    slot: State<'_, StorageSlot>,
    project_id: String,
) -> Result<Vec<Epic>, ErrorPayload> {
    let store = slot.store().await?;
    epics::list_epics(&store, &project_id)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn create_epic(
    slot: State<'_, StorageSlot>,
    input: CreateEpicInput,
) -> Result<Epic, ErrorPayload> {
    let store = slot.store().await?;
    epics::create_epic(&store, UI_ACTOR, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))
}

#[tauri::command]
pub async fn update_epic(
    slot: State<'_, StorageSlot>,
    input: UpdateEpicInput,
) -> Result<Epic, ErrorPayload> {
    let store = slot.store().await?;
    epics::update_epic(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}
