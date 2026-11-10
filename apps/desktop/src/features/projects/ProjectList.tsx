import { useEffect, useRef, useState } from "react";
import {
  createProject,
  listProjects,
  newRequestId,
} from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { FieldError } from "../../shared/ui/FieldError";
import { formatDateTime } from "../../shared/ui/format";

interface Props {
  onOpen: (projectId: string) => void;
}

export function ProjectList({ onOpen }: Props) {
  const { state } = useIpcResource(listProjects);
  const titleRef = useRef<HTMLHeadingElement>(null);
  const [name, setName] = useState("");
  // Mismo request_id mientras se reintenta el mismo envío; uno nuevo al cambiar el nombre.
  const requestId = useRef(newRequestId());
  const create = useMutation(createProject);

  useEffect(() => {
    titleRef.current?.focus();
  }, []);

  async function handleCreate() {
    const result = await create.mutate({
      requestId: requestId.current,
      name,
    });
    if (result.ok) {
      requestId.current = newRequestId();
      onOpen(result.value.id);
    }
  }

  return (
    <main className="view">
      <h1 className="view__title" ref={titleRef} tabIndex={-1}>
        Proyectos
      </h1>

      <section className="panel" aria-labelledby="create-project-title">
        <h2 id="create-project-title" className="panel__title">
          Nuevo proyecto
        </h2>
        <form
          className="form-row"
          onSubmit={(event) => {
            event.preventDefault();
            void handleCreate();
          }}
        >
          <div className="field">
            <label htmlFor="project-name">Nombre del proyecto</label>
            <input
              id="project-name"
              value={name}
              maxLength={120}
              autoComplete="off"
              aria-invalid={create.error ? true : undefined}
              aria-describedby={create.error ? "project-name-error" : undefined}
              onChange={(event) => {
                setName(event.target.value);
                requestId.current = newRequestId();
                create.reset();
              }}
            />
            <FieldError id="project-name-error" error={create.error} />
          </div>
          <button type="submit" className="button" disabled={create.pending}>
            {create.pending ? "Creando…" : "Crear proyecto"}
          </button>
        </form>
      </section>

      <section className="panel" aria-labelledby="project-list-title">
        <h2 id="project-list-title" className="panel__title">
          Tus proyectos
        </h2>
        <div role="status" aria-live="polite">
          {state.status === "loading" && (
            <p className="panel__muted">Cargando proyectos…</p>
          )}
          {state.status === "error" && (
            <p className="panel__error">{state.message}</p>
          )}
          {state.status === "ready" && state.data.length === 0 && (
            <p className="panel__muted">
              Aún no hay proyectos. Crea el primero arriba.
            </p>
          )}
        </div>

        {state.status === "ready" && state.data.length > 0 && (
          <ul className="item-list">
            {state.data.map((project) => (
              <li key={project.id}>
                <button
                  type="button"
                  className="item-list__button"
                  onClick={() => {
                    onOpen(project.id);
                  }}
                >
                  <span className="item-list__name">{project.name}</span>
                  <span className="item-list__meta">
                    {project.repositoryCount === 1
                      ? "1 repositorio"
                      : `${String(project.repositoryCount)} repositorios`}
                    {" · "}
                    {project.lastOpenedAt
                      ? `abierto ${formatDateTime(project.lastOpenedAt)}`
                      : "nunca abierto"}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </main>
  );
}
