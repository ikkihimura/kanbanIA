import { useCallback, useEffect } from "react";
import type { AttentionItem } from "../../contracts/generated/AttentionItem";
import type { WaitingApproval } from "../../contracts/generated/WaitingApproval";
import { listAttention, listWaitingApprovals } from "../../shared/ipc/commands";
import { useAgentChanges } from "../../shared/ipc/useAgentChanges";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useRunChanges } from "../runs/useRunChanges";

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

/** Una fila de la bandeja: una historia con todos sus motivos. */
export interface AttentionRow {
  storyId: string;
  number: number;
  title: string;
  /** `null` si la historia solo espera permiso (la lista de permisos no trae la columna). */
  columnName: string | null;
  reasons: string[];
}

/**
 * Une la bandeja del núcleo con los permisos pendientes (HU-020): una sola fila por historia,
 * con «Espera tu permiso (N)» como primer motivo. Las que esperan permiso van primero (la más
 * antigua antes); el resto conserva el orden del núcleo.
 */
export function attentionRows(
  items: readonly AttentionItem[],
  waiting: readonly WaitingApproval[],
): AttentionRow[] {
  const pending = new Map<
    string,
    { number: number; title: string; count: number; since: string }
  >();
  for (const w of waiting) {
    const current = pending.get(w.storyId);
    if (current) {
      current.count += w.pending;
      if (w.since < current.since) current.since = w.since;
    } else {
      pending.set(w.storyId, {
        number: w.storyNumber,
        title: w.storyTitle,
        count: w.pending,
        since: w.since,
      });
    }
  }
  const byStory = new Map(items.map((item) => [item.storyId, item]));
  const first = [...pending.entries()]
    .filter(([, p]) => p.count > 0)
    .sort(([, a], [, b]) =>
      a.since < b.since ? -1 : a.since > b.since ? 1 : 0,
    )
    .map(([storyId, p]): AttentionRow => {
      const item = byStory.get(storyId);
      return {
        storyId,
        number: item?.number ?? p.number,
        title: item?.title ?? p.title,
        columnName: item?.columnName ?? null,
        reasons: [
          `Espera tu permiso (${String(p.count)})`,
          ...(item ? reasons(item) : []),
        ],
      };
    });
  const shown = new Set(first.map((row) => row.storyId));
  const rest = items
    .filter((item) => !shown.has(item.storyId))
    .map((item): AttentionRow => ({
      storyId: item.storyId,
      number: item.number,
      title: item.title,
      columnName: item.columnName,
      reasons: reasons(item),
    }));
  return [...first, ...rest];
}

const NO_WAITING: WaitingApproval[] = [];

/** Bandeja de atención del proyecto: una fila por historia, con todos sus motivos. */
export function AttentionSection({ projectId, onOpenStory }: Props) {
  const load = useCallback(() => listAttention(projectId), [projectId]);
  const { state, run } = useIpcResource(load);
  // Permisos pendientes (HU-020): salen de la base. Si no cargan, la bandeja sigue sin ellos.
  const loadWaiting = useCallback(
    () => listWaitingApprovals(projectId),
    [projectId],
  );
  const waiting = useIpcResource(loadWaiting);
  const runWaiting = waiting.run;
  // Al volver a la ventana se actualiza (los cambios en otras vistas recargan al montarse).
  useEffect(() => {
    const refresh = () => {
      run(load, { keepData: true });
      runWaiting(loadWaiting, { keepData: true });
    };
    window.addEventListener("focus", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
    };
  }, [run, load, runWaiting, loadWaiting]);
  // Bloqueos y entregas reportados por agentes (HU-015) aparecen sin esperar.
  useAgentChanges(projectId, () => {
    run(load, { keepData: true });
  });
  // Peticiones de permiso nuevas, decididas o caducadas: aviso `run-change` del supervisor.
  useRunChanges(() => {
    runWaiting(loadWaiting, { keepData: true });
  });
  const items =
    state.status === "ready"
      ? attentionRows(
          state.data,
          waiting.state.status === "ready" ? waiting.state.data : NO_WAITING,
        )
      : [];

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
          {waiting.state.status === "error"
            ? "Nada pendiente: sin bloqueos ni entregas por revisar."
            : "Nada pendiente: sin bloqueos, entregas por revisar ni permisos que decidir."}
        </p>
      )}
      {state.status === "ready" && waiting.state.status === "error" && (
        <p className="field__hint">
          No se pudieron cargar los permisos pendientes de los agentes: la
          bandeja se muestra sin ellos.
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
                  {[item.columnName, ...item.reasons]
                    .filter((part): part is string => part !== null)
                    .join(" · ")}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
