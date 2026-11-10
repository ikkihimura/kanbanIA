use kanbania_core::error::ErrorPayload;
use kanbania_core::features::projects::{
    self, CreateProjectInput, Project, ProjectSummary, RenameProjectInput,
};
use tauri::State;

use super::UI_ACTOR;
use crate::storage::StorageSlot;

#[tauri::command]
pub async fn list_projects(
    slot: State<'_, StorageSlot>,
) -> Result<Vec<ProjectSummary>, ErrorPayload> {
    let store = slot.store().await?;
    projects::list_projects(&store)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn create_project(
    slot: State<'_, StorageSlot>,
    input: CreateProjectInput,
) -> Result<Project, ErrorPayload> {
    let store = slot.store().await?;
    projects::create_project(&store, UI_ACTOR, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))
}

#[tauri::command]
pub async fn rename_project(
    slot: State<'_, StorageSlot>,
    input: RenameProjectInput,
) -> Result<Project, ErrorPayload> {
    let store = slot.store().await?;
    projects::rename_project(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn open_project(
    slot: State<'_, StorageSlot>,
    id: String,
) -> Result<Project, ErrorPayload> {
    let store = slot.store().await?;
    projects::open_project(&store, &id)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn get_last_opened_project(
    slot: State<'_, StorageSlot>,
) -> Result<Option<Project>, ErrorPayload> {
    let store = slot.store().await?;
    projects::last_opened_project(&store)
        .await
        .map_err(|e| e.to_payload(None))
}
