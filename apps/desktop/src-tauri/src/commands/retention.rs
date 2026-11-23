//! Limpieza de datos de ejecuciones (HU-025): adaptador fino sobre `kanbania_runner::cleanup`
//! (plan sin borrar nada, limpiar un intento, eliminar un worktree huérfano). Solo personas;
//! nunca para agentes.

use kanbania_core::error::ErrorPayload;
use kanbania_core::features::runs::{
    CleanRunInput, CleanRunResult, RemoveOrphanWorktreeInput, RetentionPlan, WorktreeRemoval,
};
use kanbania_runner::cleanup;
use tauri::State;

use crate::storage::StorageSlot;
use kanbania_core::platform::actor::Actor;

/// Plan de limpieza: cuánto ocupa todo y qué se propone. No borra nada.
#[tauri::command]
pub async fn get_retention_plan(
    slot: State<'_, StorageSlot>,
) -> Result<RetentionPlan, ErrorPayload> {
    let store = slot.store().await?;
    cleanup::retention_plan(&store, slot.data_dir())
        .await
        .map_err(|e| e.to_payload(None))
}

/// Limpia lo que la persona marcó de un intento (worktree, salida, rama).
#[tauri::command]
pub async fn clean_run(
    slot: State<'_, StorageSlot>,
    input: CleanRunInput,
) -> Result<CleanRunResult, ErrorPayload> {
    let store = slot.store().await?;
    cleanup::clean_run(&store, &Actor::Human, slot.data_dir(), &input)
        .await
        .map_err(|e| e.to_payload(None))
}

/// Elimina una carpeta de worktree huérfana con las comprobaciones de HU-018.
#[tauri::command]
pub async fn remove_orphan_worktree(
    slot: State<'_, StorageSlot>,
    input: RemoveOrphanWorktreeInput,
) -> Result<WorktreeRemoval, ErrorPayload> {
    let store = slot.store().await?;
    cleanup::remove_orphan_worktree(&store, &Actor::Human, slot.data_dir(), &input)
        .await
        .map_err(|e| e.to_payload(None))
}
