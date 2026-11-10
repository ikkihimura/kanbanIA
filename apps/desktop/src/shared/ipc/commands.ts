// Envoltorios tipados de los comandos IPC. Los tipos de datos vienen de Rust (ts-rs);
// los nombres de comando deben coincidir con `register_commands` en src-tauri/src/lib.rs.
import { invoke, isTauri } from "@tauri-apps/api/core";
import type { AppInfo } from "../../contracts/generated/AppInfo";
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
