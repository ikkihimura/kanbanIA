import { useCallback, useRef, useState } from "react";
import type { Priority } from "../../contracts/generated/Priority";
import {
  createStory,
  listColumns,
  listEpics,
  listStories,
  newRequestId,
} from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { FieldError } from "../../shared/ui/FieldError";
import { PRIORITIES, PRIORITY_LABEL } from "./priorities";

interface Props {
  projectId: string;
  onOpenStory: (storyId: string) => void;
}

export function StoriesSection({ projectId, onOpenStory }: Props) {
  const [showArchived, setShowArchived] = useState(false);
  const loadStories = useCallback(
    () => listStories(projectId, true),
    [projectId],
  );
  const loadColumns = useCallback(() => listColumns(projectId), [projectId]);
  const loadEpics = useCallback(() => listEpics(projectId), [projectId]);
  const epics = useIpcResource(loadEpics);
  const stories = useIpcResource(loadStories);
  const columns = useIpcResource(loadColumns);
  const [title, setTitle] = useState("");
  const [priority, setPriority] = useState<Priority>("medium");
  const requestId = useRef(newRequestId());
  const create = useMutation(createStory);

  async function handleCreate() {
    const result = await create.mutate({
      requestId: requestId.current,
      projectId,
      title,
      priority,
    });
    if (result.ok) {
      requestId.current = newRequestId();
      onOpenStory(result.value.id);
    }
  }

  const columnName = (id: string) =>
    columns.state.status === "ready"
      ? (columns.state.data.find((c) => c.id === id)?.name ?? "")
      : "";
  const all = stories.state.status === "ready" ? stories.state.data : [];
  const epicNumber = (id: string | null) =>
    id && epics.state.status === "ready"
      ? epics.state.data.find((e) => e.id === id)?.number
      : undefined;
  const storyNumber = (id: string | null) =>
    id ? all.find((s) => s.id === id)?.number : undefined;
  const visible = showArchived ? all : all.filter((s) => !s.archivedAt);
  const archivedCount = all.length - all.filter((s) => !s.archivedAt).length;

  return (
    <section className="panel" aria-labelledby="stories-title">
      <h2 id="stories-title" className="panel__title">
        Historias
      </h2>

      <form
        className="form-row"
        aria-label="Nueva historia"
        onSubmit={(event) => {
          event.preventDefault();
          void handleCreate();
        }}
      >
        <div className="field field--grow">
          <label htmlFor="story-title">Título de la nueva historia</label>
          <input
            id="story-title"
            value={title}
            maxLength={200}
            autoComplete="off"
            aria-invalid={create.error ? true : undefined}
            aria-describedby={create.error ? "story-title-error" : undefined}
            onChange={(event) => {
              setTitle(event.target.value);
              requestId.current = newRequestId();
              create.reset();
            }}
          />
          <FieldError id="story-title-error" error={create.error} />
        </div>
        <div className="field field--narrow">
          <label htmlFor="story-priority">Prioridad</label>
          <select
            id="story-priority"
            value={priority}
            onChange={(event) => {
              setPriority(event.target.value as Priority);
              requestId.current = newRequestId();
            }}
          >
            {PRIORITIES.map((p) => (
              <option key={p} value={p}>
                {PRIORITY_LABEL[p]}
              </option>
            ))}
          </select>
        </div>
        <button type="submit" className="button" disabled={create.pending}>
          Crear historia
        </button>
      </form>

      <label className="checkbox">
        <input
          type="checkbox"
          checked={showArchived}
          onChange={(event) => {
            setShowArchived(event.target.checked);
          }}
        />
        Mostrar archivadas ({archivedCount})
      </label>

      <div role="status" aria-live="polite">
        {stories.state.status === "loading" && (
          <p className="panel__muted">Cargando historias…</p>
        )}
        {stories.state.status === "error" && (
          <p className="panel__error">{stories.state.message}</p>
        )}
        {stories.state.status === "ready" && visible.length === 0 && (
          <p className="panel__muted">
            {all.length === 0
              ? "Aún no hay historias. Escribe un título arriba para crear la primera."
              : "No hay historias activas. Activa «Mostrar archivadas» para ver el resto."}
          </p>
        )}
      </div>

      {visible.length > 0 && (
        <ul className="item-list">
          {visible.map((story) => (
            <li key={story.id}>
              <button
                type="button"
                className="item-list__button"
                onClick={() => {
                  onOpenStory(story.id);
                }}
              >
                <span className="item-list__name">
                  <span className="story-number">#{story.number}</span>{" "}
                  {story.title}
                </span>
                <span className="item-list__meta">
                  Prioridad {PRIORITY_LABEL[story.priority].toLowerCase()}
                  {" · "}
                  {columnName(story.columnId)}
                  {" · "}
                  {story.criteriaCount === 1
                    ? "1 criterio"
                    : `${String(story.criteriaCount)} criterios`}
                  {epicNumber(story.epicId) !== undefined &&
                    ` · E-${String(epicNumber(story.epicId))}`}
                  {storyNumber(story.parentId) !== undefined &&
                    ` · subtarea de #${String(storyNumber(story.parentId))}`}
                  {story.archivedAt ? " · archivada" : ""}
                </span>
                {story.pendingPrerequisites > 0 && (
                  <span className="badge badge--warn">
                    Bloqueada por {story.pendingPrerequisites}{" "}
                    {story.pendingPrerequisites === 1
                      ? "prerequisito sin aceptar"
                      : "prerequisitos sin aceptar"}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
