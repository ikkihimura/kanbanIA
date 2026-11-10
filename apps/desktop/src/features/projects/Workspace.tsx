import { useCallback, useEffect, useRef, useState } from "react";
import type { Project } from "../../contracts/generated/Project";
import {
  describeError,
  getLastOpenedProject,
  openProject,
} from "../../shared/ipc/commands";
import { ProjectList } from "./ProjectList";
import { StoryEditor } from "../stories/StoryEditor";
import { TeamView } from "../team/TeamView";
import { ProjectView, type ProjectTab } from "./ProjectView";

type View =
  | { kind: "starting" }
  | { kind: "list"; notice?: string }
  | { kind: "project"; project: Project; notice?: string }
  | { kind: "story"; project: Project; storyId: string }
  | { kind: "team"; from: View };

/** Navegación entre la lista de proyectos y un proyecto. Al iniciar reabre el último abierto. */
export function Workspace() {
  const [view, setView] = useState<View>({ kind: "starting" });
  // Solo cuenta la última apertura pedida: clics rápidos no se aplican fuera de orden.
  const latestOpen = useRef(0);
  // La pestaña del proyecto se conserva al abrir una historia y volver.
  const [tab, setTab] = useState<ProjectTab>("board");

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

  const open = useCallback((projectId: string, notice?: string) => {
    const ticket = ++latestOpen.current;
    openProject(projectId).then(
      (project) => {
        if (ticket === latestOpen.current)
          setView(
            notice
              ? { kind: "project", project, notice }
              : { kind: "project", project },
          );
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

  // Navegación principal; no aparece en el editor de historias, que tiene su propia salida
  // protegida frente a cambios sin guardar.
  const nav = (current: "projects" | "team") => (
    <nav className="main-nav" aria-label="Secciones">
      <button
        type="button"
        className="main-nav__item"
        aria-current={current === "projects" ? "page" : undefined}
        onClick={() => {
          if (view.kind === "team") setView(view.from);
          else if (current !== "projects") backToList();
        }}
      >
        Proyectos
      </button>
      <button
        type="button"
        className="main-nav__item"
        aria-current={current === "team" ? "page" : undefined}
        onClick={() => {
          if (view.kind !== "team") setView({ kind: "team", from: view });
        }}
      >
        Equipo
      </button>
    </nav>
  );

  switch (view.kind) {
    case "starting":
      return (
        <p className="view" role="status">
          Abriendo…
        </p>
      );
    case "team":
      return (
        <>
          {nav("team")}
          <TeamView />
        </>
      );
    case "list":
      return (
        <>
          {nav("projects")}
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
        <>
          {nav("projects")}
          <ProjectView
            project={view.project}
            notice={view.notice}
            tab={tab}
            onTabChange={setTab}
            onBack={backToList}
            onChanged={replaceProject}
            onOpenStory={(storyId) => {
              setView({ kind: "story", project: view.project, storyId });
            }}
          />
        </>
      );
    case "story":
      return (
        <StoryEditor
          key={view.storyId}
          projectId={view.project.id}
          storyId={view.storyId}
          onBack={(notice) => {
            setView({ kind: "project", project: view.project, notice });
          }}
          onOpenStory={(storyId) => {
            setView({ kind: "story", project: view.project, storyId });
          }}
        />
      );
  }
}
