use kanbania_core::error::ErrorPayload;
use kanbania_core::features::columns::{
    self, Column, CreateColumnInput, DeleteColumnInput, ReorderColumnInput, UpdateColumnInput,
};
use tauri::State;

use super::UI_ACTOR;
use crate::storage::StorageSlot;

#[tauri::command]
pub async fn list_columns(
    slot: State<'_, StorageSlot>,
    project_id: String,
) -> Result<Vec<Column>, ErrorPayload> {
    let store = slot.store().await?;
    columns::list_columns(&store, &project_id)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn create_column(
    slot: State<'_, StorageSlot>,
    input: CreateColumnInput,
) -> Result<Column, ErrorPayload> {
    let store = slot.store().await?;
    columns::create_column(&store, UI_ACTOR, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))
}

#[tauri::command]
pub async fn update_column(
    slot: State<'_, StorageSlot>,
    input: UpdateColumnInput,
) -> Result<Column, ErrorPayload> {
    let store = slot.store().await?;
    columns::update_column(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn reorder_column(
    slot: State<'_, StorageSlot>,
    input: ReorderColumnInput,
) -> Result<Column, ErrorPayload> {
    let store = slot.store().await?;
    columns::reorder_column(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn delete_column(
    slot: State<'_, StorageSlot>,
    input: DeleteColumnInput,
) -> Result<(), ErrorPayload> {
    let store = slot.store().await?;
    columns::delete_column(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}
