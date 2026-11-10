use kanbania_core::error::ErrorPayload;
use kanbania_core::features::stories::{
    self, ArchiveStoryInput, CreateStoryInput, RestoreStoryInput, StoryDetail, StorySummary,
    UpdateStoryInput,
};
use tauri::State;

use super::UI_ACTOR;
use crate::storage::StorageSlot;

#[tauri::command]
pub async fn list_stories(
    slot: State<'_, StorageSlot>,
    project_id: String,
    include_archived: bool,
) -> Result<Vec<StorySummary>, ErrorPayload> {
    let store = slot.store().await?;
    stories::list_stories(&store, &project_id, include_archived)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn get_story(
    slot: State<'_, StorageSlot>,
    project_id: String,
    id: String,
) -> Result<StoryDetail, ErrorPayload> {
    let store = slot.store().await?;
    stories::get_story(&store, &project_id, &id)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn create_story(
    slot: State<'_, StorageSlot>,
    input: CreateStoryInput,
) -> Result<StorySummary, ErrorPayload> {
    let store = slot.store().await?;
    stories::create_story(&store, UI_ACTOR, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))
}

#[tauri::command]
pub async fn update_story(
    slot: State<'_, StorageSlot>,
    input: UpdateStoryInput,
) -> Result<StoryDetail, ErrorPayload> {
    let store = slot.store().await?;
    stories::update_story(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn archive_story(
    slot: State<'_, StorageSlot>,
    input: ArchiveStoryInput,
) -> Result<StorySummary, ErrorPayload> {
    let store = slot.store().await?;
    stories::archive_story(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn restore_story(
    slot: State<'_, StorageSlot>,
    input: RestoreStoryInput,
) -> Result<StorySummary, ErrorPayload> {
    let store = slot.store().await?;
    stories::restore_story(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

/// Mueve una tarjeta en el tablero (entre columnas o dentro de una) de forma atómica.
/// Entrar en «hecho» devuelve `acceptance_required`: la UI abre entonces el flujo de aceptar.
#[tauri::command]
pub async fn move_story(
    slot: State<'_, StorageSlot>,
    input: kanbania_core::features::stories::MoveStoryInput,
) -> Result<StorySummary, ErrorPayload> {
    let store = slot.store().await?;
    stories::move_story(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}
