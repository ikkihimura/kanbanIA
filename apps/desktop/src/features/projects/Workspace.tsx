import { useCallback, useEffect, useRef, useState } from "react";
import type { Project } from "../../contracts/generated/Project";
import {
  describeError,
  getLastOpenedProject,
  openProject,
} from "../../shared/ipc/commands";
import { ProjectList } from "./ProjectList";
import { StoryEditor } from "../stories/StoryEditor";
import { ProjectView } from "./ProjectView";

type View =
  | { kind: "starting" }
  | { kind: "list"; notice?: string }
  | { kind: "project"; project: Project }
  | { kind: "story"; project: Project; storyId: string };

/** Navegación entre la lista de proyectos y un proyecto. Al iniciar reabre el último abierto. */
export function Workspace() {
  const [view, setView] = useState<View>({ kind: "starting" });
  // Solo cuenta la última apertura pedida: clics rápidos no se aplican fuera de orden.
  const latestOpen = useRef(0);

  useEffect(() => {
    let cancelled = false;
    getLastOpenedProject()
      .then((last) => (last ? openProject(last.id) : null))
      .then(
        (project) => {
          if (cancelled) return;
          setView(project ? { kind: "project", project } : { kind: "list" });
        },
        () => {
          // Si no se puede reabrir, la lista sigue siendo un punto de partida válido.
          if (!cancelled) setView({ kind: "list" });
        },
      );
    return () => {
      cancelled = true;
    };
  }, []);

  const open = useCallback((projectId: string) => {
    const ticket = ++latestOpen.current;
    openProject(projectId).then(
      (project) => {
        if (ticket === latestOpen.current)
          setView({ kind: "project", project });
      },
      (error: unknown) => {
        if (ticket !== latestOpen.current) return;
        setView({
          kind: "list",
          notice: `No se pudo abrir el proyecto: ${describeError(error)}`,
        });
      },
    );
  }, []);

  const backToList = useCallback(() => {
    setView({ kind: "list" });
  }, []);

  const replaceProject = useCallback((project: Project) => {
    setView({ kind: "project", project });
  }, []);

  switch (view.kind) {
    case "starting":
      return (
        <p className="view" role="status">
          Abriendo…
        </p>
      );
    case "list":
      return (
        <>
          {view.notice && (
            <p className="view panel__error" role="alert">
              {view.notice}
            </p>
          )}
          <ProjectList onOpen={open} />
        </>
      );
    case "project":
      return (
        <ProjectView
          project={view.project}
          onBack={backToList}
          onChanged={replaceProject}
          onOpenStory={(storyId) => {
            setView({ kind: "story", project: view.project, storyId });
          }}
        />
      );
    case "story":
      return (
        <StoryEditor
          key={view.storyId}
          projectId={view.project.id}
          storyId={view.storyId}
          onBack={() => {
            setView({ kind: "project", project: view.project });
          }}
        />
      );
  }
}
