import { useEffect, useId, useState } from "react";
import type { Column } from "../../contracts/generated/Column";
import {
  deleteColumn,
  reorderColumn,
  updateColumn,
  type UiError,
} from "../../shared/ipc/commands";
import { useMutation } from "../../shared/ipc/useMutation";
import { CATEGORY_LABEL, cardsLabel, parseWip } from "./categories";
import { ColumnFields, type ColumnDraft } from "./ColumnFields";

export interface FocusTarget {
  id: string;
  action: "up" | "down";
}

interface Props {
  column: Column;
  columns: Column[];
  isFirst: boolean;
  isLast: boolean;
  onChanged: (announcement: string, focus?: FocusTarget) => void;
  /** Datos desactualizados (conflicto o ya no existe): recargar. */
  onStale: () => void;
}

type Mode = "idle" | "editing" | "deleting";

export function ColumnItem({
  column,
  columns,
  isFirst,
  isLast,
  onChanged,
  onStale,
}: Props) {
  const [mode, setMode] = useState<Mode>("idle");
  const [draft, setDraft] = useState<ColumnDraft>(toDraft(column));
  const [localError, setLocalError] = useState<UiError | null>(null);
  const [destination, setDestination] = useState("");
  const update = useMutation(updateColumn);
  const reorder = useMutation(reorderColumn);
  const remove = useMutation(deleteColumn);
  const destinationId = useId();

  const overLimit =
    column.wipLimit !== null && column.cardCount > column.wipLimit;
  // Destinos válidos: otras columnas que no sean «hecho».
  const destinations = columns.filter(
    (c) => c.id !== column.id && c.category !== "done",
  );

  const staleCode =
    update.error?.code ?? reorder.error?.code ?? remove.error?.code;
  useEffect(() => {
    if (staleCode === "conflict" || staleCode === "not_found") onStale();
  }, [staleCode, onStale]);

  async function handleMove(direction: FocusTarget["action"]) {
    const toIndex = column.position + (direction === "up" ? -1 : 1);
    const result = await reorder.mutate({
      projectId: column.projectId,
      id: column.id,
      expectedRevision: column.revision,
      toIndex,
    });
    if (result.ok) {
      onChanged(
        `«${column.name}» movida a la posición ${String(toIndex + 1)} de ${String(columns.length)}.`,
        { id: column.id, action: direction },
      );
    }
  }

  async function handleSave() {
    const wipLimit = parseWip(draft.wip);
    if (wipLimit === undefined) {
      setLocalError({
        message: "Escribe un número entero o déjalo vacío.",
        field: "wipLimit",
      });
      return;
    }
    const result = await update.mutate({
      projectId: column.projectId,
      id: column.id,
      expectedRevision: column.revision,
      name: draft.name,
      category: draft.category,
      wipLimit,
    });
    if (result.ok) {
      setMode("idle");
      onChanged(`Columna «${result.value.name}» actualizada.`);
    }
  }

  async function handleDelete() {
    const result = await remove.mutate({
      projectId: column.projectId,
      id: column.id,
      expectedRevision: column.revision,
      ...(column.cardCount > 0 && destination
        ? { destinationColumnId: destination }
        : {}),
    });
    if (result.ok) {
      const target = destinations.find((c) => c.id === destination);
      onChanged(
        column.cardCount > 0 && target
          ? `Columna «${column.name}» eliminada; ${cardsLabel(column.cardCount)} movidas a «${target.name}».`
          : `Columna «${column.name}» eliminada.`,
      );
    }
  }

  return (
    <li className="column-item">
      <div className="column-item__summary">
        <span className="item-list__name">{column.name}</span>
        <span className="badge">{CATEGORY_LABEL[column.category]}</span>
        <span className="item-list__meta">
          {cardsLabel(column.cardCount)}
          {" · "}
          {column.wipLimit === null
            ? "sin límite WIP"
            : `límite WIP ${String(column.wipLimit)}`}
        </span>
        {overLimit && (
          <span className="badge badge--warn">
            Excede el límite: no admite nuevas tarjetas
          </span>
        )}
      </div>

      {reorder.error && (
        <p className="field__error" role="alert">
          {reorder.error.message}
        </p>
      )}

      {mode === "idle" && (
        <div className="repo__actions">
          <button
            type="button"
            className="button button--secondary"
            data-column-id={column.id}
            data-column-action="up"
            disabled={isFirst || reorder.pending}
            aria-label={`Subir ${column.name}`}
            onClick={() => {
              void handleMove("up");
            }}
          >
            ↑ Subir
          </button>
          <button
            type="button"
            className="button button--secondary"
            data-column-id={column.id}
            data-column-action="down"
            disabled={isLast || reorder.pending}
            aria-label={`Bajar ${column.name}`}
            onClick={() => {
              void handleMove("down");
            }}
          >
            ↓ Bajar
          </button>
          <button
            type="button"
            className="button button--secondary"
            onClick={() => {
              setDraft(toDraft(column));
              setLocalError(null);
              update.reset();
              setMode("editing");
            }}
          >
            Editar
          </button>
          <button
            type="button"
            className="button button--secondary"
            onClick={() => {
              setDestination(destinations[0]?.id ?? "");
              remove.reset();
              setMode("deleting");
            }}
          >
            Eliminar
          </button>
        </div>
      )}

      {mode === "editing" && (
        <form
          className="form-row"
          aria-label={`Editar columna ${column.name}`}
          onSubmit={(event) => {
            event.preventDefault();
            void handleSave();
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") setMode("idle");
          }}
        >
          <ColumnFields
            draft={draft}
            error={localError ?? update.error}
            autoFocus
            onChange={(next) => {
              setDraft(next);
              setLocalError(null);
              update.reset();
            }}
          />
          <button type="submit" className="button" disabled={update.pending}>
            Guardar
          </button>
          <button
            type="button"
            className="button button--secondary"
            onClick={() => {
              setMode("idle");
            }}
          >
            Cancelar
          </button>
        </form>
      )}

      {mode === "deleting" && (
        <div
          className="repo__confirm"
          role="group"
          aria-label={`Eliminar columna ${column.name}`}
          onKeyDown={(event) => {
            if (event.key === "Escape") setMode("idle");
          }}
        >
          {column.cardCount === 0 ? (
            <p>¿Eliminar la columna vacía «{column.name}»?</p>
          ) : (
            <div className="field">
              <label htmlFor={destinationId}>
                «{column.name}» tiene {cardsLabel(column.cardCount)}. ¿A qué
                columna las movemos?
              </label>
              <select
                id={destinationId}
                value={destination}
                autoFocus
                onChange={(event) => {
                  setDestination(event.target.value);
                  remove.reset();
                }}
              >
                {destinations.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                    {c.wipLimit !== null
                      ? ` (${String(c.cardCount)}/${String(c.wipLimit)})`
                      : ""}
                  </option>
                ))}
              </select>
            </div>
          )}
          {remove.error && (
            <p className="field__error" role="alert">
              {remove.error.message}
            </p>
          )}
          <div className="repo__actions">
            <button
              type="button"
              className="button button--danger"
              autoFocus={column.cardCount === 0}
              disabled={
                remove.pending || (column.cardCount > 0 && !destination)
              }
              onClick={() => {
                void handleDelete();
              }}
            >
              {column.cardCount > 0 ? "Mover y eliminar" : "Eliminar"}
            </button>
            <button
              type="button"
              className="button button--secondary"
              onClick={() => {
                setMode("idle");
              }}
            >
              Cancelar
            </button>
          </div>
        </div>
      )}
    </li>
  );
}

function toDraft(column: Column): ColumnDraft {
  return {
    name: column.name,
    category: column.category,
    wip: column.wipLimit === null ? "" : String(column.wipLimit),
  };
}
