use kanbania_core::error::ErrorPayload;
use kanbania_core::features::team::{
    self, Agent, CreateAgentInput, UpdateAgentInput, UpdateUserProfileInput, UserProfile,
};
use tauri::State;

use super::UI_ACTOR;
use crate::storage::StorageSlot;

#[tauri::command]
pub async fn get_user_profile(slot: State<'_, StorageSlot>) -> Result<UserProfile, ErrorPayload> {
    let store = slot.store().await?;
    team::get_user_profile(&store)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn update_user_profile(
    slot: State<'_, StorageSlot>,
    input: UpdateUserProfileInput,
) -> Result<UserProfile, ErrorPayload> {
    let store = slot.store().await?;
    team::update_user_profile(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn list_agents(slot: State<'_, StorageSlot>) -> Result<Vec<Agent>, ErrorPayload> {
    let store = slot.store().await?;
    team::list_agents(&store)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn create_agent(
    slot: State<'_, StorageSlot>,
    input: CreateAgentInput,
) -> Result<Agent, ErrorPayload> {
    let store = slot.store().await?;
    team::create_agent(&store, UI_ACTOR, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))
}

#[tauri::command]
pub async fn update_agent(
    slot: State<'_, StorageSlot>,
    input: UpdateAgentInput,
) -> Result<Agent, ErrorPayload> {
    let store = slot.store().await?;
    team::update_agent(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}
