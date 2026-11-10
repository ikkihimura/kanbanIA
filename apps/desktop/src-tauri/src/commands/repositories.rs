use kanbania_core::error::ErrorPayload;
use kanbania_core::features::repositories::{
    self, LinkRepositoryInput, RelocateRepositoryInput, RepositoryLink, UnlinkRepositoryInput,
};
use tauri::State;

use super::UI_ACTOR;
use crate::storage::StorageSlot;

#[tauri::command]
pub async fn list_repositories(
    slot: State<'_, StorageSlot>,
    project_id: String,
) -> Result<Vec<RepositoryLink>, ErrorPayload> {
    let store = slot.store().await?;
    repositories::list_repositories(&store, &project_id)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn link_repository(
    slot: State<'_, StorageSlot>,
    input: LinkRepositoryInput,
) -> Result<RepositoryLink, ErrorPayload> {
    let store = slot.store().await?;
    repositories::link_repository(&store, UI_ACTOR, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))
}

#[tauri::command]
pub async fn relocate_repository(
    slot: State<'_, StorageSlot>,
    input: RelocateRepositoryInput,
) -> Result<RepositoryLink, ErrorPayload> {
    let store = slot.store().await?;
    repositories::relocate_repository(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn unlink_repository(
    slot: State<'_, StorageSlot>,
    input: UnlinkRepositoryInput,
) -> Result<(), ErrorPayload> {
    let store = slot.store().await?;
    repositories::unlink_repository(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}
