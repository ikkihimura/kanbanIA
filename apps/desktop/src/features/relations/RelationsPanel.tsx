import { useCallback, useId, useRef, useState } from "react";
import type { RelationState } from "../../contracts/generated/RelationState";
import type { StoryRef } from "../../contracts/generated/StoryRef";
import type { StoryRelations } from "../../contracts/generated/StoryRelations";
import {
  addStoryDependency,
  getStoryRelations,
  listAgents,
  listEpics,
  listStories,
  removeStoryDependency,
  setStoryAgent,
  setStoryEpic,
  setStoryParent,
} from "../../shared/ipc/commands";
import { useRefreshSignal } from "../../shared/ipc/useAgentChanges";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { PixelAvatar } from "../team/PixelAvatar";

const STATE_LABEL: Record<RelationState, string> = {
  accepted: "Aceptada",
  pending: "Sin aceptar",
  archived: "Archivada sin aceptar",
};

interface Props {
  projectId: string;
  storyId: string;
  /** Archivada: solo lectura (salvo quitar dependencias, que desbloquea a otras). */
  readOnly: boolean;
  onOpenStory: (storyId: string) => void;
  /** Cambió el agente asignado: el núcleo cierra el reclamo vigente de otro agente (HU-014). */
  onAgentChanged?: () => void;
  /** Cambia cuando un agente crea una subtarea o reclama la historia: vuelve a consultar. */
  refreshSignal?: number;
}

/**
 * Épica, subtareas y dependencias. Cada cambio se guarda al momento y no altera la revisión
 * de la historia: no interfiere con el borrador del editor.
 */
export function RelationsPanel({
  projectId,
  storyId,
  readOnly,
  onOpenStory,
  onAgentChanged,
  refreshSignal,
}: Props) {
  const id = useId();
  const loadRelations = useCallback(
    () => getStoryRelations(projectId, storyId),
    [projectId, storyId],
  );
  const loadEpics = useCallback(() => listEpics(projectId), [projectId]);
  const loadStories = useCallback(
    () => listStories(projectId, false),
    [projectId],
  );
  const relations = useIpcResource(loadRelations);
  const epics = useIpcResource(loadEpics);
  const agents = useIpcResource(listAgents);
  const stories = useIpcResource(loadStories);
  const [prerequisite, setPrerequisite] = useState("");
  const [announcement, setAnnouncement] = useState("");
  const titleRef = useRef<HTMLHeadingElement>(null);
  const { run: runRelations } = relations;
  const { run: runStories } = stories;
  const refresh = useCallback(() => {
    runRelations(loadRelations, { keepData: true });
    // La lista de historias alimenta los selectores (p. ej. una subtarea nueva).
    runStories(loadStories, { keepData: true });
  }, [runRelations, runStories, loadRelations, loadStories]);
  useRefreshSignal(refreshSignal, refresh);
  const addSelectRef = useRef<HTMLSelectElement>(null);

  // Todas las mutaciones devuelven las relaciones actualizadas.
  const mutate = useMutation(
    async (change: () => Promise<StoryRelations>): Promise<StoryRelations> =>
      change(),
  );

  // Devuelve si se guardó. Mientras hay un cambio en curso se ignoran otros (los controles
  // no se deshabilitan para no perder el foco del teclado).
  async function apply(
    change: () => Promise<StoryRelations>,
    message: string,
  ): Promise<boolean> {
    if (mutate.pending) return false;
    setAnnouncement("");
    const result = await mutate.mutate(change);
    if (result.ok) {
      relations.run(() => Promise.resolve(result.value), { keepData: true });
      setAnnouncement(message);
    }
    return result.ok;
  }

  // Tras añadir o quitar, el foco va al selector de prerequisitos (o al título si no existe).
  function focusAfterListChange() {
    requestAnimationFrame(() => {
      (addSelectRef.current ?? titleRef.current)?.focus();
    });
  }

  const data = relations.state.status === "ready" ? relations.state.data : null;
  const others =
    stories.state.status === "ready"
      ? stories.state.data.filter((s) => s.id !== storyId)
      : [];
  const linkedIds = new Set(data?.prerequisites.map((p) => p.id) ?? []);
  const candidates = others.filter((s) => !linkedIds.has(s.id));
  const label = (s: { number: number; title: string }) =>
    `#${String(s.number)} ${s.title}`;

  if (relations.state.status === "loading") {
    return <p className="panel__muted">Cargando relaciones…</p>;
  }
  if (relations.state.status === "error") {
    return <p className="panel__error">{relations.state.message}</p>;
  }
  if (!data) return null;

  return (
    <section className="panel relations" aria-labelledby={`${id}-title`}>
      <h2
        id={`${id}-title`}
        className="panel__title"
        ref={titleRef}
        tabIndex={-1}
      >
        Relaciones
      </h2>
      <p className="panel__hint">
        Los cambios de relaciones se guardan al momento.
      </p>

      {data.pendingPrerequisites > 0 && (
        <p className="badge badge--warn">
          Bloqueada: {data.pendingPrerequisites}{" "}
          {data.pendingPrerequisites === 1
            ? "prerequisito sin aceptar"
            : "prerequisitos sin aceptar"}
        </p>
      )}

      <p role="status" aria-live="polite" className="relations__saved">
        {announcement && `✓ ${announcement}`}
      </p>
      {mutate.error && (
        <p className="field__error" role="alert">
          {mutate.error.message}
        </p>
      )}

      <div className="form-row">
        <div className="field">
          <label htmlFor={`${id}-agent`}>Agente asignado</label>
          <div className="field__inline">
            {data.agent && (
              <PixelAvatar
                spec={data.agent.avatar}
                name={data.agent.name}
                size={32}
                decorative
              />
            )}
            <select
              id={`${id}-agent`}
              value={data.agent?.id ?? ""}
              disabled={readOnly}
              aria-busy={mutate.pending}
              aria-describedby={`${id}-agent-hint`}
              onChange={(event) => {
                const agentId = event.target.value || null;
                void apply(
                  () => setStoryAgent({ projectId, storyId, agentId }),
                  agentId ? "Agente asignado." : "Sin agente asignado.",
                ).then((saved) => {
                  if (saved) onAgentChanged?.();
                });
              }}
            >
              <option value="">Sin agente</option>
              {/* El asignado siempre aparece, aunque la lista no haya cargado. */}
              {data.agent &&
                !(
                  agents.state.status === "ready" &&
                  agents.state.data.some((a) => a.id === data.agent?.id)
                ) && (
                  <option value={data.agent.id}>
                    A-{data.agent.number} {data.agent.name}
                  </option>
                )}
              {agents.state.status === "ready" &&
                agents.state.data.map((a) => (
                  <option key={a.id} value={a.id}>
                    A-{a.number} {a.name}
                    {a.role ? ` (${a.role})` : ""}
                  </option>
                ))}
            </select>
          </div>
          <span id={`${id}-agent-hint`} className="field__hint">
            Asignar no inicia ninguna ejecución ni actividad.
          </span>
        </div>

        <div className="field">
          <label htmlFor={`${id}-epic`}>Épica</label>
          <select
            id={`${id}-epic`}
            value={data.epic?.id ?? ""}
            disabled={readOnly}
            aria-busy={mutate.pending}
            onChange={(event) => {
              const epicId = event.target.value || null;
              void apply(
                () => setStoryEpic({ projectId, storyId, epicId }),
                epicId ? "Épica guardada." : "Épica quitada.",
              );
            }}
          >
            <option value="">Sin épica</option>
            {epics.state.status === "ready" &&
              epics.state.data.map((e) => (
                <option key={e.id} value={e.id}>
                  E-{e.number} {e.title}
                </option>
              ))}
          </select>
        </div>

        <div className="field field--grow">
          <label htmlFor={`${id}-parent`}>Subtarea de</label>
          <select
            id={`${id}-parent`}
            value={data.parent?.id ?? ""}
            disabled={readOnly}
            aria-busy={mutate.pending}
            onChange={(event) => {
              const parentId = event.target.value || null;
              void apply(
                () => setStoryParent({ projectId, storyId, parentId }),
                parentId ? "Historia padre guardada." : "Ya no es subtarea.",
              );
            }}
          >
            <option value="">Ninguna (historia independiente)</option>
            {data.parent && !others.some((s) => s.id === data.parent?.id) && (
              <option value={data.parent.id}>{label(data.parent)}</option>
            )}
            {others.map((s) => (
              <option key={s.id} value={s.id}>
                {label(s)}
              </option>
            ))}
          </select>
        </div>
      </div>

      <RefList
        title="Subtareas"
        empty="Sin subtareas."
        items={data.children}
        onOpenStory={onOpenStory}
      />

      <RefList
        title="Depende de"
        empty="Sin prerequisitos."
        items={data.prerequisites}
        onOpenStory={onOpenStory}
        onRemove={(item) => {
          void apply(
            () =>
              removeStoryDependency({
                projectId,
                storyId,
                dependsOnId: item.id,
              }),
            `Ya no depende de ${label(item)}.`,
          ).then((ok) => {
            if (ok) focusAfterListChange();
          });
        }}
        removing={mutate.pending}
      />

      {!readOnly && (
        <form
          className="form-row"
          aria-label="Nuevo prerequisito"
          onSubmit={(event) => {
            event.preventDefault();
            if (!prerequisite) return;
            void apply(
              () =>
                addStoryDependency({
                  projectId,
                  storyId,
                  dependsOnId: prerequisite,
                }),
              "Prerequisito guardado.",
            ).then((ok) => {
              if (ok) {
                setPrerequisite("");
                focusAfterListChange();
              }
            });
          }}
        >
          <div className="field field--grow">
            <label htmlFor={`${id}-dependency`}>Añadir prerequisito</label>
            <select
              id={`${id}-dependency`}
              ref={addSelectRef}
              value={prerequisite}
              onChange={(event) => {
                setPrerequisite(event.target.value);
                mutate.reset();
              }}
            >
              <option value="">Elige una historia…</option>
              {candidates.map((s) => (
                <option key={s.id} value={s.id}>
                  {label(s)}
                </option>
              ))}
            </select>
          </div>
          <button
            type="submit"
            className="button button--secondary"
            aria-busy={mutate.pending}
            disabled={!prerequisite || mutate.pending}
          >
            Añadir
          </button>
        </form>
      )}

      <RefList
        title="Bloquea a"
        empty="Ninguna historia depende de esta."
        items={data.dependents}
        onOpenStory={onOpenStory}
      />
    </section>
  );
}

function RefList({
  title,
  empty,
  items,
  onOpenStory,
  onRemove,
  removing = false,
}: {
  title: string;
  empty: string;
  items: StoryRef[];
  onOpenStory: (storyId: string) => void;
  onRemove?: (item: StoryRef) => void;
  removing?: boolean;
}) {
  return (
    <div className="relations__group">
      <h3 className="relations__title">{title}</h3>
      {items.length === 0 ? (
        <p className="panel__muted">{empty}</p>
      ) : (
        <ul className="relations__list">
          {items.map((item) => (
            <li key={item.id} className="relations__item">
              <button
                type="button"
                className="link-button"
                onClick={() => {
                  onOpenStory(item.id);
                }}
              >
                #{item.number} {item.title}
              </button>
              <span
                className={`badge ${item.state === "accepted" ? "badge--ok" : "badge--warn"}`}
              >
                {STATE_LABEL[item.state]}
              </span>
              {onRemove && (
                <button
                  type="button"
                  className="button button--secondary"
                  aria-label={`Quitar dependencia de #${String(item.number)} ${item.title}`}
                  disabled={removing}
                  onClick={() => {
                    onRemove(item);
                  }}
                >
                  Quitar
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
