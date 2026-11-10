use kanbania_core::error::ErrorPayload;
use kanbania_core::features::relations::{
    self, DependencyInput, SetStoryEpicInput, SetStoryParentInput, StoryRelations,
};
use tauri::State;

use crate::storage::StorageSlot;

#[tauri::command]
pub async fn get_story_relations(
    slot: State<'_, StorageSlot>,
    project_id: String,
    story_id: String,
) -> Result<StoryRelations, ErrorPayload> {
    let store = slot.store().await?;
    relations::get_story_relations(&store, &project_id, &story_id)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn set_story_epic(
    slot: State<'_, StorageSlot>,
    input: SetStoryEpicInput,
) -> Result<StoryRelations, ErrorPayload> {
    let store = slot.store().await?;
    relations::set_story_epic(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn set_story_parent(
    slot: State<'_, StorageSlot>,
    input: SetStoryParentInput,
) -> Result<StoryRelations, ErrorPayload> {
    let store = slot.store().await?;
    relations::set_story_parent(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn add_story_dependency(
    slot: State<'_, StorageSlot>,
    input: DependencyInput,
) -> Result<StoryRelations, ErrorPayload> {
    let store = slot.store().await?;
    relations::add_story_dependency(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn remove_story_dependency(
    slot: State<'_, StorageSlot>,
    input: DependencyInput,
) -> Result<StoryRelations, ErrorPayload> {
    let store = slot.store().await?;
    relations::remove_story_dependency(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}
