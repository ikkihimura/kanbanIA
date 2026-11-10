// Envoltorios tipados de los comandos IPC. Los tipos de datos vienen de Rust (ts-rs);
// los nombres de comando deben coincidir con `register_commands` en src-tauri/src/lib.rs.
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AppInfo } from "../../contracts/generated/AppInfo";
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
