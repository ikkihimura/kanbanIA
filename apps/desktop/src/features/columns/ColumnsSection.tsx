import { useCallback, useEffect, useRef, useState } from "react";
import {
  createColumn,
  listColumns,
  newRequestId,
  type UiError,
} from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { parseWip } from "./categories";
import { ColumnFields, type ColumnDraft } from "./ColumnFields";
import { ColumnItem, type FocusTarget } from "./ColumnItem";

interface Props {
  projectId: string;
}

const EMPTY_DRAFT: ColumnDraft = { name: "", category: "active", wip: "" };

export function ColumnsSection({ projectId }: Props) {
  const load = useCallback(() => listColumns(projectId), [projectId]);
  const { state, run } = useIpcResource(load);
  const titleRef = useRef<HTMLHeadingElement>(null);
  const [announcement, setAnnouncement] = useState("");
  const [draft, setDraft] = useState<ColumnDraft>(EMPTY_DRAFT);
  const [localError, setLocalError] = useState<UiError | null>(null);
  const requestId = useRef(newRequestId());
  const create = useMutation(createColumn);
  // Tras reordenar, devolver el foco al mismo botón de la columna movida.
  const pendingFocus = useRef<FocusTarget | null>(null);

  const reload = useCallback(() => {
    run(load, { keepData: true });
  }, [run, load]);

  useEffect(() => {
    const target = pendingFocus.current;
    if (state.status !== "ready" || !target) return;
    pendingFocus.current = null;
    requestAnimationFrame(() => {
      const find = (action: FocusTarget["action"]) =>
        document.querySelector<HTMLButtonElement>(
          `[data-column-action="${action}"][data-column-id="${target.id}"]`,
        );
      // En un extremo el mismo botón queda deshabilitado: usar el opuesto de la columna.
      const opposite = target.action === "up" ? "down" : "up";
      const button = [find(target.action), find(opposite)].find(
        (candidate) => candidate && !candidate.disabled,
      );
      (button ?? titleRef.current)?.focus();
    });
  }, [state]);

  async function handleCreate() {
    const wipLimit = parseWip(draft.wip);
    if (wipLimit === undefined) {
      setLocalError({
        message: "Escribe un número entero o déjalo vacío.",
        field: "wipLimit",
      });
      return;
    }
    const result = await create.mutate({
      requestId: requestId.current,
      projectId,
      name: draft.name,
      category: draft.category,
      wipLimit,
    });
    if (result.ok) {
      requestId.current = newRequestId();
      setDraft(EMPTY_DRAFT);
      setAnnouncement(`Columna «${result.value.name}» creada al final.`);
      reload();
    }
  }

  const columns = state.status === "ready" ? state.data : [];

  return (
    <section className="panel" aria-labelledby="columns-title">
      <h2
        id="columns-title"
        className="panel__title"
        ref={titleRef}
        tabIndex={-1}
      >
        Columnas
      </h2>
      <p className="panel__hint">
        El nombre es libre; el tipo de etapa decide las reglas. Una columna
        «Hecho» solo recibe tarjetas aceptadas.
      </p>

      <div role="status" aria-live="polite" className="visually-hidden">
        {announcement}
      </div>
      <div role="status" aria-live="polite">
        {state.status === "loading" && (
          <p className="panel__muted">Cargando columnas…</p>
        )}
        {state.status === "error" && (
          <p className="panel__error">{state.message}</p>
        )}
      </div>

      {columns.length > 0 && (
        <ol className="item-list">
          {columns.map((column, index) => (
            <ColumnItem
              key={column.id}
              column={column}
              columns={columns}
              isFirst={index === 0}
              isLast={index === columns.length - 1}
              onChanged={(message, focus) => {
                setAnnouncement(message);
                pendingFocus.current = focus ?? null;
                if (!focus) titleRef.current?.focus();
                reload();
              }}
              onStale={reload}
            />
          ))}
        </ol>
      )}

      <form
        className="form-row form-row--spaced"
        aria-label="Nueva columna"
        onSubmit={(event) => {
          event.preventDefault();
          void handleCreate();
        }}
      >
        <ColumnFields
          draft={draft}
          error={localError ?? create.error}
          onChange={(next) => {
            setDraft(next);
            setLocalError(null);
            requestId.current = newRequestId();
            create.reset();
          }}
        />
        <button type="submit" className="button" disabled={create.pending}>
          Añadir columna
        </button>
      </form>
    </section>
  );
}
