import { useCallback, useRef, useState } from "react";
import type { Epic } from "../../contracts/generated/Epic";
import {
  createEpic,
  listEpics,
  newRequestId,
  updateEpic,
} from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { FieldError } from "../../shared/ui/FieldError";
import { EpicProgressBar } from "./EpicProgressBar";

interface Props {
  projectId: string;
}

export function EpicsSection({ projectId }: Props) {
  const load = useCallback(() => listEpics(projectId), [projectId]);
  const { state, run } = useIpcResource(load);
  const [title, setTitle] = useState("");
  const requestId = useRef(newRequestId());
  const create = useMutation(createEpic);
  const [announcement, setAnnouncement] = useState("");

  const reload = useCallback(() => {
    run(load, { keepData: true });
  }, [run, load]);

  async function handleCreate() {
    const result = await create.mutate({
      requestId: requestId.current,
      projectId,
      title,
    });
    if (result.ok) {
      requestId.current = newRequestId();
      setTitle("");
      setAnnouncement(`Épica E-${String(result.value.number)} creada.`);
      reload();
    }
  }

  return (
    <section className="panel" aria-labelledby="epics-title">
      <h2 id="epics-title" className="panel__title">
        Épicas
      </h2>
      <p className="panel__hint">
        El avance cuenta historias aceptadas sobre las no archivadas de cada
        épica.
      </p>

      <form
        className="form-row"
        aria-label="Nueva épica"
        onSubmit={(event) => {
          event.preventDefault();
          void handleCreate();
        }}
      >
        <div className="field field--grow">
          <label htmlFor="epic-title">Título de la nueva épica</label>
          <input
            id="epic-title"
            value={title}
            maxLength={120}
            autoComplete="off"
            aria-invalid={create.error ? true : undefined}
            aria-describedby={create.error ? "epic-title-error" : undefined}
            onChange={(event) => {
              setTitle(event.target.value);
              requestId.current = newRequestId();
              create.reset();
            }}
          />
          <FieldError id="epic-title-error" error={create.error} />
        </div>
        <button
          type="submit"
          className="button"
          aria-busy={create.pending}
          disabled={create.pending}
        >
          Crear épica
        </button>
      </form>

      <p role="status" aria-live="polite" className="relations__saved">
        {announcement && `✓ ${announcement}`}
      </p>
      <div role="status" aria-live="polite">
        {state.status === "loading" && (
          <p className="panel__muted">Cargando épicas…</p>
        )}
        {state.status === "error" && (
          <p className="panel__error">{state.message}</p>
        )}
        {state.status === "ready" && state.data.length === 0 && (
          <p className="panel__muted">Aún no hay épicas.</p>
        )}
      </div>

      {state.status === "ready" && state.data.length > 0 && (
        <ul className="item-list">
          {state.data.map((epic) => (
            <EpicItem
              key={epic.id}
              epic={epic}
              onChanged={(message) => {
                setAnnouncement(message);
                reload();
              }}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function EpicItem({
  epic,
  onChanged,
}: {
  epic: Epic;
  onChanged: (message: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState({
    title: epic.title,
    description: epic.description,
  });
  const update = useMutation(updateEpic);
  const label = `E-${String(epic.number)} ${epic.title}`;

  async function handleSave() {
    const result = await update.mutate({
      projectId: epic.projectId,
      id: epic.id,
      expectedRevision: epic.revision,
      ...draft,
    });
    if (result.ok) {
      setEditing(false);
      onChanged(`Épica E-${String(epic.number)} actualizada.`);
    } else if (result.error.code === "conflict") {
      // No permitir sobrescribir a ciegas: se cierra el formulario y se recarga la versión vigente.
      setEditing(false);
      onChanged(
        "La épica cambió en otra edición: se cargó la versión vigente. Vuelve a editarla si hace falta.",
      );
    }
  }

  return (
    <li className="column-item">
      <div className="column-item__summary">
        <span className="item-list__name">
          <span className="story-number">E-{epic.number}</span> {epic.title}
        </span>
        {!editing && (
          <button
            type="button"
            className="button button--secondary"
            aria-label={`Editar ${label}`}
            onClick={() => {
              setDraft({ title: epic.title, description: epic.description });
              update.reset();
              setEditing(true);
            }}
          >
            Editar
          </button>
        )}
      </div>
      <EpicProgressBar progress={epic.progress} label={label} />
      {!editing && epic.description && (
        <p className="panel__muted">{epic.description}</p>
      )}
      {editing && (
        <form
          className="form-row"
          aria-label={`Editar ${label}`}
          onSubmit={(event) => {
            event.preventDefault();
            void handleSave();
          }}
          onKeyDown={(event) => {
            // Escape solo cancela si no hay cambios: no descartar texto sin querer.
            const unchanged =
              draft.title === epic.title &&
              draft.description === epic.description;
            if (event.key === "Escape" && unchanged) setEditing(false);
          }}
        >
          <div className="field field--grow">
            <label htmlFor={`epic-${epic.id}-title`}>Título</label>
            <input
              id={`epic-${epic.id}-title`}
              value={draft.title}
              maxLength={120}
              autoFocus
              onChange={(event) => {
                setDraft({ ...draft, title: event.target.value });
                update.reset();
              }}
            />
          </div>
          <div className="field form-row__full">
            <label htmlFor={`epic-${epic.id}-description`}>
              Objetivo (texto)
            </label>
            <textarea
              id={`epic-${epic.id}-description`}
              rows={3}
              maxLength={5000}
              value={draft.description}
              onChange={(event) => {
                setDraft({ ...draft, description: event.target.value });
                update.reset();
              }}
            />
          </div>
          {update.error && (
            <p className="field__error form-row__full" role="alert">
              {update.error.message}
            </p>
          )}
          <button
            type="submit"
            className="button"
            aria-busy={update.pending}
            disabled={update.pending}
          >
            Guardar
          </button>
          <button
            type="button"
            className="button button--secondary"
            onClick={() => {
              setEditing(false);
            }}
          >
            Cancelar
          </button>
        </form>
      )}
    </li>
  );
}
