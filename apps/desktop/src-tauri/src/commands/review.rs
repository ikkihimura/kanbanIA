//! Revisión desde la app de escritorio: el actor siempre es la persona (`Actor::Human`).
//! Los agentes solo llegarán por MCP (M2) y el núcleo les impide aceptar o decidir.

use kanbania_core::error::ErrorPayload;
use kanbania_core::features::review::{
    self, AcceptStoryInput, AttentionItem, Delivery, ReopenStoryInput, ReturnDeliveryInput,
    SetBlockerInput, StoryReview, SubmitDeliveryInput,
};
use kanbania_core::features::stories::StorySummary;
use kanbania_core::platform::actor::Actor;
use tauri::State;

use crate::storage::StorageSlot;

#[tauri::command]
pub async fn get_story_review(
    slot: State<'_, StorageSlot>,
    project_id: String,
    story_id: String,
) -> Result<StoryReview, ErrorPayload> {
    let store = slot.store().await?;
    review::get_story_review(&store, &project_id, &story_id)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn list_attention(
    slot: State<'_, StorageSlot>,
    project_id: String,
) -> Result<Vec<AttentionItem>, ErrorPayload> {
    let store = slot.store().await?;
    review::list_attention(&store, &project_id)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn submit_delivery(
    slot: State<'_, StorageSlot>,
    input: SubmitDeliveryInput,
) -> Result<Delivery, ErrorPayload> {
    let store = slot.store().await?;
    review::submit_delivery(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))
}

#[tauri::command]
pub async fn return_delivery(
    slot: State<'_, StorageSlot>,
    input: ReturnDeliveryInput,
) -> Result<(), ErrorPayload> {
    let store = slot.store().await?;
    review::return_delivery(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn accept_story(
    slot: State<'_, StorageSlot>,
    input: AcceptStoryInput,
) -> Result<StorySummary, ErrorPayload> {
    let store = slot.store().await?;
    review::accept_story(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn reopen_story(
    slot: State<'_, StorageSlot>,
    input: ReopenStoryInput,
) -> Result<StorySummary, ErrorPayload> {
    let store = slot.store().await?;
    review::reopen_story(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn set_story_blocker(
    slot: State<'_, StorageSlot>,
    input: SetBlockerInput,
) -> Result<(), ErrorPayload> {
    let store = slot.store().await?;
    review::set_story_blocker(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(None))
}
