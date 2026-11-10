// Modelo puro del tablero: filtros, carriles y movimiento optimista. Sin React ni IPC, para
// probarlo de forma aislada. El núcleo sigue siendo la autoridad: si rechaza un movimiento,
// la UI vuelve al estado anterior.
import type { Agent } from "../../contracts/generated/Agent";
import type { Column } from "../../contracts/generated/Column";
import type { Epic } from "../../contracts/generated/Epic";
import type { StorySummary } from "../../contracts/generated/StorySummary";

export type LaneMode = "none" | "epic" | "agent";

export interface BoardFilter {
  text: string;
  /** `""` = todas; `"none"` = sin épica; o el id de la épica. */
  epicId: string;
}

export interface Lane {
  key: string;
  label: string;
}

/** Tarjetas de cada columna en orden, solo las activas (no archivadas). */
export function cardsByColumn(
  columns: Column[],
  stories: StorySummary[],
): Map<string, StorySummary[]> {
  const byColumn = new Map<string, StorySummary[]>(
    columns.map((c) => [c.id, []]),
  );
  for (const story of stories) {
    if (story.archivedAt !== null || story.position === null) continue;
    byColumn.get(story.columnId)?.push(story);
  }
  for (const list of byColumn.values()) {
    list.sort((a, b) => (a.position ?? 0) - (b.position ?? 0));
  }
  return byColumn;
}

export function matchesFilter(
  story: StorySummary,
  filter: BoardFilter,
): boolean {
  const text = filter.text.trim().toLowerCase();
  if (text) {
    const number = text.startsWith("#") ? text.slice(1) : null;
    const byNumber = number !== null && String(story.number) === number;
    if (!byNumber && !story.title.toLowerCase().includes(text)) return false;
  }
  if (filter.epicId === "none") return story.epicId === null;
  if (filter.epicId) return story.epicId === filter.epicId;
  return true;
}

export function laneOf(story: StorySummary, mode: LaneMode): string {
  if (mode === "epic") return story.epicId ?? "none";
  if (mode === "agent") return story.agentId ?? "none";
  return "all";
}

/** Carriles en orden: los de entidades existentes y, al final, «sin épica/agente». */
export function lanesFor(
  mode: LaneMode,
  epics: Epic[],
  agents: Agent[],
): Lane[] {
  if (mode === "epic") {
    return [
      ...epics.map((e) => ({
        key: e.id,
        label: `E-${String(e.number)} ${e.title}`,
      })),
      { key: "none", label: "Sin épica" },
    ];
  }
  if (mode === "agent") {
    return [
      ...agents.map((a) => ({
        key: a.id,
        label: `A-${String(a.number)} ${a.name}`,
      })),
      { key: "none", label: "Sin agente" },
    ];
  }
  return [{ key: "all", label: "" }];
}

/**
 * Aplica un movimiento de forma optimista: saca la tarjeta de su columna y la inserta en
 * `toIndex` (acotado) de la columna destino, dejando posiciones densas en ambas. Es la misma
 * regla que aplica el núcleo, para que la vista previa coincida con el resultado guardado.
 */
export function applyMove(
  stories: StorySummary[],
  storyId: string,
  toColumnId: string,
  toIndex: number,
): StorySummary[] {
  const moving = stories.find((s) => s.id === storyId);
  // Las archivadas (sin posición) no están en el tablero.
  if ((moving?.position ?? null) === null || !moving) return stories;
  const active = (columnId: string) =>
    stories
      .filter(
        (s) =>
          s.columnId === columnId && s.position !== null && s.id !== storyId,
      )
      .sort((a, b) => (a.position ?? 0) - (b.position ?? 0));

  const source = active(moving.columnId);
  const target = moving.columnId === toColumnId ? source : active(toColumnId);
  const index = Math.max(0, Math.min(toIndex, target.length));
  const reordered = [
    ...target.slice(0, index),
    { ...moving, columnId: toColumnId },
    ...target.slice(index),
  ];

  const updated = new Map<string, StorySummary>();
  if (moving.columnId !== toColumnId) {
    source.forEach((s, position) => updated.set(s.id, { ...s, position }));
  }
  reordered.forEach((s, position) => updated.set(s.id, { ...s, position }));
  return stories.map((s) => updated.get(s.id) ?? s);
}

/**
 * Índice de destino al soltar: antes de `beforeStoryId` o, si es `null`, justo después de
 * `afterStoryId` (la última tarjeta visible del carril) o al final de la columna.
 */
export function dropIndex(
  columnCards: StorySummary[],
  draggedId: string,
  target: { beforeStoryId: string | null; afterStoryId?: string | null },
): number {
  const others = columnCards.filter((s) => s.id !== draggedId);
  if (target.beforeStoryId !== null) {
    const index = others.findIndex((s) => s.id === target.beforeStoryId);
    if (index !== -1) return index;
  }
  if (target.afterStoryId) {
    const index = others.findIndex((s) => s.id === target.afterStoryId);
    if (index !== -1) return index + 1;
  }
  return others.length;
}

export type KeyMove = "left" | "right" | "up" | "down";

/**
 * Destino de un movimiento por teclado, o `null` si no hay a dónde ir. Arriba/abajo
 * intercambia con la tarjeta vecina *visible* del mismo carril (`cellCards`), para que los
 * filtros y carriles no produzcan saltos invisibles. Izquierda/derecha la lleva al final de la
 * columna contigua.
 */
export function keyboardTarget(
  story: StorySummary,
  columns: Column[],
  byColumn: Map<string, StorySummary[]>,
  cellCards: StorySummary[],
  move: KeyMove,
): { columnId: string; index: number } | null {
  if (move === "up" || move === "down") {
    const at = cellCards.findIndex((s) => s.id === story.id);
    const neighbor = cellCards[at + (move === "up" ? -1 : 1)];
    const index = neighbor?.position ?? null;
    if (at === -1 || index === null) return null;
    return { columnId: story.columnId, index };
  }
  const columnIndex = columns.findIndex((c) => c.id === story.columnId);
  const next = columns[columnIndex + (move === "left" ? -1 : 1)];
  if (columnIndex === -1 || !next) return null;
  return { columnId: next.id, index: byColumn.get(next.id)?.length ?? 0 };
}
