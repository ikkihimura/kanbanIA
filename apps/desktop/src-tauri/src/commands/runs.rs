//! Intentos de ejecución desde la app (HU-017/HU-018/HU-019/HU-021). El actor es siempre la
//! persona.
//! Registrar eventos (`record_run_event`) y la API del supervisor no tienen comando. Iniciar,
//! cancelar y cambiar la configuración despiertan al supervisor. Los errores de Git y de disco
//! llegan a la UI con mensajes fijos y seguros (sin rutas ni salida de Git).

use kanbania_core::error::{CoreError, ErrorPayload};
use kanbania_core::features::runs::{
    self, AcpAdapterCheck, AcpAgent, CancelRunInput, CodexHomeStatus, CreateRunInput,
    DecideApprovalInput, NodeCandidate, NodeSearchRoots, RecordWorktreeRemovalInput,
    RemoveRunWorktreeInput, RunApproval, RunDetail, RunOutputView, RunSettings, RunSummary,
    RunWorktree, StartRunInput, UpdateRunSettingsInput, WaitingApproval, WorktreeChanges,
    WorktreeRemoval,
};
use kanbania_core::platform::actor::Actor;
use kanbania_runner::acp::{node_path_env, probe_adapter};
use kanbania_runner::supervisor::{adapter_env, git_config, read_run_output};
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

/// Decide una petición de permiso de un agente (HU-020): permitir una vez o rechazar. Despierta
/// al supervisor para que entregue la decisión al agente.
#[tauri::command]
pub async fn decide_approval(
    slot: State<'_, StorageSlot>,
    supervisor: State<'_, RunSupervisorSlot>,
    input: DecideApprovalInput,
) -> Result<RunApproval, ErrorPayload> {
    let store = slot.store().await?;
    let approval = runs::decide_approval(&store, &Actor::Human, &input)
        .await
        .map_err(|e| e.to_payload(Some(&input.request_id)))?;
    supervisor.wake();
    Ok(approval)
}

/// Intentos del proyecto que esperan una decisión de permiso (bandeja y tablero).
#[tauri::command]
pub async fn list_waiting_approvals(
    slot: State<'_, StorageSlot>,
    project_id: String,
) -> Result<Vec<WaitingApproval>, ErrorPayload> {
    let store = slot.store().await?;
    runs::list_waiting_approvals(&store, &project_id)
        .await
        .map_err(|e| e.to_payload(None))
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

/// Pide iniciar un intento en cola con el ejecutor elegido (proceso de prueba sin IA o Claude
/// Code vía ACP, que exige un adaptador configurado). Sigue `queued` hasta que el supervisor
/// tenga hueco.
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

/// Límite de «Comprobar adaptador» (solo `initialize`).
const ADAPTER_CHECK_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

/// «Comprobar adaptador» (HU-019, HU-021): lanza el adaptador ACP de `agent` con el mismo
/// entorno que una ejecución (Codex: su carpeta propia, `codex-home`, que se crea si falta),
/// hace solo `initialize` (≤ 30 s), termina su árbol y devuelve lo que declaró (protocolo,
/// nombre, título, versión y capacidades; sin `_meta` ni datos de cuenta). Con `path`, comprueba
/// esa ruta antes de guardarla (mismas reglas que la configuración); sin ella, la configurada
/// para ese agente. Igual con `nodePath` (HU-026): con ella (vacía = sin ruta de node), esa
/// ruta sin guardar (validada como al guardarla, errores en `nodePath`); sin ella, la
/// guardada. Su carpeta va delante del `PATH` del adaptador, como en una ejecución; un script
/// `#!/usr/bin/env node` sin `node` en ese `PATH` da un mensaje propio. No crea sesión ni envía
/// instrucciones (no comprueba la sesión iniciada). Errores → `validation`
/// (`claudeAcpPath`/`codexAcpPath`) con un mensaje fijo, sin rutas ni texto del adaptador.
/// Nunca para agentes.
#[tauri::command]
pub async fn check_acp_adapter(
    slot: State<'_, StorageSlot>,
    agent: AcpAgent,
    path: Option<String>,
    node_path: Option<String>,
) -> Result<AcpAdapterCheck, ErrorPayload> {
    let typed = path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned);
    let saved = if typed.is_none() || node_path.is_none() {
        let store = slot.store().await?;
        Some(
            runs::get_run_settings(&store)
                .await
                .map_err(|e| e.to_payload(None))?,
        )
    } else {
        None
    };
    let requested = match typed {
        Some(path) => path,
        None => saved
            .as_ref()
            .and_then(|settings| settings.adapter_path(agent))
            .map(str::to_owned)
            .ok_or_else(|| {
                adapter_invalid(
                    agent,
                    "No hay ningún adaptador configurado: indica su ruta.",
                )
            })?,
    };
    let program = runs::check_adapter_path(agent, &requested)
        .await
        .map_err(|e| e.to_payload(None))?;
    // La escrita o, sin ella, la guardada: las dos se validan igual (una guardada pudo
    // desaparecer al cambiar de versión con nvm, y el error lo dice en su campo).
    let node_text = match node_path {
        Some(typed) => typed,
        None => saved
            .as_ref()
            .and_then(|settings| settings.node_path.clone())
            .unwrap_or_default(),
    };
    let node = runs::check_node_path(&node_text, Some(slot.data_dir()))
        .await
        .map_err(|e| e.to_payload(None))?;
    let data_dir = slot.data_dir().to_path_buf();
    let mut extra_env = tauri::async_runtime::spawn_blocking(move || adapter_env(agent, &data_dir))
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or_else(|| {
            eprintln!("[runs] no se pudo preparar el entorno del adaptador");
            adapter_invalid(agent, "No se pudo preparar la carpeta propia del agente.")
        })?;
    extra_env.extend(node_path_env(node.as_deref()));
    let declared = probe_adapter(
        program,
        Vec::new(),
        std::env::temp_dir(),
        extra_env,
        ADAPTER_CHECK_LIMIT,
    )
    .await
    .map_err(|error| {
        eprintln!("[runs] comprobación del adaptador fallida: {error:?}");
        adapter_invalid(agent, &error.to_string())
    })?;
    // El núcleo omite (no recorta) una identidad fuera de los límites y filtra las capacidades
    // desconocidas.
    AcpAdapterCheck::from_reported(
        declared.protocol_version,
        declared.name,
        declared.title,
        declared.version,
        declared
            .capabilities
            .iter()
            .map(|c| (*c).to_owned())
            .collect(),
    )
    .map_err(|e| e.to_payload(None))
}

/// Instalaciones de `node` en las ubicaciones habituales (HU-026), para que la persona elija una
/// como ruta de node. Solo mira archivos: no ejecuta nada ni guarda nada. Rutas locales: solo
/// para la persona, nunca para agentes.
#[tauri::command]
pub async fn find_node_candidates() -> Vec<NodeCandidate> {
    runs::find_node_candidates(NodeSearchRoots::from_env()).await
}

/// Estado de la carpeta de Codex de la app (HU-021): su ruta (para que la persona inicie sesión
/// con `CODEX_HOME=<ruta> codex login`) y si hay un `auth.json` (solo se comprueba que exista;
/// nunca se lee). No crea la carpeta. Ruta local: solo para la persona, nunca para agentes.
#[tauri::command]
pub async fn get_codex_home_status(
    slot: State<'_, StorageSlot>,
) -> Result<CodexHomeStatus, ErrorPayload> {
    let data_dir = slot.data_dir().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || runs::codex_home_status(&data_dir))
        .await
        .map_err(|_| io_unavailable())
}

fn adapter_invalid(agent: AcpAgent, reason: &str) -> ErrorPayload {
    CoreError::Validation {
        field: agent.settings_field(),
        reason: reason.to_owned(),
    }
    .to_payload(None)
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
