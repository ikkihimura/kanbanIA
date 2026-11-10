import type { ColumnCategory } from "../../contracts/generated/ColumnCategory";

/** Etiquetas visibles de la categoría semántica (las reglas usan la categoría, no el nombre). */
export const CATEGORY_LABEL: Record<ColumnCategory, string> = {
  backlog: "Pendiente",
  ready: "Listo para empezar",
  active: "En curso",
  review: "En revisión",
  done: "Hecho (requiere aceptación)",
};

export const CATEGORIES: ColumnCategory[] = [
  "backlog",
  "ready",
  "active",
  "review",
  "done",
];

/** Campo de límite WIP: vacío = sin límite. Devuelve `undefined` si el texto no es un entero. */
export function parseWip(text: string): number | null | undefined {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  return /^\d+$/.test(trimmed) ? Number(trimmed) : undefined;
}

export function cardsLabel(count: number): string {
  return count === 1 ? "1 tarjeta" : `${String(count)} tarjetas`;
}
