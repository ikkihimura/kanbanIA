//! Reclamos de historias desde la app (HU-014): ver los vigentes y liberarlos. El actor es
//! siempre la persona. Reclamar y renovar solo existen para agentes (servicio local / MCP).

use kanbania_core::error::ErrorPayload;
use kanbania_core::features::claims::{self, ReleaseClaimInput, StoryClaim};
use kanbania_core::platform::actor::Actor;
use tauri::State;

use crate::storage::StorageSlot;

#[tauri::command]
pub async fn list_active_claims(
    slot: State<'_, StorageSlot>,
    project_id: String,
) -> Result<Vec<StoryClaim>, ErrorPayload> {
    let store = slot.store().await?;
    claims::list_active_claims(&store, &project_id)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn release_claim(
    slot: State<'_, StorageSlot>,
    input: ReleaseClaimInput,
) -> Result<(), ErrorPayload> {
    let store = slot.store().await?;
    claims::release_claim(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(None))
}
