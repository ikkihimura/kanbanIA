import { open } from "@tauri-apps/plugin-dialog";

/** Selector nativo de carpetas (permiso `dialog:allow-open`). `null` si se cancela. */
export async function chooseFolder(title: string): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false, title });
  return typeof selected === "string" ? selected : null;
}
