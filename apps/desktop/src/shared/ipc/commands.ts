// Envoltorios tipados de los comandos IPC. Los tipos de datos vienen de Rust (ts-rs);
// los nombres de comando deben coincidir con `register_commands` en src-tauri/src/lib.rs.
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AgentCredential } from "../../contracts/generated/AgentCredential";
import type { AppInfo } from "../../contracts/generated/AppInfo";
import type { CreateAgentCredentialInput } from "../../contracts/generated/CreateAgentCredentialInput";
import type { CreatedAgentCredential } from "../../contracts/generated/CreatedAgentCredential";
import type { LocalServiceStatus } from "../../contracts/generated/LocalServiceStatus";
import type { RevokeAgentCredentialInput } from "../../contracts/generated/RevokeAgentCredentialInput";
import type { ReleaseClaimInput } from "../../contracts/generated/ReleaseClaimInput";
import type { StoryClaim } from "../../contracts/generated/StoryClaim";
import type { AgentChange } from "../../contracts/generated/AgentChange";
import type { StoryActivity } from "../../contracts/generated/StoryActivity";
import type { StoryActivityInput } from "../../contracts/generated/StoryActivityInput";
import type { CancelRunInput } from "../../contracts/generated/CancelRunInput";
import type { CreateRunInput } from "../../contracts/generated/CreateRunInput";
import type { RunDetail } from "../../contracts/generated/RunDetail";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import type { RemoveRunWorktreeInput } from "../../contracts/generated/RemoveRunWorktreeInput";
import type { RunChange } from "../../contracts/generated/RunChange";
import type { RunOutputView } from "../../contracts/generated/RunOutputView";
import type { RunSettings } from "../../contracts/generated/RunSettings";
import type { RunWorktree } from "../../contracts/generated/RunWorktree";
import type { StartRunInput } from "../../contracts/generated/StartRunInput";
import type { UpdateRunSettingsInput } from "../../contracts/generated/UpdateRunSettingsInput";
import type { WorktreeChanges } from "../../contracts/generated/WorktreeChanges";
import type { BackupInfo } from "../../contracts/generated/BackupInfo";
import type { RestoreOutcome } from "../../contracts/generated/RestoreOutcome";
import type { RestoreSource } from "../../contracts/generated/RestoreSource";
import type { ApplyImportInput } from "../../contracts/generated/ApplyImportInput";
import type { ExportProjectInput } from "../../contracts/generated/ExportProjectInput";
import type { ExportResult } from "../../contracts/generated/ExportResult";
import type { ImportPreview } from "../../contracts/generated/ImportPreview";
import type { ImportResult } from "../../contracts/generated/ImportResult";
import type { MoveStoryInput } from "../../contracts/generated/MoveStoryInput";
import type { AcceptStoryInput } from "../../contracts/generated/AcceptStoryInput";
import type { AttentionItem } from "../../contracts/generated/AttentionItem";
import type { Delivery } from "../../contracts/generated/Delivery";
import type { ReopenStoryInput } from "../../contracts/generated/ReopenStoryInput";
import type { ReturnDeliveryInput } from "../../contracts/generated/ReturnDeliveryInput";
import type { SetBlockerInput } from "../../contracts/generated/SetBlockerInput";
import type { StoryReview } from "../../contracts/generated/StoryReview";
import type { SubmitDeliveryInput } from "../../contracts/generated/SubmitDeliveryInput";
import type { Agent } from "../../contracts/generated/Agent";
import type { CreateAgentInput } from "../../contracts/generated/CreateAgentInput";
import type { SetStoryAgentInput } from "../../contracts/generated/SetStoryAgentInput";
import type { UpdateAgentInput } from "../../contracts/generated/UpdateAgentInput";
import type { UpdateUserProfileInput } from "../../contracts/generated/UpdateUserProfileInput";
import type { UserProfile } from "../../contracts/generated/UserProfile";
import type { CreateEpicInput } from "../../contracts/generated/CreateEpicInput";
import type { DependencyInput } from "../../contracts/generated/DependencyInput";
import type { Epic } from "../../contracts/generated/Epic";
import type { SetStoryEpicInput } from "../../contracts/generated/SetStoryEpicInput";
import type { SetStoryParentInput } from "../../contracts/generated/SetStoryParentInput";
import type { StoryRelations } from "../../contracts/generated/StoryRelations";
import type { UpdateEpicInput } from "../../contracts/generated/UpdateEpicInput";
import type { ArchiveStoryInput } from "../../contracts/generated/ArchiveStoryInput";
import type { CreateStoryInput } from "../../contracts/generated/CreateStoryInput";
import type { RestoreStoryInput } from "../../contracts/generated/RestoreStoryInput";
import type { StoryDetail } from "../../contracts/generated/StoryDetail";
import type { StorySummary } from "../../contracts/generated/StorySummary";
import type { UpdateStoryInput } from "../../contracts/generated/UpdateStoryInput";
import type { Column } from "../../contracts/generated/Column";
import type { CreateColumnInput } from "../../contracts/generated/CreateColumnInput";
import type { CreateProjectInput } from "../../contracts/generated/CreateProjectInput";
import type { DeleteColumnInput } from "../../contracts/generated/DeleteColumnInput";
import type { ErrorPayload } from "../../contracts/generated/ErrorPayload";
import type { LinkRepositoryInput } from "../../contracts/generated/LinkRepositoryInput";
import type { Project } from "../../contracts/generated/Project";
import type { ProjectSummary } from "../../contracts/generated/ProjectSummary";
import type { RelocateRepositoryInput } from "../../contracts/generated/RelocateRepositoryInput";
import type { RenameProjectInput } from "../../contracts/generated/RenameProjectInput";
import type { ReorderColumnInput } from "../../contracts/generated/ReorderColumnInput";
import type { RepositoryLink } from "../../contracts/generated/RepositoryLink";
import type { StorageStatus } from "../../contracts/generated/StorageStatus";
import type { UnlinkRepositoryInput } from "../../contracts/generated/UnlinkRepositoryInput";
import type { UpdateColumnInput } from "../../contracts/generated/UpdateColumnInput";

export class NotInTauriError extends Error {
  constructor() {
    super(
      "La interfaz se abrió fuera de la ventana de Tauri; usa `npm run dev` en la raíz.",
    );
    this.name = "NotInTauriError";
  }
}

async function call<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!isTauri()) {
    throw new NotInTauriError();
  }
  return invoke<T>(command, args);
}

/** Identificador para mutaciones reintentables (idempotencia en el núcleo). */
export function newRequestId(): string {
  return crypto.randomUUID();
}

// --- sistema ---

export function getAppInfo(): Promise<AppInfo> {
  return call("get_app_info");
}

export function getStorageStatus(): Promise<StorageStatus> {
  return call("get_storage_status");
}

export function retryOpenStorage(): Promise<StorageStatus> {
  return call("retry_open_storage");
}

// --- proyectos ---

export function listProjects(): Promise<ProjectSummary[]> {
  return call("list_projects");
}

export function createProject(input: CreateProjectInput): Promise<Project> {
  return call("create_project", { input });
}

export function renameProject(input: RenameProjectInput): Promise<Project> {
  return call("rename_project", { input });
}

export function openProject(id: string): Promise<Project> {
  return call("open_project", { id });
}

export function getLastOpenedProject(): Promise<Project | null> {
  return call("get_last_opened_project");
}

// --- repositorios ---

export function listRepositories(projectId: string): Promise<RepositoryLink[]> {
  return call("list_repositories", { projectId });
}

export function linkRepository(
  input: LinkRepositoryInput,
): Promise<RepositoryLink> {
  return call("link_repository", { input });
}

export function relocateRepository(
  input: RelocateRepositoryInput,
): Promise<RepositoryLink> {
  return call("relocate_repository", { input });
}

export function unlinkRepository(input: UnlinkRepositoryInput): Promise<void> {
  return call("unlink_repository", { input });
}

// --- columnas ---

export function listColumns(projectId: string): Promise<Column[]> {
  return call("list_columns", { projectId });
}

export function createColumn(input: CreateColumnInput): Promise<Column> {
  return call("create_column", { input });
}

export function updateColumn(input: UpdateColumnInput): Promise<Column> {
  return call("update_column", { input });
}

export function reorderColumn(input: ReorderColumnInput): Promise<Column> {
  return call("reorder_column", { input });
}

export function deleteColumn(input: DeleteColumnInput): Promise<void> {
  return call("delete_column", { input });
}

// --- historias ---

export function listStories(
  projectId: string,
  includeArchived: boolean,
): Promise<StorySummary[]> {
  return call("list_stories", { projectId, includeArchived });
}

export function getStory(projectId: string, id: string): Promise<StoryDetail> {
  return call("get_story", { projectId, id });
}

export function createStory(input: CreateStoryInput): Promise<StorySummary> {
  return call("create_story", { input });
}

/** Mueve una tarjeta de forma atómica (WIP, conflictos y «hecho» se validan en el núcleo). */
export function moveStory(input: MoveStoryInput): Promise<StorySummary> {
  return call("move_story", { input });
}

export function updateStory(input: UpdateStoryInput): Promise<StoryDetail> {
  return call("update_story", { input });
}

export function archiveStory(input: ArchiveStoryInput): Promise<StorySummary> {
  return call("archive_story", { input });
}

export function restoreStory(input: RestoreStoryInput): Promise<StorySummary> {
  return call("restore_story", { input });
}

// --- épicas ---

export function listEpics(projectId: string): Promise<Epic[]> {
  return call("list_epics", { projectId });
}

export function createEpic(input: CreateEpicInput): Promise<Epic> {
  return call("create_epic", { input });
}

export function updateEpic(input: UpdateEpicInput): Promise<Epic> {
  return call("update_epic", { input });
}

// --- relaciones ---

export function getStoryRelations(
  projectId: string,
  storyId: string,
): Promise<StoryRelations> {
  return call("get_story_relations", { projectId, storyId });
}

export function setStoryEpic(
  input: SetStoryEpicInput,
): Promise<StoryRelations> {
  return call("set_story_epic", { input });
}

export function setStoryParent(
  input: SetStoryParentInput,
): Promise<StoryRelations> {
  return call("set_story_parent", { input });
}

export function addStoryDependency(
  input: DependencyInput,
): Promise<StoryRelations> {
  return call("add_story_dependency", { input });
}

export function removeStoryDependency(
  input: DependencyInput,
): Promise<StoryRelations> {
  return call("remove_story_dependency", { input });
}

export function setStoryAgent(
  input: SetStoryAgentInput,
): Promise<StoryRelations> {
  return call("set_story_agent", { input });
}

// --- equipo ---

export function getUserProfile(): Promise<UserProfile> {
  return call("get_user_profile");
}

export function updateUserProfile(
  input: UpdateUserProfileInput,
): Promise<UserProfile> {
  return call("update_user_profile", { input });
}

export function listAgents(): Promise<Agent[]> {
  return call("list_agents");
}

export function createAgent(input: CreateAgentInput): Promise<Agent> {
  return call("create_agent", { input });
}

export function updateAgent(input: UpdateAgentInput): Promise<Agent> {
  return call("update_agent", { input });
}

// --- revisión y atención ---

export function getStoryReview(
  projectId: string,
  storyId: string,
): Promise<StoryReview> {
  return call("get_story_review", { projectId, storyId });
}

export function listAttention(projectId: string): Promise<AttentionItem[]> {
  return call("list_attention", { projectId });
}

export function submitDelivery(input: SubmitDeliveryInput): Promise<Delivery> {
  return call("submit_delivery", { input });
}

export function returnDelivery(input: ReturnDeliveryInput): Promise<void> {
  return call("return_delivery", { input });
}

export function acceptStory(input: AcceptStoryInput): Promise<StorySummary> {
  return call("accept_story", { input });
}

export function reopenStory(input: ReopenStoryInput): Promise<StorySummary> {
  return call("reopen_story", { input });
}

export function setStoryBlocker(input: SetBlockerInput): Promise<void> {
  return call("set_story_blocker", { input });
}

// --- cierre con cambios sin guardar ---

// --- intentos de ejecución (HU-017) ---

/**
 * Prepara un intento: congela la instantánea (historia, criterios, agente, repositorios e
 * instrucciones con la nota) y lo deja en cola. Falla con `run_active` si la historia ya tiene
 * uno activo. Mantén el mismo `requestId` al reintentar el mismo envío.
 */
export function createRun(input: CreateRunInput): Promise<RunSummary> {
  return call("create_run", { input });
}

/** Cancela un intento activo; uno terminado devuelve `invalid_run_transition`. */
export function cancelRun(input: CancelRunInput): Promise<RunSummary> {
  return call("cancel_run", { input });
}

/** Intentos de una historia, del más reciente al más antiguo. */
export function listStoryRuns(
  projectId: string,
  storyId: string,
): Promise<RunSummary[]> {
  return call("list_story_runs", { projectId, storyId });
}

/** Detalle de un intento: resumen, instantánea congelada y últimos eventos. */
export function getRun(projectId: string, runId: string): Promise<RunDetail> {
  return call("get_run", { projectId, runId });
}

// --- ejecución supervisada (HU-018) ---

/**
 * Pide iniciar un intento en cola con el proceso de prueba (sin IA). Sigue `queued` hasta que
 * el supervisor tenga hueco. Mantén el mismo `requestId` al reintentar el mismo envío.
 */
export function startRun(input: StartRunInput): Promise<RunSummary> {
  return call("start_run", { input });
}

/**
 * Últimas líneas (≤ 500) de la salida de un intento. El texto es dato no confiable: muéstralo
 * como texto plano, nunca como Markdown ni HTML.
 */
export function getRunOutput(
  projectId: string,
  runId: string,
): Promise<RunOutputView> {
  return call("get_run_output", { projectId, runId });
}

/** Cambios del worktree de un intento terminado (`missing` si la carpeta ya no existe). */
export function getWorktreeChanges(
  projectId: string,
  runId: string,
): Promise<WorktreeChanges> {
  return call("get_worktree_changes", { projectId, runId });
}

/**
 * Elimina el worktree de un intento terminado. Sin `force`, uno con cambios devuelve
 * `validation` (campo `force`); `force: true` es «Descartar cambios y eliminar». La rama se
 * conserva siempre.
 */
export function removeRunWorktree(
  input: RemoveRunWorktreeInput,
): Promise<RunWorktree> {
  return call("remove_run_worktree", { input });
}

export function getRunSettings(): Promise<RunSettings> {
  return call("get_run_settings");
}

/** Revisión obsoleta → `conflict` con la configuración vigente. */
export function updateRunSettings(
  input: UpdateRunSettingsInput,
): Promise<RunSettings> {
  return call("update_run_settings", { input });
}

/** Nombre del evento emitido por el host (`RUN_CHANGE_EVENT` en run_supervisor.rs). */
export const RUN_CHANGE_EVENT = "run-change";

/** Comprueba la forma del aviso antes de usarlo: un evento mal formado se ignora. */
export function isRunChange(value: unknown): value is RunChange {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    Array.isArray(candidate.runIds) &&
    candidate.runIds.every((id) => typeof id === "string") &&
    Array.isArray(candidate.storyIds) &&
    candidate.storyIds.every((id) => typeof id === "string")
  );
}

/**
 * Escucha los avisos del supervisor (despachar, iniciar, terminar, interrumpir…). Solo sirven
 * para recargar: el núcleo sigue siendo la autoridad. Devuelve la función para dejar de
 * escuchar.
 */
export async function onRunChange(
  handler: (change: RunChange) => void,
): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  return listen<unknown>(RUN_CHANGE_EVENT, (event) => {
    if (isRunChange(event.payload)) handler(event.payload);
  });
}

/** Nombre del evento emitido por el host (`CONFIRM_CLOSE_EVENT` en exit_guard.rs). */
export const CONFIRM_CLOSE_EVENT = "unsaved-changes:confirm-close";

export async function setUnsavedChanges(dirty: boolean): Promise<void> {
  if (!isTauri()) return;
  await invoke("set_unsaved_changes", { dirty });
}

export function discardAndClose(): Promise<void> {
  return call("discard_and_close");
}

/** Escucha la petición de confirmar el cierre. Devuelve la función para dejar de escuchar. */
export async function onConfirmClose(handler: () => void): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  return listen(CONFIRM_CLOSE_EVENT, handler);
}

// --- portabilidad (HU-010) ---

export function exportProject(
  input: ExportProjectInput,
): Promise<ExportResult> {
  return call("export_project", { input });
}

export function previewImport(path: string): Promise<ImportPreview> {
  return call("preview_import", { path });
}

export function applyImport(input: ApplyImportInput): Promise<ImportResult> {
  return call("apply_import", { input });
}

// --- copias de seguridad (HU-011) ---

export function listBackups(): Promise<BackupInfo[]> {
  return call("list_backups");
}

export function createBackup(): Promise<BackupInfo> {
  return call("create_backup");
}

export function saveBackupToFolder(args: {
  id: string;
  destination: string;
}): Promise<string> {
  return call("save_backup_to_folder", args);
}

/** Sustituye todos los datos: el host cierra el almacén, restaura y lo reabre. */
export function restoreBackup(source: RestoreSource): Promise<RestoreOutcome> {
  return call("restore_backup", { source });
}

// --- acceso de agentes y servicio local (HU-012) ---

export function listAgentCredentials(
  projectId: string,
): Promise<AgentCredential[]> {
  return call("list_agent_credentials", { projectId });
}

export function createAgentCredential(
  input: CreateAgentCredentialInput,
): Promise<CreatedAgentCredential> {
  return call("create_agent_credential", { input });
}

export function revokeAgentCredential(
  input: RevokeAgentCredentialInput,
): Promise<AgentCredential> {
  return call("revoke_agent_credential", { input });
}

export function getLocalServiceStatus(): Promise<LocalServiceStatus> {
  return call("get_local_service_status");
}

// --- reclamos de historias (HU-014) ---

/** Reclamos vigentes del proyecto, ordenados por número de historia. */
export function listActiveClaims(projectId: string): Promise<StoryClaim[]> {
  return call("list_active_claims", { projectId });
}

/** Libera un reclamo (solo personas). Idempotente: liberar uno ya cerrado no falla. */
export function releaseClaim(input: ReleaseClaimInput): Promise<void> {
  return call("release_claim", { input });
}

// --- actividad reportada por agentes (HU-015) ---

/**
 * Actividad que los agentes reportaron en una historia (de la más reciente a la más antigua).
 * Son afirmaciones del agente, no hechos observados por la app.
 */
export function listStoryActivity(
  input: StoryActivityInput,
): Promise<StoryActivity> {
  return call("list_story_activity", { input });
}

/** Nombre del evento emitido por el host (`AGENT_CHANGE_EVENT` en commands/agent_reports.rs). */
export const AGENT_CHANGE_EVENT = "agent-change";

/** Comprueba la forma del aviso antes de usarlo: un evento mal formado se ignora. */
export function isAgentChange(value: unknown): value is AgentChange {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.projectId === "string" &&
    Array.isArray(candidate.storyIds) &&
    candidate.storyIds.every((id) => typeof id === "string")
  );
}

/**
 * Escucha los avisos de cambios hechos por agentes (reclamar, renovar, bloqueo, subtarea,
 * entrega). Solo sirven para recargar: el núcleo sigue siendo la autoridad. Devuelve la función
 * para dejar de escuchar.
 */
export async function onAgentChange(
  handler: (change: AgentChange) => void,
): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  return listen<unknown>(AGENT_CHANGE_EVENT, (event) => {
    if (isAgentChange(event.payload)) handler(event.payload);
  });
}

// --- errores ---

export function isErrorPayload(error: unknown): error is ErrorPayload {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    "message" in error &&
    typeof error.message === "string"
  );
}

/** Mensaje legible para errores de `invoke`: `ErrorPayload` del núcleo, `Error` o texto. */
export function describeError(error: unknown): string {
  if (isErrorPayload(error)) return error.message;
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Error desconocido al comunicarse con el núcleo.";
}

/** Error normalizado para la UI; conserva `code`, `field` y `current` si vienen del núcleo. */
export type UiError = Pick<ErrorPayload, "message"> &
  Partial<Pick<ErrorPayload, "code" | "field" | "current">>;

export function toUiError(error: unknown): UiError {
  return isErrorPayload(error) ? error : { message: describeError(error) };
}
