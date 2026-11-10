import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ErrorPayload } from "../../contracts/generated/ErrorPayload";
import type { Column } from "../../contracts/generated/Column";
import type { StoryDetail } from "../../contracts/generated/StoryDetail";
import type { StorySummary } from "../../contracts/generated/StorySummary";
import * as commands from "../../shared/ipc/commands";
import { StoriesSection } from "./StoriesSection";
import { StoryEditor } from "./StoryEditor";

// Solo se simulan las llamadas IPC; validación, errores y helpers son los reales.
vi.mock("../../shared/ipc/commands", async (importOriginal) => {
  const actual = await importOriginal<typeof commands>();
  return {
    ...actual,
    getStory: vi.fn(),
    updateStory: vi.fn(),
    archiveStory: vi.fn(),
    restoreStory: vi.fn(),
    listColumns: vi.fn(),
    listStories: vi.fn(),
    createStory: vi.fn(),
    setUnsavedChanges: vi.fn(),
    getStoryRelations: vi.fn(),
    listEpics: vi.fn(),
  };
});

const mocked = vi.mocked(commands);

function story(overrides: Partial<StoryDetail> = {}): StoryDetail {
  return {
    id: "s1",
    projectId: "p1",
    number: 7,
    columnId: "c1",
    position: 0,
    title: "Exportar CSV",
    description: "Texto inicial",
    priority: "medium",
    archivedAt: null,
    acceptedAt: null,
    criteria: [{ id: "k1", number: 1, text: "Descarga un archivo" }],
    revision: 1,
    createdAt: "2026-11-09T10:00:00.000Z",
    updatedAt: "2026-11-09T10:00:00.000Z",
    ...overrides,
  };
}

function column(
  id: string,
  name: string,
  category: Column["category"],
): Column {
  return {
    id,
    projectId: "p1",
    name,
    category,
    position: 0,
    wipLimit: null,
    revision: 1,
    cardCount: 0,
  };
}

function summaryOf(detail: StoryDetail): StorySummary {
  return {
    id: detail.id,
    projectId: detail.projectId,
    number: detail.number,
    columnId: detail.columnId,
    position: 0,
    title: detail.title,
    priority: detail.priority,
    archivedAt: null,
    acceptedAt: null,
    epicId: null,
    parentId: null,
    agentId: null,
    blockerReason: null,
    pendingPrerequisites: 0,
    criteriaCount: detail.criteria.length,
    revision: detail.revision + 1,
  };
}

function conflictWith(current: StoryDetail): ErrorPayload {
  return {
    code: "conflict",
    message: "Otra edición cambió estos datos.",
    recoverable: true,
    current,
  };
}

function renderEditor(onBack = vi.fn()) {
  const view = render(
    <StoryEditor
      projectId="p1"
      storyId="s1"
      onBack={onBack}
      onOpenStory={vi.fn()}
    />,
  );
  return { ...view, onBack };
}

beforeEach(() => {
  mocked.setUnsavedChanges.mockResolvedValue(undefined);
  mocked.getStoryRelations.mockResolvedValue({
    storyId: "s1",
    epic: null,
    agent: null,
    parent: null,
    children: [],
    prerequisites: [],
    dependents: [],
    pendingPrerequisites: 0,
  });
  mocked.listEpics.mockResolvedValue([]);
  mocked.listStories.mockResolvedValue([]);
  mocked.getStory.mockResolvedValue(story());
  mocked.listColumns.mockResolvedValue([]);
});

describe("StoryEditor", () => {
  it("muestra el error de carga con opción de reintentar", async () => {
    mocked.getStory.mockRejectedValueOnce({
      code: "not_found",
      message: "Historia no encontrado",
      recoverable: true,
    } satisfies ErrorPayload);
    renderEditor();

    expect(
      await screen.findByText(/No se pudo cargar la historia/),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Reintentar" }));
    expect(await screen.findByDisplayValue("Exportar CSV")).toBeInTheDocument();
  });

  it("indica cambios sin guardar, guarda y vuelve al proyecto con aviso", async () => {
    const user = userEvent.setup();
    mocked.updateStory.mockImplementation((input) =>
      Promise.resolve(
        story({
          title: input.title,
          revision: 2,
          criteria: [
            { id: "k1", number: 1, text: "Descarga un archivo" },
            { id: "k2", number: 2, text: "Con cabeceras" },
          ],
        }),
      ),
    );
    const { onBack } = renderEditor();

    const title = await screen.findByLabelText("Título");
    await user.clear(title);
    await user.type(title, "Exportar informe");
    await user.click(screen.getByRole("button", { name: "Añadir criterio" }));
    await user.type(screen.getByLabelText("Nuevo"), "Con cabeceras");
    expect(screen.getByText("Cambios sin guardar")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Guardar y volver" }));

    await waitFor(() => {
      expect(onBack).toHaveBeenCalledWith("Historia #7 guardada.");
    });
    expect(mocked.updateStory).toHaveBeenCalledWith({
      projectId: "p1",
      id: "s1",
      expectedRevision: 1,
      title: "Exportar informe",
      description: "Texto inicial",
      priority: "medium",
      criteria: [
        { id: "k1", text: "Descarga un archivo" },
        { text: "Con cabeceras" },
      ],
    });
  });

  it("sin cambios de contenido, «Guardar y volver» vuelve sin guardar", async () => {
    const user = userEvent.setup();
    const { onBack } = renderEditor();

    const button = await screen.findByRole("button", {
      name: "Guardar y volver",
    });
    expect(button).toBeEnabled();
    expect(button).toHaveAttribute("aria-busy", "false");
    await user.click(button);

    expect(onBack).toHaveBeenCalledWith(undefined);
    expect(mocked.updateStory).not.toHaveBeenCalled();
  });

  it("informa al host de los cambios sin guardar para proteger el cierre", async () => {
    const user = userEvent.setup();
    mocked.updateStory.mockImplementation((input) =>
      Promise.resolve(story({ title: input.title, revision: 2 })),
    );
    const { unmount, onBack } = renderEditor();

    const title = await screen.findByLabelText("Título");
    await user.type(title, "!");
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);

    await user.click(screen.getByRole("button", { name: "Guardar y volver" }));
    await waitFor(() => {
      expect(onBack).toHaveBeenCalled();
    });
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(false);

    // Salir del editor deja de bloquear el cierre.
    unmount();
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(false);
  });

  it("abrir otra historia desde relaciones con cambios sin guardar pide confirmación", async () => {
    const user = userEvent.setup();
    mocked.getStoryRelations.mockResolvedValue({
      storyId: "s1",
      epic: null,
      agent: null,
      parent: null,
      children: [
        {
          id: "s2",
          number: 8,
          title: "Subtarea",
          state: "pending",
          archived: false,
        },
      ],
      prerequisites: [],
      dependents: [],
      pendingPrerequisites: 0,
    });
    const onOpenStory = vi.fn();
    render(
      <StoryEditor
        projectId="p1"
        storyId="s1"
        onBack={vi.fn()}
        onOpenStory={onOpenStory}
      />,
    );

    await user.type(await screen.findByLabelText("Título"), "!");
    await user.click(
      await screen.findByRole("button", { name: "#8 Subtarea" }),
    );
    expect(onOpenStory).not.toHaveBeenCalled();
    expect(
      screen.getByText("Tienes cambios sin guardar. ¿Salir y descartarlos?"),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Salir sin guardar" }));
    expect(onOpenStory).toHaveBeenCalledWith("s2");
  });

  it("ante un conflicto conserva el borrador y permite mantenerlo", async () => {
    const user = userEvent.setup();
    // Otro editor cambió la descripción y borró CA-1.
    const theirs = story({
      description: "Texto de otra persona",
      criteria: [],
      revision: 2,
    });
    mocked.updateStory
      .mockRejectedValueOnce(conflictWith(theirs))
      .mockImplementationOnce((input) =>
        Promise.resolve(
          story({ description: input.description, revision: 3, criteria: [] }),
        ),
      );
    const { onBack } = renderEditor();

    const description = await screen.findByLabelText("Descripción en Markdown");
    await user.clear(description);
    await user.type(description, "Mi borrador largo");
    await user.click(screen.getByRole("button", { name: "Guardar y volver" }));

    expect(
      await screen.findByText("Otra edición guardó cambios mientras editabas"),
    ).toBeInTheDocument();
    expect(screen.getByText("Texto de otra persona")).toBeInTheDocument();
    expect(screen.getByLabelText("Descripción en Markdown")).toHaveValue(
      "Mi borrador largo",
    );

    await user.click(
      screen.getByRole("button", { name: "Conservar mi versión" }),
    );

    await waitFor(() => {
      expect(mocked.updateStory).toHaveBeenCalledTimes(2);
    });
    const retry = mocked.updateStory.mock.calls[1]?.[0];
    expect(retry?.expectedRevision).toBe(2);
    expect(retry?.description).toBe("Mi borrador largo");
    // CA-1 ya no existe en la versión vigente: se reenvía como criterio nuevo, no se pierde.
    expect(retry?.criteria).toEqual([{ text: "Descarga un archivo" }]);
    // El conflicto no volvió al proyecto; conservar la versión sí, tras guardarla.
    await waitFor(() => {
      expect(onBack).toHaveBeenCalledTimes(1);
    });
    expect(onBack).toHaveBeenCalledWith("Historia #7 guardada.");
  });

  it("puede descartar el borrador y adoptar la versión guardada", async () => {
    const user = userEvent.setup();
    mocked.updateStory.mockRejectedValueOnce(
      conflictWith(story({ title: "Título ajeno", revision: 2 })),
    );
    renderEditor();

    const title = await screen.findByLabelText("Título");
    await user.clear(title);
    await user.type(title, "Mío");
    await user.click(screen.getByRole("button", { name: "Guardar y volver" }));
    await user.click(
      await screen.findByRole("button", {
        name: "Descartar mi borrador y usar la guardada",
      }),
    );

    expect(screen.getByLabelText("Título")).toHaveValue("Título ajeno");
    expect(
      screen.queryByText("Otra edición guardó cambios mientras editabas"),
    ).toBeNull();
  });

  it("muestra los errores de validación junto al campo", async () => {
    const user = userEvent.setup();
    mocked.updateStory.mockRejectedValueOnce({
      code: "validation",
      message: "El título es obligatorio.",
      recoverable: true,
      field: "title",
    } satisfies ErrorPayload);
    renderEditor();

    const title = await screen.findByLabelText("Título");
    await user.type(title, " ");
    await user.click(screen.getByRole("button", { name: "Guardar y volver" }));

    expect(await screen.findByText("El título es obligatorio.")).toBeVisible();
    expect(screen.getByLabelText("Título")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });

  it("restaurar envía la columna que se muestra si la original ya no es válida", async () => {
    const user = userEvent.setup();
    // La columna original ("c-done") pasó a ser «hecho»: no aparece entre las opciones.
    mocked.getStory.mockResolvedValue(
      story({
        columnId: "c-done",
        archivedAt: "2026-11-09T11:00:00.000Z",
        position: null,
      }),
    );
    mocked.listColumns.mockResolvedValue([
      column("c-done", "Hecho", "done"),
      column("c-ready", "Listo", "ready"),
      column("c-review", "Revisión", "review"),
    ]);
    mocked.restoreStory.mockResolvedValue({
      ...summaryOf(story()),
      columnId: "c-ready",
    });
    renderEditor();

    const select = await screen.findByLabelText("Restaurar en la columna");
    await waitFor(() => {
      expect(select).toHaveValue("c-ready");
    });
    await user.click(screen.getByRole("button", { name: "Restaurar" }));

    expect(mocked.restoreStory).toHaveBeenCalledWith(
      expect.objectContaining({ columnId: "c-ready" }),
    );
  });

  it("una historia archivada es de solo lectura y ofrece restaurar", async () => {
    mocked.getStory.mockResolvedValue(
      story({ archivedAt: "2026-11-09T11:00:00.000Z", position: null }),
    );
    renderEditor();

    expect(
      await screen.findByRole("button", { name: "Restaurar" }),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Título")).toBeNull();
    expect(screen.getByText("CA-1: Descarga un archivo")).toBeInTheDocument();
  });
});

describe("StoriesSection", () => {
  it("muestra el estado vacío", async () => {
    mocked.listStories.mockResolvedValue([]);
    render(<StoriesSection projectId="p1" onOpenStory={vi.fn()} />);

    expect(await screen.findByText(/Aún no hay historias/)).toBeInTheDocument();
  });
});
