import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import {
  cancelRun,
  getStoryRelations,
  listAgents,
  listStoryRuns,
  newRequestId,
} from "../../shared/ipc/commands";
import { useRefreshSignal } from "../../shared/ipc/useAgentChanges";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { formatDateTime } from "../../shared/ui/format";
import { PrepareRunForm } from "./PrepareRunForm";
import { RunDetailView } from "./RunDetailView";
import {
  RUN_STATE_LABEL,
  formatCost,
  formatTokens,
  isActiveRun,
  runStateBadgeClass,
} from "./runFormat";

/** Sin ejecutor (HU-018/019), un intento preparado se queda en cola: se dice, no se simula. */
export const NO_RUNNER_NOTICE =
  "Aún no hay ejecutor: el intento queda en cola hasta que llegue la ejecución de agentes (M3).";

interface Props {
  projectId: string;
  storyId: string;
  /** La historia no admite intentos nuevos (archivada o aceptada). */
  closed: "archived" | "accepted" | null;
  /**
   * Cambia cuando la historia cambió (aceptar o archivar cancelan su intento activo en el
   * núcleo; asignar cambia el agente por defecto) o llegó un aviso de un agente.
   */
  refreshSignal?: number;
  /** Hay una nota o un agente elegidos sin enviar. */
  onDirtyChange: (dirty: boolean) => void;
}

/** Intentos de ejecución de la historia (HU-017): lista, preparar, detalle y cancelar. */
export function RunsPanel({
  projectId,
  storyId,
  closed,
  refreshSignal,
  onDirtyChange,
}: Props) {
  const id = useId();
  const loadRuns = useCallback(
    () => listStoryRuns(projectId, storyId),
    [projectId, storyId],
  );
  const runs = useIpcResource(loadRuns);
  const loadRelations = useCallback(
    () => getStoryRelations(projectId, storyId),
    [projectId, storyId],
  );
  const relations = useIpcResource(loadRelations);
  const agents = useIpcResource(listAgents);

  const [selected, setSelected] = useState<{
    runId: string;
    focus: boolean;
  } | null>(null);
  const [detailEpoch, setDetailEpoch] = useState(0);
  const [message, setMessage] = useState("");
  const [confirm, setConfirm] = useState<RunSummary | null>(null);
  // Se conserva al reintentar la misma cancelación; cambia al abrir otra confirmación.
  const cancelRequestId = useRef(newRequestId());
  const canceller = useMutation(cancelRun);

  const titleRef = useRef<HTMLHeadingElement>(null);
  const confirmCancelRef = useRef<HTMLButtonElement>(null);
  const detailButtons = useRef(new Map<string, HTMLButtonElement>());
  const cancelButtons = useRef(new Map<string, HTMLButtonElement>());

  useEffect(() => {
    if (confirm) confirmCancelRef.current?.focus();
  }, [confirm]);

  const runsRun = runs.run;
  const relationsRun = relations.run;
  const reloadRuns = useCallback(() => {
    runsRun(loadRuns, { keepData: true });
  }, [runsRun, loadRuns]);
  const reloadAll = useCallback(() => {
    reloadRuns();
    relationsRun(loadRelations, { keepData: true });
    setDetailEpoch((n) => n + 1);
  }, [reloadRuns, relationsRun, loadRelations]);
  useRefreshSignal(refreshSignal, reloadAll);

  const list = runs.state.status === "ready" ? runs.state.data : null;
  const activeRun = list?.find((r) => isActiveRun(r.state)) ?? null;

  function toggleDetail(runId: string) {
    setSelected((current) =>
      current?.runId === runId ? null : { runId, focus: true },
    );
  }

  function closeDetail() {
    const runId = selected?.runId;
    setSelected(null);
    const button = runId ? detailButtons.current.get(runId) : undefined;
    (button ?? titleRef.current)?.focus();
  }

  function openConfirm(run: RunSummary) {
    cancelRequestId.current = newRequestId();
    canceller.reset();
    setMessage("");
    setConfirm(run);
  }

  function closeConfirm() {
    const runId = confirm?.id;
    setConfirm(null);
    canceller.reset();
    const button = runId ? cancelButtons.current.get(runId) : undefined;
    (button ?? titleRef.current)?.focus();
  }

  async function handleCancel() {
    if (!confirm) return;
    const result = await canceller.mutate({
      requestId: cancelRequestId.current,
      projectId,
      runId: confirm.id,
    });
    if (result.ok) {
      setConfirm(null);
      // El botón «Cancelar intento» desaparece: el foco va al título de la sección.
      titleRef.current?.focus();
      setMessage(`Intento #${String(result.value.number)} cancelado.`);
      reloadRuns();
      setDetailEpoch((n) => n + 1);
    } else if (result.error.code === "invalid_run_transition") {
      // Ya terminó (otra vía lo cerró): la lista debe reflejar su estado real.
      reloadRuns();
      setDetailEpoch((n) => n + 1);
    }
  }

  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <h2
        id={`${id}-title`}
        className="panel__title"
        ref={titleRef}
        tabIndex={-1}
      >
        Intentos
      </h2>
      <p className="field__hint">
        Cada intento guarda una copia de la historia y de las instrucciones que
        recibiría el agente. Solo puede haber un intento activo a la vez.
      </p>
      <p className="run-notice">{NO_RUNNER_NOTICE}</p>

      <p role="status" className="relations__saved">
        {message && `✓ ${message}`}
      </p>

      {closed === null ? (
        <PrepareRunForm
          projectId={projectId}
          storyId={storyId}
          assigned={
            relations.state.status === "ready"
              ? relations.state.data.agent
              : undefined
          }
          agents={agents.state.status === "ready" ? agents.state.data : null}
          activeRun={activeRun}
          onShowRun={(runId) => {
            setSelected({ runId, focus: true });
          }}
          onRunActive={reloadRuns}
          onDirtyChange={onDirtyChange}
          onCreated={(run) => {
            setMessage(
              `Intento #${String(run.number)} preparado: queda en cola (aún no hay ejecutor).`,
            );
            reloadRuns();
            setSelected({ runId: run.id, focus: false });
            setDetailEpoch((n) => n + 1);
          }}
        />
      ) : (
        <p className="panel__muted">
          {closed === "archived"
            ? "La historia está archivada: restáurala para preparar un intento."
            : "La historia está aceptada: reábrela para preparar otro intento."}
        </p>
      )}
      {agents.state.status === "error" && (
        <p className="panel__error" role="alert">
          No se pudo cargar el equipo: {agents.state.message}. Puedes preparar
          el intento con el agente asignado.
        </p>
      )}

      {runs.state.status === "loading" && (
        <p className="panel__muted" role="status">
          Cargando intentos…
        </p>
      )}
      {runs.state.status === "error" && (
        <div>
          <p className="panel__error" role="alert">
            No se pudieron cargar los intentos: {runs.state.message}
          </p>
          <button
            type="button"
            className="button"
            onClick={() => {
              runs.run(loadRuns);
            }}
          >
            Reintentar
          </button>
        </div>
      )}
      {list?.length === 0 && (
        <p className="panel__muted">Esta historia no tiene intentos todavía.</p>
      )}
      {list && list.length > 0 && (
        <ol
          className="item-list run-list"
          aria-label="Intentos, del más reciente al más antiguo"
        >
          {list.map((run) => {
            const open = selected?.runId === run.id;
            return (
              <li key={run.id} className="repo">
                <div className="repo__summary">
                  <span>
                    <strong>Intento #{run.number}</strong>{" "}
                    <span className={runStateBadgeClass(run.state)}>
                      {RUN_STATE_LABEL[run.state]}
                    </span>
                  </span>
                  <span className="repo__actions">
                    <button
                      type="button"
                      className="button button--secondary"
                      aria-expanded={open}
                      aria-controls={open ? `${id}-detail` : undefined}
                      ref={(el) => {
                        if (el) detailButtons.current.set(run.id, el);
                        else detailButtons.current.delete(run.id);
                      }}
                      onClick={() => {
                        toggleDetail(run.id);
                      }}
                    >
                      {`${open ? "Ocultar detalle" : "Ver detalle"} `}
                      <span className="visually-hidden">
                        {`del intento #${String(run.number)}`}
                      </span>
                    </button>
                    {isActiveRun(run.state) && (
                      <button
                        type="button"
                        className="button button--secondary"
                        disabled={canceller.pending}
                        ref={(el) => {
                          if (el) cancelButtons.current.set(run.id, el);
                          else cancelButtons.current.delete(run.id);
                        }}
                        onClick={() => {
                          openConfirm(run);
                        }}
                      >
                        Cancelar intento…
                      </button>
                    )}
                  </span>
                </div>
                <dl className="facts facts--compact">
                  <dt>Agente</dt>
                  <dd>
                    A-{run.agentNumber} {run.agentName}
                  </dd>
                  <dt>Creado</dt>
                  <dd>
                    <time dateTime={run.createdAt}>
                      {formatDateTime(run.createdAt)}
                    </time>
                  </dd>
                  {run.endedAt !== null && (
                    <>
                      <dt>Terminado</dt>
                      <dd>
                        <time dateTime={run.endedAt}>
                          {formatDateTime(run.endedAt)}
                        </time>
                      </dd>
                    </>
                  )}
                  {run.endReason !== null && (
                    <>
                      <dt>Motivo</dt>
                      <dd>{run.endReason}</dd>
                    </>
                  )}
                  <dt>Tokens de entrada</dt>
                  <dd>{formatTokens(run.usage.inputTokens)}</dd>
                  <dt>Tokens de salida</dt>
                  <dd>{formatTokens(run.usage.outputTokens)}</dd>
                  <dt>Coste</dt>
                  <dd>{formatCost(run.usage.costUsdMicros)}</dd>
                </dl>
                {run.state === "queued" && (
                  <p className="field__hint">
                    En cola: nadie lo está ejecutando todavía.
                  </p>
                )}

                {confirm?.id === run.id && (
                  <div
                    className="repo__confirm"
                    role="alertdialog"
                    aria-labelledby={`${id}-confirm`}
                    onKeyDown={(event) => {
                      if (event.key === "Escape" && !canceller.pending)
                        closeConfirm();
                    }}
                  >
                    <p id={`${id}-confirm`}>
                      ¿Cancelar el intento #{run.number}? Quedará como
                      «Cancelado» y no se podrá reanudar; su instantánea y sus
                      eventos se conservan. Podrás preparar otro intento.
                    </p>
                    {canceller.error && (
                      <p className="field__error" role="alert">
                        {canceller.error.message}
                      </p>
                    )}
                    <div className="repo__actions">
                      <button
                        type="button"
                        className="button button--danger"
                        aria-busy={canceller.pending}
                        disabled={canceller.pending}
                        onClick={() => {
                          void handleCancel();
                        }}
                      >
                        Cancelar intento
                      </button>
                      <button
                        type="button"
                        className="button button--secondary"
                        ref={confirmCancelRef}
                        disabled={canceller.pending}
                        onClick={closeConfirm}
                      >
                        Mantener
                      </button>
                    </div>
                  </div>
                )}
              </li>
            );
          })}
        </ol>
      )}

      {selected && (
        <div id={`${id}-detail`}>
          <RunDetailView
            key={selected.runId}
            projectId={projectId}
            runId={selected.runId}
            refreshSignal={detailEpoch}
            focusOnLoad={selected.focus}
            onClose={closeDetail}
          />
        </div>
      )}
    </section>
  );
}
