import { useCallback, useId } from "react";
import type { ActivityEntry } from "../../contracts/generated/ActivityEntry";
import { listStoryActivity } from "../../shared/ipc/commands";
import { useRefreshSignal } from "../../shared/ipc/useAgentChanges";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { formatDateTime, formatTime } from "../../shared/ui/format";

interface Props {
  projectId: string;
  storyId: string;
  /** Cambia cuando llega un aviso `agent-change` de esta historia: vuelve a consultar. */
  refreshSignal?: number;
}

/**
 * Acción reportada, en texto. `detail` es una referencia corta que genera la app («#12»,
 * «E2»), nunca texto del agente; aun así se muestra como texto plano.
 */
export function describeActivity(entry: ActivityEntry): string {
  switch (entry.kind) {
    case "claimed":
      return "Reclamó";
    case "renewed":
      return "Renovó el reclamo";
    case "blocker_reported":
      return "Reportó un bloqueo";
    case "blocker_cleared":
      return "Quitó su bloqueo";
    case "subtask_created":
      return entry.detail === null
        ? "Creó una subtarea"
        : `Creó la subtarea ${entry.detail}`;
    case "delivered":
      return entry.detail === null
        ? "Registró una entrega"
        : `Entregó ${entry.detail}`;
  }
}

/** HH:MM si es de hoy (hora local); si no, fecha y hora. */
function formatWhen(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toDateString() === new Date().toDateString()
    ? formatTime(iso)
    : formatDateTime(iso);
}

function When({ iso }: { iso: string }) {
  return (
    <time dateTime={iso} title={formatDateTime(iso)}>
      {formatWhen(iso)}
    </time>
  );
}

/**
 * Actividad del agente en la historia (HU-015). Todo lo que se ve aquí es lo que el agente
 * dijo haber hecho por MCP: la app no lo comprueba ni sabe si su proceso sigue vivo.
 */
export function AgentActivityPanel({
  projectId,
  storyId,
  refreshSignal,
}: Props) {
  const id = useId();
  const load = useCallback(
    () => listStoryActivity({ projectId, storyId }),
    [projectId, storyId],
  );
  const { state, run } = useIpcResource(load);
  const refresh = useCallback(() => {
    run(load, { keepData: true });
  }, [run, load]);
  useRefreshSignal(refreshSignal, refresh);

  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="panel__title">
        Actividad del agente
      </h2>
      <p className="field__hint" id={`${id}-note`}>
        Reportado por el agente: la app registra lo que el agente dice haber
        hecho, pero no lo comprueba.
      </p>

      {state.status === "loading" && (
        <p className="panel__muted" role="status">
          Cargando actividad…
        </p>
      )}
      {state.status === "error" && (
        <div>
          <p className="panel__error" role="alert">
            No se pudo cargar la actividad del agente: {state.message}
          </p>
          <button
            type="button"
            className="button"
            onClick={() => {
              run(load);
            }}
          >
            Reintentar
          </button>
        </div>
      )}
      {state.status === "ready" && state.data.entries.length === 0 && (
        <p className="panel__muted">
          Ningún agente ha reportado actividad en esta historia.
        </p>
      )}
      {state.status === "ready" && state.data.entries.length > 0 && (
        <>
          {state.data.lastReportedAt !== null && (
            <>
              <p>
                Última interacción reportada:{" "}
                <strong>
                  <When iso={state.data.lastReportedAt} />
                </strong>
              </p>
              <p className="field__hint">
                Es la hora del último reporte; no indica que el agente siga
                ocupándose de la historia.
              </p>
            </>
          )}
          <ol
            className="relations__list"
            aria-label="Acciones reportadas, de la más reciente a la más antigua"
            aria-describedby={`${id}-note`}
          >
            {state.data.entries.map((entry, index) => (
              <li
                // Una misma acción puede repetirse a la misma hora: el índice desempata.
                key={`${entry.at}-${entry.kind}-${String(index)}`}
                className="relations__item"
              >
                <strong>
                  A-{entry.agent.agentNumber} {entry.agent.agentName}
                </strong>
                <span>{describeActivity(entry)}</span>
                <span className="item-list__meta">
                  <When iso={entry.at} />
                </span>
              </li>
            ))}
          </ol>
        </>
      )}
    </section>
  );
}
