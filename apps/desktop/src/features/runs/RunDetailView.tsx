import { useCallback, useEffect, useId, useRef, type RefObject } from "react";
import type { AcpSessionInfo } from "../../contracts/generated/AcpSessionInfo";
import type { RunDetail } from "../../contracts/generated/RunDetail";
import type { RunExecutor } from "../../contracts/generated/RunExecutor";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import type { SnapshotStoryRef } from "../../contracts/generated/SnapshotStoryRef";
import { getRun } from "../../shared/ipc/commands";
import { useRefreshSignal } from "../../shared/ipc/useAgentChanges";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { formatDateTime } from "../../shared/ui/format";
import { MarkdownPreview } from "../../shared/ui/MarkdownPreview";
import { CATEGORY_LABEL } from "../columns/categories";
import { PRIORITY_LABEL } from "../stories/priorities";
import { CAPABILITY_LABEL } from "../team/TeamView";
import { ApprovalsSection } from "./ApprovalsSection";
import { RunOutputPanel } from "./RunOutputPanel";
import {
  ACP_AGENT_NAME,
  NOT_AVAILABLE,
  SCENARIO_LABEL,
  acpAgentOf,
  canStartRun,
  describeAgentIdentity,
  describeAgentMode,
  describeAgentModel,
  describeCapabilities,
  describeRunEvent,
  executorLabel,
  formatCost,
  formatTokens,
  isActiveRun,
  isCancelPending,
  runStateBadgeClass,
  runStatusLabel,
} from "./runFormat";
import { StartRunForm } from "./StartRunForm";
import { WorktreeSection } from "./WorktreeSection";

/** Dónde poner el foco al cargar: el encabezado, el formulario de inicio o ninguno. */
export type DetailFocus = "title" | "start" | null;

interface Props {
  projectId: string;
  runId: string;
  /**
   * Cambia tras crear, iniciar o cancelar, al recargar la historia o con un aviso
   * `run-change` del intento: vuelve a consultar (detalle y salida).
   */
  refreshSignal: number;
  /** Foco al cargar (se abrió desde la lista o desde «Iniciar…»). */
  focusOnLoad: DetailFocus;
  onClose: () => void;
  /** Se pidió iniciar el intento: el panel recarga la lista y lo anuncia. */
  onStarted: (run: RunSummary, executor: RunExecutor) => void;
  /** El intento cambió desde aquí (o el núcleo dijo que ya había cambiado). */
  onChanged: () => void;
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
  onStarted,
  onChanged,
}: Props) {
  const id = useId();
  const load = useCallback(() => getRun(projectId, runId), [projectId, runId]);
  const { state, run } = useIpcResource(load);
  const reload = useCallback(() => {
    run(load, { keepData: true });
  }, [run, load]);
  useRefreshSignal(refreshSignal, reload);

  const titleRef = useRef<HTMLHeadingElement>(null);
  const startFocusRef = useRef<HTMLInputElement>(null);
  const focused = useRef(focusOnLoad === null);
  const focusTarget = useRef(focusOnLoad);
  useEffect(() => {
    if (state.status !== "loading" && !focused.current) {
      focused.current = true;
      // Sin formulario de inicio (ya no está en cola), el foco va al encabezado.
      const target =
        focusTarget.current === "start" ? startFocusRef.current : null;
      (target ?? titleRef.current)?.focus();
    }
  }, [state.status]);

  return (
    <section
      className="run-detail"
      aria-labelledby={`${id}-title`}
      onKeyDown={(event) => {
        if (event.key === "Escape" && !event.defaultPrevented) onClose();
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
      {state.status === "ready" && (
        <DetailBody
          detail={state.data}
          projectId={projectId}
          refreshSignal={refreshSignal}
          startFocusRef={startFocusRef}
          onStarted={(started, executor) => {
            // El formulario desaparece: el foco va al encabezado del detalle.
            titleRef.current?.focus();
            reload();
            onStarted(started, executor);
          }}
          onChanged={() => {
            reload();
            onChanged();
          }}
        />
      )}
    </section>
  );
}

function storyRef(ref: SnapshotStoryRef): string {
  return `#${String(ref.number)} ${ref.title} (${ref.accepted ? "aceptada" : "sin aceptar"})`;
}

interface BodyProps {
  detail: RunDetail;
  projectId: string;
  refreshSignal: number;
  startFocusRef: RefObject<HTMLInputElement | null>;
  onStarted: (run: RunSummary, executor: RunExecutor) => void;
  onChanged: () => void;
}

function DetailBody({
  detail,
  projectId,
  refreshSignal,
  startFocusRef,
  onStarted,
  onChanged,
}: BodyProps) {
  const id = useId();
  const { run, snapshot } = detail;
  const { story, agent } = snapshot;
  const request = detail.startRequest;
  const requestRepo =
    request && snapshot.repositories.find((r) => r.id === request.repositoryId);
  const executor = request?.executor ?? null;
  const acpAgent = executor === null ? null : acpAgentOf(executor);
  const acp = acpAgent !== null;
  const agentName = acpAgent === null ? "" : ACP_AGENT_NAME[acpAgent];
  const session = lastAgentSession(detail);
  const live =
    run.state === "preparing" ||
    run.state === "running" ||
    run.state === "waiting_approval";
  return (
    <div className="run-detail__body">
      <p>
        <span className={runStateBadgeClass(run.state)}>
          {runStatusLabel(run)}
        </span>{" "}
        Agente asignado al intento: A-{run.agentNumber} {run.agentName}
        {executor === null
          ? "."
          : acp
            ? ` (lo ejecuta ${agentName} por ACP con las instrucciones de la instantánea).`
            : " (no participa: lo ejecuta el proceso de prueba)."}
        {run.endedAt !== null &&
          ` Terminado el ${formatDateTime(run.endedAt)}.`}
        {run.endReason !== null && ` Motivo: ${run.endReason}`}
      </p>

      {/* Arriba del detalle y siempre en el mismo sitio: una petición pendiente se ve sin
          desplazarse, y decidirla no la mueve (no se pierden el foco ni el aviso). */}
      {detail.approvals.length > 0 && (
        <ApprovalsSection
          projectId={projectId}
          approvals={detail.approvals}
          onChanged={onChanged}
        />
      )}

      {!detail.snapshotIntact && (
        <p className="run-detail__warning" role="alert">
          La instantánea no coincide con su huella: no es fiable.
        </p>
      )}

      <section className="run-execution" aria-label="Ejecución">
        <h4 className="relations__title">Ejecución</h4>
        <dl className="facts facts--compact">
          <dt>Ejecutor</dt>
          <dd>
            {request ? (
              <span className="badge">{executorLabel(request.executor)}</span>
            ) : (
              "Se elige al iniciar"
            )}
          </dd>
          {request && (
            <>
              {request.executor.kind === "testProcess" && (
                <>
                  <dt>Escenario</dt>
                  <dd>{SCENARIO_LABEL[request.executor.scenario]}</dd>
                </>
              )}
              <dt>Repositorio</dt>
              <dd className="repo__path">
                {requestRepo ? requestRepo.path : NOT_AVAILABLE}
              </dd>
              <dt>Inicio pedido</dt>
              <dd>
                <time dateTime={request.requestedAt}>
                  {formatDateTime(request.requestedAt)}
                </time>
              </dd>
              {request.dispatchedAt !== null && (
                <>
                  <dt>Salió de la cola</dt>
                  <dd>
                    <time dateTime={request.dispatchedAt}>
                      {formatDateTime(request.dispatchedAt)}
                    </time>
                  </dd>
                </>
              )}
            </>
          )}
        </dl>
        {executor?.kind === "testProcess" && (
          <p className="field__hint">
            Ningún agente ni IA trabaja en este intento: el ejecutor es un
            proceso de prueba incluido en la app.
          </p>
        )}
        {acp && (
          <p className="field__hint">
            Ejecutor: {agentName} por ACP. Lo que aparece como «agente
            (reportado)» es lo que el agente dice, no algo comprobado por la
            app. Un intento completado no acepta la historia.
          </p>
        )}
        {session && <AgentSessionFacts session={session} />}

        {run.state === "queued" && run.startRequestedAt !== null && (
          <p className="run-detail__info" role="status">
            En cola · esperando turno: arrancará cuando haya hueco según el
            límite de ejecuciones a la vez (Sistema → Ejecuciones).
          </p>
        )}
        {request?.voidedAt != null && run.state === "queued" && (
          <p className="run-detail__warning">
            La petición de inicio se anuló al reiniciar la ejecución (al abrir
            la app, tras restaurar una copia o al reintentar el almacenamiento;{" "}
            <time dateTime={request.voidedAt}>
              {formatDateTime(request.voidedAt)}
            </time>
            ): nada arranca solo. Pulsa «Iniciar» de nuevo si quieres
            ejecutarlo.
          </p>
        )}
        {isCancelPending(run) && run.cancelRequestedAt !== null && (
          <p className="run-detail__warning" role="status">
            Cancelación solicitada el {formatDateTime(run.cancelRequestedAt)}
            {detail.cancelReason !== null && ` (${detail.cancelReason})`}: se
            está deteniendo el proceso y sus procesos hijos…
          </p>
        )}
        {run.state === "completed" && (
          <p className="run-detail__info">
            {acp
              ? "El agente reportó el fin de su turno."
              : "El proceso terminó bien."}{" "}
            No cambia la historia ni su columna: aceptarla sigue siendo una
            decisión humana.
          </p>
        )}
        {canStartRun(run) && (
          <StartRunForm
            projectId={projectId}
            runId={run.id}
            runNumber={run.number}
            repositories={snapshot.repositories}
            focusRef={startFocusRef}
            onStarted={onStarted}
            onStale={onChanged}
          />
        )}
      </section>

      <WorktreeSection
        projectId={projectId}
        runId={run.id}
        worktree={detail.worktree}
        ended={!isActiveRun(run.state)}
        onChanged={onChanged}
      />

      {detail.outputRemovedAt !== null ? (
        <section className="run-output" aria-labelledby={`${id}-output-title`}>
          <h4 id={`${id}-output-title`} className="relations__title">
            Salida del proceso
          </h4>
          <p className="panel__muted">
            La salida de este intento se eliminó al limpiar (
            <time dateTime={detail.outputRemovedAt}>
              {formatDateTime(detail.outputRemovedAt)}
            </time>
            ); sus eventos se conservan.
          </p>
        </section>
      ) : (
        run.state !== "queued" && (
          <RunOutputPanel
            projectId={projectId}
            runId={run.id}
            live={live}
            refreshSignal={refreshSignal}
          />
        )
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
          Es una copia congelada: editar la historia después no la cambia.
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
        <p className="field__hint">
          Instrucciones congeladas en la instantánea. Con Claude Code (ACP) o
          Codex (ACP) se envían las mismas, salvo la sección de repositorios,
          que se sustituye por la carpeta del worktree (sin rutas de tu disco);
          la huella del texto enviado aparece en la sesión del agente.
        </p>
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
              <span>{describeRunEvent(e.event, detail.approvals)}</span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

const count = new Intl.NumberFormat("es");

/** Último evento `acp_session` del intento (la sesión ACP negociada), si lo hay. */
function lastAgentSession(detail: RunDetail): AcpSessionInfo | null {
  for (let i = detail.events.length - 1; i >= 0; i--) {
    const event = detail.events[i]?.event;
    if (event?.kind === "acp_session") return event;
  }
  return null;
}

/** Lo que declaró el agente al negociar la sesión. Texto plano: es dato no confiable. */
function AgentSessionFacts({ session }: { session: AcpSessionInfo }) {
  return (
    <section aria-label="Sesión del agente (reportado por el agente)">
      <h5 className="run-detail__subtitle">
        Sesión del agente (reportado por el agente)
      </h5>
      <dl className="facts facts--compact">
        <dt>Agente</dt>
        <dd>{describeAgentIdentity(session)}</dd>
        <dt>Modelo (reportado por el agente)</dt>
        <dd>{describeAgentModel(session.model)}</dd>
        <dt>Protocolo</dt>
        <dd>ACP v{session.protocolVersion}</dd>
        <dt>Modo fijado por la app</dt>
        <dd>{describeAgentMode(session.mode)}</dd>
        <dt>Capacidades</dt>
        <dd>{describeCapabilities(session.capabilities)}</dd>
        {session.promptSha256 !== null && session.promptChars !== null && (
          <>
            <dt>Instrucciones enviadas por la app</dt>
            <dd>
              {count.format(session.promptChars)} caracteres · SHA-256{" "}
              <span className="input--mono" title={session.promptSha256}>
                {session.promptSha256.slice(0, 12)}…
              </span>
              <span className="visually-hidden">
                {" "}
                (huella completa: {session.promptSha256})
              </span>
            </dd>
          </>
        )}
      </dl>
    </section>
  );
}
