import { useCallback, useEffect, useRef, type KeyboardEvent } from "react";
import type { StorySummary } from "../../contracts/generated/StorySummary";
import { getStory } from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { AcceptForm } from "../review/ReviewPanel";

interface Props {
  projectId: string;
  storyId: string;
  columnName: string;
  /** Columna «hecho» y posición donde se soltó: se acepta y coloca en una sola operación. */
  target: { columnId: string; toIndex: number };
  /** Aceptada: el tablero la coloca en la columna y posición donde se soltó. */
  onAccepted: (accepted: StorySummary) => void;
  onCancel: () => void;
}

/**
 * Soltar una tarjeta en una columna «hecho» abre la aceptación humana de HU-009: la tarjeta no
 * se mueve hasta que se marcan los criterios y se acepta. Cancelar la deja donde estaba.
 */
export function AcceptDialog({
  projectId,
  storyId,
  columnName,
  target,
  onAccepted,
  onCancel,
}: Props) {
  const load = useCallback(
    () => getStory(projectId, storyId),
    [projectId, storyId],
  );
  const { state } = useIpcResource(load);
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  return (
    <div className="modal-backdrop">
      <div
        ref={dialogRef}
        className="modal modal--wide panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="accept-dialog-title"
        tabIndex={-1}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.stopPropagation();
            onCancel();
            return;
          }
          if (event.key === "Tab") trapFocus(event);
        }}
      >
        <h2 id="accept-dialog-title" className="panel__title">
          {state.status === "ready"
            ? `Aceptar #${String(state.data.number)} ${state.data.title}`
            : "Aceptar historia"}
        </h2>
        <p>
          Para entrar en «{columnName}» la historia debe aceptarse. La tarjeta
          no se mueve hasta que la aceptes.
        </p>
        {state.status === "loading" && (
          <p className="panel__muted" role="status">
            Cargando criterios…
          </p>
        )}
        {state.status === "error" && (
          <p className="panel__error" role="alert">
            {state.message}
          </p>
        )}
        {state.status === "ready" && (
          <AcceptForm
            story={state.data}
            locked={false}
            target={target}
            onAccepted={onAccepted}
          />
        )}
        <div className="repo__actions">
          <button
            type="button"
            className="button button--secondary"
            onClick={onCancel}
          >
            Cancelar
          </button>
        </div>
      </div>
    </div>
  );
}

function trapFocus(event: KeyboardEvent<HTMLElement>) {
  const focusable = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>(
      "button:not(:disabled), input:not(:disabled)",
    ),
  );
  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last?.focus();
  } else if (
    !event.shiftKey &&
    (document.activeElement === last ||
      document.activeElement === event.currentTarget)
  ) {
    event.preventDefault();
    first?.focus();
  }
}
