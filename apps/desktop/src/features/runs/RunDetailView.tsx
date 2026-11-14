import { useCallback, useEffect, useId, useRef } from "react";
import type { RunDetail } from "../../contracts/generated/RunDetail";
import type { SnapshotStoryRef } from "../../contracts/generated/SnapshotStoryRef";
import { getRun } from "../../shared/ipc/commands";
import { useRefreshSignal } from "../../shared/ipc/useAgentChanges";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { formatDateTime } from "../../shared/ui/format";
import { MarkdownPreview } from "../../shared/ui/MarkdownPreview";
import { CATEGORY_LABEL } from "../columns/categories";
import { PRIORITY_LABEL } from "../stories/priorities";
import { CAPABILITY_LABEL } from "../team/TeamView";
import {
  NOT_AVAILABLE,
  RUN_STATE_LABEL,
  describeRunEvent,
  formatCost,
  formatTokens,
  runStateBadgeClass,
} from "./runFormat";

interface Props {
  projectId: string;
  runId: string;
  /** Cambia tras crear o cancelar, o al recargar la historia: vuelve a consultar. */
  refreshSignal: number;
  /** Mueve el foco al encabezado al cargar (se abrió desde la lista). */
  focusOnLoad: boolean;
  onClose: () => void;
}

/**
 * Detalle de un intento (HU-017). Todo sale de `getRun`: la instantánea es la copia congelada
 * al preparar el intento, nunca la historia actual.
 */
export function RunDetailView({
  projectId,
  runId,
  refreshSignal,
  focusOnLoad,
  onClose,
}: Props) {
  const id = useId();
  const load = useCallback(() => getRun(projectId, runId), [projectId, runId]);
  const { state, run } = useIpcResource(load);
  const reload = useCallback(() => {
    run(load, { keepData: true });
  }, [run, load]);
  useRefreshSignal(refreshSignal, reload);

  const titleRef = useRef<HTMLHeadingElement>(null);
  const focused = useRef(!focusOnLoad);
  useEffect(() => {
    if (state.status !== "loading" && !focused.current) {
      focused.current = true;
      titleRef.current?.focus();
    }
  }, [state.status]);

  return (
    <section
      className="run-detail"
      aria-labelledby={`${id}-title`}
      onKeyDown={(event) => {
        if (event.key === "Escape") onClose();
      }}
    >
      <div className="repo__summary">
        <h3
          id={`${id}-title`}
          className="relations__title"
          ref={titleRef}
          tabIndex={-1}
        >
          {state.status === "ready"
            ? `Detalle del intento #${String(state.data.run.number)}`
            : "Detalle del intento"}
        </h3>
        <button
          type="button"
          className="button button--secondary"
          onClick={onClose}
        >
          Cerrar detalle
        </button>
      </div>

      {state.status === "loading" && (
        <p className="panel__muted" role="status">
          Cargando intento…
        </p>
      )}
      {state.status === "error" && (
        <div>
          <p className="panel__error" role="alert">
            No se pudo cargar el intento: {state.message}
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
      {state.status === "ready" && <DetailBody detail={state.data} />}
    </section>
  );
}

function storyRef(ref: SnapshotStoryRef): string {
  return `#${String(ref.number)} ${ref.title} (${ref.accepted ? "aceptada" : "sin aceptar"})`;
}

function DetailBody({ detail }: { detail: RunDetail }) {
  const { run, snapshot } = detail;
  const { story, agent } = snapshot;
  return (
    <div className="run-detail__body">
      <p>
        <span className={runStateBadgeClass(run.state)}>
          {RUN_STATE_LABEL[run.state]}
        </span>{" "}
        Agente A-{run.agentNumber} {run.agentName}.
        {run.endedAt !== null &&
          ` Terminado el ${formatDateTime(run.endedAt)}.`}
        {run.endReason !== null && ` Motivo: ${run.endReason}`}
      </p>

      {!detail.snapshotIntact && (
        <p className="run-detail__warning" role="alert">
          La instantánea no coincide con su huella: no es fiable.
        </p>
      )}

      <div className="run-detail__snapshot">
        <h4 className="relations__title">
          Instantánea congelada al preparar el intento (
          <time dateTime={snapshot.createdAt}>
            {formatDateTime(snapshot.createdAt)}
          </time>
          )
        </h4>
        <p className="field__hint">
          Es una copia de lo que recibiría el agente: editar la historia después
          no la cambia.
        </p>

        <p className="item-list__name">
          <span className="story-number">#{story.number}</span> {story.title}
        </p>
        <p className="field__hint">
          Prioridad {PRIORITY_LABEL[story.priority].toLowerCase()} · columna «
          {story.column.name}» ({CATEGORY_LABEL[story.column.category]}) ·
          revisión {story.revision}
        </p>

        <h5 className="run-detail__subtitle">Descripción</h5>
        <MarkdownPreview source={story.description} />

        <h5 className="run-detail__subtitle">Criterios de aceptación</h5>
        {snapshot.criteria.length === 0 ? (
          <p className="panel__muted">Sin criterios.</p>
        ) : (
          <ol className="review__criteria">
            {snapshot.criteria.map((c) => (
              <li key={c.number}>
                CA-{c.number}: {c.text}
              </li>
            ))}
          </ol>
        )}

        <h5 className="run-detail__subtitle">Contexto</h5>
        <dl className="facts facts--compact">
          <dt>Épica</dt>
          <dd>
            {snapshot.epic
              ? `E-${String(snapshot.epic.number)} ${snapshot.epic.title}`
              : "Sin épica"}
          </dd>
          <dt>Historia padre</dt>
          <dd>{snapshot.parent ? storyRef(snapshot.parent) : "Sin padre"}</dd>
          <dt>Prerequisitos</dt>
          <dd>
            {snapshot.prerequisites.length === 0
              ? "Ninguno"
              : snapshot.prerequisites.map(storyRef).join("; ")}
          </dd>
        </dl>

        <h5 className="run-detail__subtitle">Configuración del agente</h5>
        <dl className="facts facts--compact">
          <dt>Agente</dt>
          <dd>
            A-{agent.number} {agent.name}
          </dd>
          <dt>Rol</dt>
          <dd>{agent.role || NOT_AVAILABLE}</dd>
          <dt>Proveedor</dt>
          <dd>{agent.provider || NOT_AVAILABLE}</dd>
          <dt>Modelo</dt>
          <dd>{agent.model || NOT_AVAILABLE}</dd>
          <dt>Capacidades</dt>
          <dd>
            {agent.capabilities.length === 0
              ? "Ninguna"
              : agent.capabilities.map((c) => CAPABILITY_LABEL[c]).join(", ")}
          </dd>
        </dl>

        <h5 className="run-detail__subtitle">Repositorios</h5>
        {snapshot.repositories.length === 0 ? (
          <p className="panel__muted">Sin repositorios vinculados.</p>
        ) : (
          <ul className="relations__list">
            {snapshot.repositories.map((repo) => (
              <li key={repo.id} className="run-detail__repo">
                <span className="repo__path">{repo.path}</span>
                <span className="field__hint">
                  Commit:{" "}
                  <span className="input--mono">
                    {repo.head ?? NOT_AVAILABLE}
                  </span>{" "}
                  · Rama: {repo.branch ?? NOT_AVAILABLE}
                </span>
              </li>
            ))}
          </ul>
        )}

        <h5 className="run-detail__subtitle">Nota de la persona</h5>
        {snapshot.note === null ? (
          <p className="panel__muted">Sin nota.</p>
        ) : (
          <MarkdownPreview source={snapshot.note} />
        )}

        <h5 className="run-detail__subtitle">Instrucciones generadas</h5>
        <MarkdownPreview source={snapshot.instructions} />
      </div>

      <h4 className="relations__title">Uso</h4>
      <dl className="facts facts--compact">
        <dt>Tokens de entrada</dt>
        <dd>{formatTokens(run.usage.inputTokens)}</dd>
        <dt>Tokens de salida</dt>
        <dd>{formatTokens(run.usage.outputTokens)}</dd>
        <dt>Coste</dt>
        <dd>{formatCost(run.usage.costUsdMicros)}</dd>
      </dl>

      <h4 className="relations__title">Eventos</h4>
      {detail.eventsTruncated && (
        <p className="field__hint">
          Se muestran los últimos {detail.events.length} eventos: los anteriores
          no aparecen aquí.
        </p>
      )}
      {detail.events.length === 0 ? (
        <p className="panel__muted">Sin eventos.</p>
      ) : (
        <ol className="relations__list" aria-label="Eventos en orden">
          {detail.events.map((e) => (
            <li key={e.sequence} className="relations__item">
              <span className="item-list__meta">
                {e.sequence}.{" "}
                <time dateTime={e.at}>{formatDateTime(e.at)}</time>
              </span>
              <span>{describeRunEvent(e.event)}</span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
