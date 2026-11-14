import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { StoryDetail } from "../../contracts/generated/StoryDetail";
import {
  archiveStory,
  getStory,
  listColumns,
  restoreStory,
} from "../../shared/ipc/commands";
import { useAgentChanges } from "../../shared/ipc/useAgentChanges";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { formatDateTime } from "../../shared/ui/format";
import { MarkdownPreview } from "../../shared/ui/MarkdownPreview";
import { useUnsavedChangesGuard } from "../../shared/ui/useUnsavedChangesGuard";
import { AgentActivityPanel } from "../agent_activity/AgentActivityPanel";
import { ClaimPanel } from "../claims/ClaimPanel";
import { RelationsPanel } from "../relations/RelationsPanel";
import { ReviewPanel } from "../review/ReviewPanel";
import { PRIORITY_LABEL } from "./priorities";
import { StoryForm } from "./StoryForm";

interface Props {
  projectId: string;
  storyId: string;
  /** Volver al proyecto, opcionalmente con un aviso (p. ej. «Historia #3 guardada»). */
  onBack: (notice?: string) => void;
  onOpenStory: (storyId: string) => void;
}

export function StoryEditor({
  projectId,
  storyId,
  onBack,
  onOpenStory,
}: Props) {
  const load = useCallback(
    () => getStory(projectId, storyId),
    [projectId, storyId],
  );
  const { state, run } = useIpcResource(load);
  const titleRef = useRef<HTMLHeadingElement>(null);
  const [dirty, setDirty] = useState(false);
  // Navegación pendiente de confirmar porque hay cambios sin guardar.
  const [pendingLeave, setPendingLeave] = useState<(() => void) | null>(null);
  const [confirmArchive, setConfirmArchive] = useState(false);
  // Versión conocida más reciente (tras guardar), para encabezado y archivo.
  const [latest, setLatest] = useState<StoryDetail | null>(null);
  const archive = useMutation(archiveStory);
  // Reclamo y asignación se afectan entre sí (liberar quita la asignación; cambiarla cierra el
  // reclamo): cada contador vuelve a montar el panel contrario para que no muestre datos viejos.
  const [claimEpoch, setClaimEpoch] = useState(0);
  const [relationsEpoch, setRelationsEpoch] = useState(0);
  // Cerrar la ventana o salir con cambios sin guardar pide confirmación (ver exit_guard.rs).
  useUnsavedChangesGuard(dirty);

  const story = latest ?? (state.status === "ready" ? state.data : null);

  useEffect(() => {
    if (state.status === "ready") titleRef.current?.focus();
  }, [state.status]);

  // El formulario solo se reinicia con la versión recargada si no hay borrador: si el
  // usuario escribió mientras otra acción cambiaba la historia, conserva su texto (al
  // guardar verá el conflicto y podrá conservar su versión).
  const [formEpoch, setFormEpoch] = useState(0);
  const dirtyRef = useRef(dirty);
  useEffect(() => {
    dirtyRef.current = dirty;
  }, [dirty]);
  const reload = useCallback(() => {
    setLatest(null);
    if (!dirtyRef.current) setFormEpoch((n) => n + 1);
    run(load, { keepData: true });
  }, [run, load]);

  // Avisos de agentes (HU-015): reclamar, renovar, bloqueo, subtarea o entrega. Se recargan
  // la historia y los paneles de solo lectura; el borrador nunca se pisa. El formulario solo
  // se reconstruye si la revisión cambió y no hay cambios sin guardar (así no se pierde el
  // foco por un aviso que no tocó la historia). Con borrador, guardar dará el conflicto.
  const [agentEpoch, setAgentEpoch] = useState(0);
  const agentSync = useRef(false);
  const formRevision = useRef<number | null>(null);
  useAgentChanges(projectId, (change) => {
    if (!change.storyIds.includes(storyId)) return;
    setAgentEpoch((n) => n + 1);
    agentSync.current = true;
    setLatest(null);
    run(load, { keepData: true });
  });
  // Revisión con la que se montó el formulario (su `key` cambia al reconstruirlo).
  const formKey = story ? `${story.id}-${String(formEpoch)}` : null;
  const mountedFormKey = useRef<string | null>(null);
  useEffect(() => {
    if (story?.archivedAt !== null) {
      mountedFormKey.current = null;
      return;
    }
    if (mountedFormKey.current !== formKey) {
      mountedFormKey.current = formKey;
      formRevision.current = story.revision;
    }
  }, [story, formKey]);
  useEffect(() => {
    if (state.status !== "ready" || !agentSync.current) return;
    agentSync.current = false;
    if (!dirtyRef.current && state.data.revision !== formRevision.current)
      setFormEpoch((n) => n + 1);
  }, [state]);

  function leave(action: () => void) {
    if (dirty) setPendingLeave(() => action);
    else action();
  }

  async function handleArchive(current: StoryDetail) {
    const result = await archive.mutate({
      projectId,
      id: current.id,
      expectedRevision: current.revision,
    });
    setConfirmArchive(false);
    if (result.ok) reload();
  }

  return (
    <main className="view">
      <nav aria-label="Navegación">
        <button
          type="button"
          className="link-button"
          onClick={() => {
            leave(() => {
              onBack();
            });
          }}
        >
          ← Proyecto
        </button>
      </nav>

      {pendingLeave && (
        <div
          className="repo__confirm panel"
          role="alertdialog"
          aria-label="Cambios sin guardar"
        >
          <p>Tienes cambios sin guardar. ¿Salir y descartarlos?</p>
          <div className="repo__actions">
            <button
              type="button"
              className="button button--danger"
              onClick={pendingLeave}
            >
              Salir sin guardar
            </button>
            <button
              type="button"
              className="button button--secondary"
              autoFocus
              onClick={() => {
                setPendingLeave(null);
              }}
            >
              Seguir editando
            </button>
          </div>
        </div>
      )}

      {state.status === "loading" && (
        <p role="status" className="panel__muted">
          Cargando historia…
        </p>
      )}
      {state.status === "error" && (
        <div role="alert" className="panel panel__error">
          <p>No se pudo cargar la historia: {state.message}</p>
          <button type="button" className="button" onClick={reload}>
            Reintentar
          </button>
        </div>
      )}

      {story && (
        <>
          <header className="view__header">
            <h1 className="view__title" ref={titleRef} tabIndex={-1}>
              <span className="story-number">#{story.number}</span>{" "}
              {story.title}
            </h1>
            {!story.archivedAt && !confirmArchive && (
              <button
                type="button"
                className="button button--secondary"
                disabled={dirty}
                title={
                  dirty ? "Guarda o descarta los cambios antes" : undefined
                }
                onClick={() => {
                  setConfirmArchive(true);
                }}
              >
                Archivar
              </button>
            )}
          </header>

          {confirmArchive && (
            <div
              className="repo__confirm panel"
              role="group"
              aria-label="Confirmar archivo"
            >
              <p>
                La historia saldrá del tablero y dejará de contar para el límite
                WIP. Se conserva todo su contenido y podrás restaurarla.
              </p>
              <div className="repo__actions">
                <button
                  type="button"
                  className="button button--danger"
                  autoFocus
                  disabled={archive.pending}
                  onClick={() => {
                    void handleArchive(story);
                  }}
                >
                  Archivar
                </button>
                <button
                  type="button"
                  className="button button--secondary"
                  onClick={() => {
                    setConfirmArchive(false);
                  }}
                >
                  Cancelar
                </button>
              </div>
            </div>
          )}
          {archive.error && (
            <p className="field__error" role="alert">
              {archive.error.message}
            </p>
          )}

          <RelationsPanel
            key={`relations-${story.id}-${story.archivedAt ?? "active"}-${String(relationsEpoch)}`}
            projectId={projectId}
            storyId={story.id}
            readOnly={story.archivedAt !== null}
            refreshSignal={agentEpoch}
            onOpenStory={(id) => {
              leave(() => {
                onOpenStory(id);
              });
            }}
            onAgentChanged={() => {
              setClaimEpoch((n) => n + 1);
            }}
          />

          <ClaimPanel
            key={`claim-${story.id}-${String(claimEpoch)}`}
            projectId={projectId}
            storyId={story.id}
            refreshSignal={agentEpoch}
            onReleased={() => {
              setRelationsEpoch((n) => n + 1);
            }}
          />

          <AgentActivityPanel
            key={`activity-${story.id}`}
            projectId={projectId}
            storyId={story.id}
            refreshSignal={agentEpoch}
          />

          <ReviewPanel
            key={`review-${story.id}`}
            story={story}
            locked={dirty}
            onStoryChanged={reload}
            refreshSignal={agentEpoch}
          />

          {story.archivedAt ? (
            <ArchivedStory story={story} onRestored={reload} />
          ) : (
            <section className="panel">
              <StoryForm
                key={formKey}
                story={story}
                onSaved={setLatest}
                onDirtyChange={setDirty}
                onDone={(saved) => {
                  onBack(
                    saved
                      ? `Historia #${String(saved.number)} guardada.`
                      : undefined,
                  );
                }}
              />
            </section>
          )}
        </>
      )}
    </main>
  );
}

function ArchivedStory({
  story,
  onRestored,
}: {
  story: StoryDetail;
  onRestored: () => void;
}) {
  const selectId = useId();
  const loadColumns = useCallback(
    () => listColumns(story.projectId),
    [story.projectId],
  );
  const columns = useIpcResource(loadColumns);
  const [chosen, setChosen] = useState(story.columnId);
  const restore = useMutation(restoreStory);
  const options =
    columns.state.status === "ready"
      ? columns.state.data.filter((c) => c.category !== "done")
      : [];
  // Lo enviado debe ser lo que se ve: si la columna original no es válida, la primera opción.
  const columnId = options.some((c) => c.id === chosen)
    ? chosen
    : (options[0]?.id ?? "");

  async function handleRestore() {
    const result = await restore.mutate({
      projectId: story.projectId,
      id: story.id,
      expectedRevision: story.revision,
      columnId,
    });
    if (result.ok) onRestored();
  }

  return (
    <section className="panel" aria-label="Historia archivada">
      <p className="badge badge--warn">
        Archivada {story.archivedAt ? formatDateTime(story.archivedAt) : ""}
      </p>
      <p>
        Prioridad {PRIORITY_LABEL[story.priority].toLowerCase()}. Solo lectura
        mientras esté archivada.
      </p>
      <MarkdownPreview source={story.description} />
      {story.criteria.length > 0 && (
        <ol className="criteria">
          {story.criteria.map((c) => (
            <li key={c.id}>
              CA-{c.number}: {c.text}
            </li>
          ))}
        </ol>
      )}
      <form
        className="form-row form-row--spaced"
        aria-label="Restaurar historia"
        onSubmit={(event) => {
          event.preventDefault();
          void handleRestore();
        }}
      >
        <div className="field">
          <label htmlFor={selectId}>Restaurar en la columna</label>
          <select
            id={selectId}
            value={columnId}
            onChange={(event) => {
              setChosen(event.target.value);
              restore.reset();
            }}
          >
            {options.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
        </div>
        <button
          type="submit"
          className="button"
          aria-busy={restore.pending}
          disabled={restore.pending || columnId === ""}
        >
          Restaurar
        </button>
      </form>
      {restore.error && (
        <p className="field__error" role="alert">
          {restore.error.message}
        </p>
      )}
    </section>
  );
}
