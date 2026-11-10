// Datos de prueba del tablero (solo para pruebas).
import type { Column } from "../../contracts/generated/Column";
import type { StorySummary } from "../../contracts/generated/StorySummary";

export function column(
  id: string,
  position: number,
  extra: Partial<Column> = {},
): Column {
  return {
    id,
    projectId: "p1",
    name: id,
    category: "active",
    position,
    wipLimit: null,
    revision: 1,
    cardCount: 0,
    ...extra,
  };
}

export function card(
  id: string,
  columnId: string,
  position: number | null,
  extra: Partial<StorySummary> = {},
): StorySummary {
  return {
    id,
    projectId: "p1",
    number: Number(id.replace(/\D/g, "")) || 1,
    columnId,
    position,
    title: `Historia ${id}`,
    priority: "medium",
    archivedAt: position === null ? "2026-11-01T00:00:00Z" : null,
    acceptedAt: null,
    epicId: null,
    parentId: null,
    agentId: null,
    blockerReason: null,
    pendingPrerequisites: 0,
    criteriaCount: 1,
    revision: 1,
    ...extra,
  };
}
