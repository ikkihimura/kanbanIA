import { useEffect, useRef, useState } from "react";
import type { Project } from "../../contracts/generated/Project";
import { renameProject } from "../../shared/ipc/commands";
import { useMutation } from "../../shared/ipc/useMutation";
import { FieldError } from "../../shared/ui/FieldError";
import { ColumnsSection } from "../columns/ColumnsSection";
import { EpicsSection } from "../epics/EpicsSection";
import { RepositoriesSection } from "../repositories/RepositoriesSection";
import { StoriesSection } from "../stories/StoriesSection";

interface Props {
  project: Project;
  onBack: () => void;
  onChanged: (project: Project) => void;
  onOpenStory: (storyId: string) => void;
  /** Aviso tras volver de otra pantalla (p. ej. historia guardada). */
  notice?: string | undefined;
}

export function ProjectView({
  project,
  onBack,
  onChanged,
  onOpenStory,
  notice,
}: Props) {
  const titleRef = useRef<HTMLHeadingElement>(null);
  const renameButtonRef = useRef<HTMLButtonElement>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(project.name);
  const rename = useMutation(renameProject);

  useEffect(() => {
    titleRef.current?.focus();
  }, [project.id]);

  function stopEditing() {
    setEditing(false);
    rename.reset();
    // Devolver el foco al botón que abrió el formulario.
    requestAnimationFrame(() => renameButtonRef.current?.focus());
  }

  async function handleRename() {
    const result = await rename.mutate({
      id: project.id,
      expectedRevision: project.revision,
      name: draft,
    });
    if (result.ok) {
      onChanged(result.value);
      stopEditing();
    }
  }

  // Ante un conflicto, adoptar el estado vigente que devuelve el núcleo.
  const conflictCurrent =
    rename.error?.code === "conflict" ? rename.error.current : undefined;
  useEffect(() => {
    if (conflictCurrent && typeof conflictCurrent === "object") {
      onChanged(conflictCurrent as unknown as Project);
    }
  }, [conflictCurrent, onChanged]);

  return (
    <main className="view">
      <nav aria-label="Navegación">
        <button type="button" className="link-button" onClick={onBack}>
          ← Proyectos
        </button>
      </nav>

      {notice && (
        <p className="notice" role="status">
          ✓ {notice}
        </p>
      )}

      <header className="view__header">
        <h1 className="view__title" ref={titleRef} tabIndex={-1}>
          {project.name}
        </h1>
        {!editing && (
          <button
            type="button"
            className="button button--secondary"
            ref={renameButtonRef}
            onClick={() => {
              setDraft(project.name);
              setEditing(true);
            }}
          >
            Renombrar
          </button>
        )}
      </header>

      {editing && (
        <form
          className="form-row panel"
          aria-label="Renombrar proyecto"
          onSubmit={(event) => {
            event.preventDefault();
            void handleRename();
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") stopEditing();
          }}
        >
          <div className="field">
            <label htmlFor="rename-project">Nuevo nombre</label>
            <input
              id="rename-project"
              value={draft}
              maxLength={120}
              autoFocus
              autoComplete="off"
              aria-invalid={rename.error ? true : undefined}
              aria-describedby={
                rename.error ? "rename-project-error" : undefined
              }
              onChange={(event) => {
                setDraft(event.target.value);
                rename.reset();
              }}
            />
            <FieldError id="rename-project-error" error={rename.error} />
          </div>
          <button type="submit" className="button" disabled={rename.pending}>
            Guardar
          </button>
          <button
            type="button"
            className="button button--secondary"
            onClick={stopEditing}
          >
            Cancelar
          </button>
        </form>
      )}

      <StoriesSection
        key={`stories-${project.id}`}
        projectId={project.id}
        onOpenStory={onOpenStory}
      />
      <EpicsSection key={`epics-${project.id}`} projectId={project.id} />
      <ColumnsSection key={`columns-${project.id}`} projectId={project.id} />
      <RepositoriesSection key={project.id} projectId={project.id} />
    </main>
  );
}
