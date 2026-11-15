//! Intentos de ejecución desde la app (HU-017/HU-018). El actor es siempre la persona.
//! Registrar eventos (`record_run_event`) y la API del supervisor no tienen comando. Iniciar,
//! cancelar y cambiar la configuración despiertan al supervisor. Los errores de Git y de disco
//! llegan a la UI con mensajes fijos y seguros (sin rutas ni salida de Git).

use kanbania_core::error::{CoreError, ErrorPayload};
use kanbania_core::features::runs::{
    self, CancelRunInput, CreateRunInput, RecordWorktreeRemovalInput, RemoveRunWorktreeInput,
    RunDetail, RunOutputView, RunSettings, RunSummary, RunWorktree, StartRunInput,
    UpdateRunSettingsInput, WorktreeChanges, WorktreeRemoval,
};
use kanbania_core::platform::actor::Actor;
use kanbania_runner::supervisor::{git_config, read_run_output};
use kanbania_runner::worktree::{
    DEFAULT_GIT_TIMEOUT, RemoveOutcome, WorktreeError, WorktreeErrorKind, remove_worktree,
    worktree_status,
};
use tauri::State;

use crate::run_supervisor::RunSupervisorSlot;
use crate::storage::StorageSlot;

#[tauri::command]
pub async fn create_run(
    slot: State<'_, StorageSlot>,
    input: CreateRunInput,
) -> Result<RunSummary, ErrorPayload> {
    let store = slot.store().await?;
    runs::create_run(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))
}

#[tauri::command]
pub async fn cancel_run(
    slot: State<'_, StorageSlot>,
    supervisor: State<'_, RunSupervisorSlot>,
    input: CancelRunInput,
) -> Result<RunSummary, ErrorPayload> {
    let store = slot.store().await?;
    let run = runs::cancel_run(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))?;
    // Con proceso es una petición: que el supervisor mate el árbol ya.
    supervisor.wake();
    Ok(run)
}

#[tauri::command]
pub async fn list_story_runs(
    slot: State<'_, StorageSlot>,
    project_id: String,
    story_id: String,
) -> Result<Vec<RunSummary>, ErrorPayload> {
    let store = slot.store().await?;
    runs::list_story_runs(&store, &project_id, &story_id)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn get_run(
    slot: State<'_, StorageSlot>,
    project_id: String,
    run_id: String,
) -> Result<RunDetail, ErrorPayload> {
    let store = slot.store().await?;
    runs::get_run(&store, &project_id, &run_id)
        .await
        .map_err(|e| e.to_payload(None))
}

/// Pide iniciar un intento en cola con el proceso de prueba (sin IA). Sigue `queued` hasta que
/// el supervisor tenga hueco.
#[tauri::command]
pub async fn start_run(
    slot: State<'_, StorageSlot>,
    supervisor: State<'_, RunSupervisorSlot>,
    input: StartRunInput,
) -> Result<RunSummary, ErrorPayload> {
    // Sin supervisor la petición se quedaría en cola sin que nada la atienda: no se registra.
    if !supervisor.is_running().await {
        return Err(CoreError::Validation {
            field: "runId",
            reason: SUPERVISOR_UNAVAILABLE.into(),
        }
        .to_payload(Some(&input.request_id)));
    }
    let store = slot.store().await?;
    let run = runs::start_run(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))?;
    supervisor.wake();
    Ok(run)
}

/// Últimas líneas (≤ 500) de la salida de un intento del proyecto. Texto no confiable: la UI
/// lo muestra como texto plano.
#[tauri::command]
pub async fn get_run_output(
    slot: State<'_, StorageSlot>,
    project_id: String,
    run_id: String,
) -> Result<RunOutputView, ErrorPayload> {
    let store = slot.store().await?;
    // Comprueba que el intento existe y es de este proyecto (y valida el id).
    let run = runs::get_run(&store, &project_id, &run_id)
        .await
        .map_err(|e| e.to_payload(None))?
        .run;
    let data_dir = slot.data_dir().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || read_run_output(&data_dir, &run.id))
        .await
        .map_err(|_| io_unavailable())?
        .map_err(|_| io_unavailable())
}

/// Cambios del worktree de un intento terminado (`git status`), para decidir cómo eliminarlo.
#[tauri::command]
pub async fn get_worktree_changes(
    slot: State<'_, StorageSlot>,
    project_id: String,
    run_id: String,
) -> Result<WorktreeChanges, ErrorPayload> {
    let store = slot.store().await?;
    let target = runs::get_worktree_target(&store, &project_id, &run_id)
        .await
        .map_err(|e| e.to_payload(None))?;
    let root = runs::worktrees_root(&store).map_err(|e| e.to_payload(None))?;
    let git = git_config(slot.data_dir(), DEFAULT_GIT_TIMEOUT);
    let path = std::path::PathBuf::from(&target.worktree.path);
    let branch = &target.worktree.branch;
    match worktree_status(&git, &target.repository_path, &root, &path, branch).await {
        Ok(status) => Ok(WorktreeChanges {
            missing: false,
            changes: u64::try_from(status.changes).unwrap_or(u64::MAX),
            ignored: u64::try_from(status.ignored).unwrap_or(u64::MAX),
            complete: status.complete,
            commits_at_risk: status.commits_at_risk,
        }),
        Err(error) if error.kind == WorktreeErrorKind::Missing => Ok(WorktreeChanges {
            missing: true,
            changes: 0,
            ignored: 0,
            complete: true,
            commits_at_risk: false,
        }),
        Err(error) => Err(worktree_payload(&error)),
    }
}

/// Elimina el worktree de un intento terminado. Sin `force`, uno con cambios, archivos ignorados
/// o commits en riesgo → `validation` (campo `force`); `force` es «Descartar cambios y eliminar». La rama
/// se conserva siempre. Si la carpeta ya no existía, solo se marca como eliminado.
#[tauri::command]
pub async fn remove_run_worktree(
    slot: State<'_, StorageSlot>,
    input: RemoveRunWorktreeInput,
) -> Result<RunWorktree, ErrorPayload> {
    let store = slot.store().await?;
    let target = runs::get_worktree_target(&store, &input.project_id, &input.run_id)
        .await
        .map_err(|e| e.to_payload(None))?;
    let root = runs::worktrees_root(&store).map_err(|e| e.to_payload(None))?;
    let git = git_config(slot.data_dir(), DEFAULT_GIT_TIMEOUT);
    let path = std::path::PathBuf::from(&target.worktree.path);
    let outcome = remove_worktree(
        &git,
        &target.repository_path,
        &root,
        &path,
        &target.worktree.branch,
        input.force,
    )
    .await
    .map_err(|e| worktree_payload(&e))?;
    let removal = match outcome {
        RemoveOutcome::Removed => WorktreeRemoval::Clean,
        RemoveOutcome::Forced => WorktreeRemoval::Discarded,
        RemoveOutcome::Missing => WorktreeRemoval::Missing,
    };
    runs::record_worktree_removal(
        &store,
        &Actor::Human,
        &RecordWorktreeRemovalInput {
            project_id: input.project_id,
            run_id: input.run_id,
            removal,
        },
    )
    .await
    .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn get_run_settings(slot: State<'_, StorageSlot>) -> Result<RunSettings, ErrorPayload> {
    let store = slot.store().await?;
    runs::get_run_settings(&store)
        .await
        .map_err(|e| e.to_payload(None))
}

#[tauri::command]
pub async fn update_run_settings(
    slot: State<'_, StorageSlot>,
    supervisor: State<'_, RunSupervisorSlot>,
    input: UpdateRunSettingsInput,
) -> Result<RunSettings, ErrorPayload> {
    let store = slot.store().await?;
    let settings = runs::update_run_settings(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(None))?;
    // Más concurrencia puede dejar hueco para los que esperan.
    supervisor.wake();
    Ok(settings)
}

const SUPERVISOR_UNAVAILABLE: &str =
    "No se pueden ejecutar intentos ahora: cierra Kanban IA y vuelve a abrirlo.";

/// Error de Git o de disco para la UI: el mensaje fijo y seguro del runner; el detalle (rutas,
/// salida de Git) no sale del host.
fn worktree_payload(error: &WorktreeError) -> ErrorPayload {
    // `force`: la UI lo trata como «tiene algo que se perdería» (cambios sin commit, archivos
    // ignorados o commits en riesgo) y ofrece «Descartar … y eliminar».
    let field = if error.needs_force() {
        "force"
    } else {
        "worktree"
    };
    eprintln!("[runs] operación de worktree rechazada: {:?}", error.kind);
    CoreError::Validation {
        field,
        reason: error.to_string(),
    }
    .to_payload(None)
}

fn io_unavailable() -> ErrorPayload {
    CoreError::StorageIo(std::io::Error::other("salida no disponible")).to_payload(None)
}
