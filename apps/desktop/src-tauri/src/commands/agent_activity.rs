//! Actividad reportada por agentes (HU-015), para la app. Los agentes reportan por el servicio
//! local; la app solo la lee. Los cambios llegan a la UI con el evento `agent-change`
//! (`service.rs`).

use kanbania_core::error::ErrorPayload;
use kanbania_core::features::agent_activity::{self, StoryActivity, StoryActivityInput};
use tauri::State;

use crate::storage::StorageSlot;

#[tauri::command]
pub async fn list_story_activity(
    slot: State<'_, StorageSlot>,
    input: StoryActivityInput,
) -> Result<StoryActivity, ErrorPayload> {
    let store = slot.store().await?;
    agent_activity::list_story_activity(&store, &input)
        .await
        .map_err(|e| e.to_payload(None))
}
