// Envoltorios tipados de los comandos IPC. Los tipos de datos vienen de Rust (ts-rs);
// los nombres de comando deben coincidir con `register_commands` en src-tauri/src/lib.rs.
import { invoke, isTauri } from "@tauri-apps/api/core";
import type { AppInfo } from "../../contracts/generated/AppInfo";

export class NotInTauriError extends Error {
  constructor() {
    super(
      "La interfaz se abrió fuera de la ventana de Tauri; usa `npm run dev` en la raíz.",
    );
    this.name = "NotInTauriError";
  }
}

async function call<T>(command: string): Promise<T> {
  if (!isTauri()) {
    throw new NotInTauriError();
  }
  return invoke<T>(command);
}

export function getAppInfo(): Promise<AppInfo> {
  return call<AppInfo>("get_app_info");
}

export function describeError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Error desconocido al comunicarse con el núcleo.";
}
