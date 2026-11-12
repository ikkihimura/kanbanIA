//! Credenciales de agentes y estado del servicio local (HU-012). Solo desde la app: el actor
//! es siempre la persona. Nunca se exponen por el servicio local ni por MCP.

use kanbania_core::error::ErrorPayload;
use kanbania_core::features::access::{
    self, AgentCredential, CreateAgentCredentialInput, CreatedAgentCredential, LocalServiceStatus,
    RevokeAgentCredentialInput,
};
use kanbania_core::platform::actor::Actor;
use tauri::State;

use crate::service::ServiceSlot;
use crate::storage::StorageSlot;

#[tauri::command]
pub async fn list_agent_credentials(
    slot: State<'_, StorageSlot>,
    project_id: String,
) -> Result<Vec<AgentCredential>, ErrorPayload> {
    let store = slot.store().await?;
    access::list_agent_credentials(&store, &project_id)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn create_agent_credential(
    slot: State<'_, StorageSlot>,
    input: CreateAgentCredentialInput,
) -> Result<CreatedAgentCredential, ErrorPayload> {
    let store = slot.store().await?;
    access::create_agent_credential(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn revoke_agent_credential(
    slot: State<'_, StorageSlot>,
    input: RevokeAgentCredentialInput,
) -> Result<AgentCredential, ErrorPayload> {
    let store = slot.store().await?;
    access::revoke_agent_credential(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn get_local_service_status(
    service: State<'_, ServiceSlot>,
    slot: State<'_, StorageSlot>,
) -> Result<LocalServiceStatus, ErrorPayload> {
    Ok(service.status(&slot).await)
}
