import { useCallback, useEffect } from "react";
import type { AttentionItem } from "../../contracts/generated/AttentionItem";
import { listAttention } from "../../shared/ipc/commands";
import { useAgentChanges } from "../../shared/ipc/useAgentChanges";
import { useIpcResource } from "../../shared/ipc/useIpcResource";

interface Props {
  projectId: string;
  onOpenStory: (storyId: string) => void;
}

function reasons(item: AttentionItem): string[] {
  const out: string[] = [];
  if (item.blockerReason) out.push(`Bloqueada: ${item.blockerReason}`);
  if (item.pendingDeliveries > 0) {
    out.push(
      item.pendingDeliveries === 1
        ? "1 entrega pendiente de revisión"
        : `${String(item.pendingDeliveries)} entregas pendientes de revisión`,
    );
  }
  if (item.pendingPrerequisites > 0) {
    out.push(
      item.pendingPrerequisites === 1
        ? "1 prerequisito sin aceptar"
        : `${String(item.pendingPrerequisites)} prerequisitos sin aceptar`,
    );
  }
  return out;
}

/** Bandeja de atención del proyecto: una fila por historia, con todos sus motivos. */
export function AttentionSection({ projectId, onOpenStory }: Props) {
  const load = useCallback(() => listAttention(projectId), [projectId]);
  const { state, run } = useIpcResource(load);
  // Al volver a la ventana se actualiza (los cambios en otras vistas recargan al montarse).
  useEffect(() => {
    const refresh = () => {
      run(load, { keepData: true });
    };
    window.addEventListener("focus", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
    };
  }, [run, load]);
  // Bloqueos y entregas reportados por agentes (HU-015) aparecen sin esperar.
  useAgentChanges(projectId, () => {
    run(load, { keepData: true });
  });
  const items = state.status === "ready" ? state.data : [];

  return (
    <section className="panel attention" aria-labelledby="attention-title">
      <h2 id="attention-title" className="panel__title">
        Atención{state.status === "ready" ? ` (${String(items.length)})` : ""}
      </h2>
      {state.status === "loading" && <p className="panel__muted">Cargando…</p>}
      {state.status === "error" && (
        <p className="panel__error">{state.message}</p>
      )}
      {state.status === "ready" && items.length === 0 && (
        <p className="panel__muted">
          Nada pendiente: sin bloqueos ni entregas por revisar.
        </p>
      )}
      {items.length > 0 && (
        <ul className="item-list">
          {items.map((item) => (
            <li key={item.storyId}>
              <button
                type="button"
                className="item-list__button"
                onClick={() => {
                  onOpenStory(item.storyId);
                }}
              >
                <span className="item-list__name">
                  <span className="story-number">#{item.number}</span>{" "}
                  {item.title}
                </span>
                <span className="item-list__meta">
                  {item.columnName} · {reasons(item).join(" · ")}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
