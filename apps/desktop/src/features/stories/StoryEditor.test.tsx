import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ErrorPayload } from "../../contracts/generated/ErrorPayload";
import type { StoryReview } from "../../contracts/generated/StoryReview";
import type { Column } from "../../contracts/generated/Column";
import type { RunDetail } from "../../contracts/generated/RunDetail";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import type { StoryClaim } from "../../contracts/generated/StoryClaim";
import type { StoryDetail } from "../../contracts/generated/StoryDetail";
import type { StorySummary } from "../../contracts/generated/StorySummary";
import * as commands from "../../shared/ipc/commands";
import { emitAgentChange, fakeAgentChanges } from "../../test/agentChanges";
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
    releaseClaim: vi.fn(),
    setStoryAgent: vi.fn(),
    listAgents: vi.fn(),
    getStoryRelations: vi.fn(),
    listEpics: vi.fn(),
    listActiveClaims: vi.fn(),
    getStoryReview: vi.fn(),
    listStoryActivity: vi.fn(),
    listStoryRuns: vi.fn(),
    createRun: vi.fn(),
    cancelRun: vi.fn(),
    getRun: vi.fn(),
    onAgentChange: vi.fn(),
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

const noRelations = {
  storyId: "s1",
  epic: null,
  agent: null,
  parent: null,
  children: [],
  prerequisites: [],
  dependents: [],
  pendingPrerequisites: 0,
};

const roboClaim: StoryClaim = {
  claimId: "c1",
  storyId: "s1",
  storyNumber: 7,
  owner: { agentId: "a1", agentNumber: 2, agentName: "Robo" },
  claimedAt: "2026-11-12T10:00:00Z",
  expiresAt: "2099-11-12T10:30:00Z",
  renewedAt: null,
};

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

const emptyReview: StoryReview = {
  storyId: "s1",
  acceptedAt: null,
  blocker: null,
  deliveries: [],
  history: [],
};

beforeEach(() => {
  vi.clearAllMocks();
  fakeAgentChanges(mocked.onAgentChange);
  mocked.getStoryReview.mockResolvedValue(emptyReview);
  mocked.listStoryActivity.mockResolvedValue({
    lastReportedAt: null,
    entries: [],
  });
  mocked.setUnsavedChanges.mockResolvedValue(undefined);
  mocked.listStoryRuns.mockResolvedValue([]);
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
  mocked.listActiveClaims.mockResolvedValue([]);
  mocked.listAgents.mockResolvedValue([]);
});

describe("StoryEditor", () => {
  it("muestra el error de carga con opción de reintentar", async () => {
    mocked.getStory.mockRejectedValueOnce({
      code: "not_found",
      message: "No se encontró: Historia.",
      recoverable: true,
    } satisfies ErrorPayload);
    renderEditor();

    expect(
      await screen.findByText(/No se pudo cargar la historia/),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Reintentar" }));
    expect(await screen.findByDisplayValue("Exportar CSV")).toBeInTheDocument();
  });

  it("muestra el reclamo vigente de la historia", async () => {
    mocked.listActiveClaims.mockResolvedValue([
      {
        claimId: "c1",
        storyId: "s1",
        storyNumber: 7,
        owner: { agentId: "a1", agentNumber: 2, agentName: "Robo" },
        claimedAt: "2026-11-12T10:00:00Z",
        expiresAt: "2099-11-12T10:30:00Z",
        renewedAt: null,
      },
    ]);
    renderEditor();

    expect(
      await screen.findByRole("heading", { name: "Reclamo" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("A-2 Robo", { selector: "strong" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Liberar reclamo…" }),
    ).toBeInTheDocument();
  });

  it("liberar el reclamo recarga la asignación en «Relaciones»", async () => {
    const user = userEvent.setup();
    const robo = {
      id: "a1",
      number: 2,
      name: "Robo",
      avatar: { base: 0, hair: 0, eyes: 0, accessory: 0, palette: 0 },
    } as never;
    mocked.listAgents.mockResolvedValue([robo]);
    mocked.getStoryRelations
      .mockResolvedValueOnce({ ...noRelations, agent: robo })
      .mockResolvedValue(noRelations);
    mocked.listActiveClaims
      .mockResolvedValueOnce([roboClaim])
      .mockResolvedValue([]);
    mocked.releaseClaim.mockResolvedValue(undefined);
    renderEditor();

    const select = await screen.findByLabelText("Agente asignado");
    await waitFor(() => {
      expect(select).toHaveValue("a1");
    });
    await user.click(
      await screen.findByRole("button", { name: "Liberar reclamo…" }),
    );
    await user.click(screen.getByRole("button", { name: "Liberar" }));

    expect(mocked.releaseClaim).toHaveBeenCalledWith({
      projectId: "p1",
      claimId: "c1",
    });
    await waitFor(() => {
      expect(screen.getByLabelText("Agente asignado")).toHaveValue("");
    });
    // «Relaciones» e «Intentos» (agente por defecto) consultan la asignación al montar y al
    // liberar: dos veces cada uno.
    expect(mocked.getStoryRelations).toHaveBeenCalledTimes(4);
  });

  it("quitar el agente en «Relaciones» vuelve a consultar el reclamo", async () => {
    const user = userEvent.setup();
    const robo = {
      id: "a1",
      number: 2,
      name: "Robo",
      avatar: { base: 0, hair: 0, eyes: 0, accessory: 0, palette: 0 },
    } as never;
    mocked.listAgents.mockResolvedValue([robo]);
    mocked.getStoryRelations.mockResolvedValue({ ...noRelations, agent: robo });
    mocked.setStoryAgent.mockResolvedValue(noRelations);
    mocked.listActiveClaims
      .mockResolvedValueOnce([roboClaim])
      .mockResolvedValue([]);
    renderEditor();

    expect(
      await screen.findByText("A-2 Robo", { selector: "strong" }),
    ).toBeInTheDocument();
    const select = await screen.findByLabelText("Agente asignado");
    await waitFor(() => {
      expect(select).toHaveValue("a1");
    });
    await user.selectOptions(select, "");

    expect(mocked.setStoryAgent).toHaveBeenCalledWith({
      projectId: "p1",
      storyId: "s1",
      agentId: null,
    });
    expect(await screen.findByText("Sin reclamo.")).toBeInTheDocument();
    expect(mocked.listActiveClaims).toHaveBeenCalledTimes(2);
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
    // El aviso al host llega en un efecto: puede ir justo después de `onBack`.
    await waitFor(() => {
      expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(false);
    });

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

describe("StoryEditor con avisos de agentes", () => {
  const claimedAt = "2026-11-12T10:00:00Z";
  const deliveredAt = "2026-11-12T10:20:00Z";

  /** Respuestas del núcleo después de que el agente entregue E2 y cree la subtarea #12. */
  function agentDelivered() {
    mocked.getStory.mockResolvedValue(story({ columnId: "c2", revision: 2 }));
    mocked.getStoryReview.mockResolvedValue({
      ...emptyReview,
      deliveries: [
        {
          id: "d2",
          storyId: "s1",
          number: 2,
          source: "reported",
          agentId: "a1",
          agentName: "Robo",
          summary: "Exportación lista",
          references: [],
          status: "pending",
          returnReason: null,
          createdAt: deliveredAt,
          decidedAt: null,
        },
      ],
    });
    mocked.listStoryActivity.mockResolvedValue({
      lastReportedAt: deliveredAt,
      entries: [
        {
          kind: "delivered",
          agent: roboClaim.owner,
          detail: "E2",
          at: deliveredAt,
        },
        {
          kind: "subtask_created",
          agent: roboClaim.owner,
          detail: "#12",
          at: claimedAt,
        },
        {
          kind: "claimed",
          agent: roboClaim.owner,
          detail: null,
          at: claimedAt,
        },
      ],
    });
    // Entregar cierra el reclamo.
    mocked.listActiveClaims.mockResolvedValue([]);
    mocked.getStoryRelations.mockResolvedValue({
      ...noRelations,
      children: [
        {
          id: "s12",
          number: 12,
          title: "Subtarea del agente",
          state: "pending",
          archived: false,
        },
      ],
    });
  }

  function claimedByRobo() {
    mocked.listActiveClaims.mockResolvedValue([roboClaim]);
    mocked.listStoryActivity.mockResolvedValue({
      lastReportedAt: claimedAt,
      entries: [
        {
          kind: "claimed",
          agent: roboClaim.owner,
          detail: null,
          at: claimedAt,
        },
      ],
    });
  }

  it("recarga reclamo, actividad, revisión y subtareas sin perder el borrador", async () => {
    const user = userEvent.setup();
    claimedByRobo();
    renderEditor();

    expect(await screen.findByText("Reclamó")).toBeInTheDocument();
    const claimRegion = await screen.findByRole("region", { name: "Reclamo" });
    expect(
      await within(claimRegion).findByText("A-2 Robo", { selector: "strong" }),
    ).toBeInTheDocument();
    // Borrador en el contenido y texto a medio escribir en un panel de revisión.
    const title = await screen.findByLabelText("Título");
    await user.type(title, " ya");
    const reason = await screen.findByLabelText("Motivo del bloqueo");
    await user.type(reason, "Esperando");
    expect(screen.getByText("Cambios sin guardar")).toBeInTheDocument();

    agentDelivered();
    await emitAgentChange({ projectId: "p1", storyIds: ["s1", "s12"] });

    expect(await screen.findByText("Entregó E2")).toBeInTheDocument();
    expect(screen.getByText("Creó la subtarea #12")).toBeInTheDocument();
    expect(
      await screen.findByText("1 entrega pendiente de revisión"),
    ).toBeInTheDocument();
    expect(await screen.findByText("Sin reclamo.")).toBeInTheDocument();
    expect(
      await screen.findByRole("button", { name: "#12 Subtarea del agente" }),
    ).toBeInTheDocument();
    // Nada de lo escrito se pierde.
    expect(screen.getByLabelText("Título")).toHaveValue("Exportar CSV ya");
    expect(screen.getByLabelText("Motivo del bloqueo")).toHaveValue(
      "Esperando",
    );
    expect(screen.getByText("Cambios sin guardar")).toBeInTheDocument();

    // El borrador sigue ligado a la revisión que editó: el núcleo detectará el conflicto.
    mocked.updateStory.mockRejectedValue(
      conflictWith(story({ columnId: "c2", revision: 2 })),
    );
    await user.click(screen.getByRole("button", { name: "Guardar y volver" }));
    expect(mocked.updateStory).toHaveBeenCalledWith(
      expect.objectContaining({
        expectedRevision: 1,
        title: "Exportar CSV ya",
      }),
    );
    expect(screen.getByLabelText("Título")).toHaveValue("Exportar CSV ya");
  });

  it("sin borrador, el formulario toma la revisión nueva", async () => {
    const user = userEvent.setup();
    mocked.updateStory.mockImplementation((input) =>
      Promise.resolve(story({ title: input.title, revision: 3 })),
    );
    renderEditor();
    await screen.findByLabelText("Título");

    agentDelivered();
    await emitAgentChange({ projectId: "p1", storyIds: ["s1"] });
    expect(await screen.findByText("Entregó E2")).toBeInTheDocument();

    await user.type(screen.getByLabelText("Título"), "!");
    await user.click(screen.getByRole("button", { name: "Guardar y volver" }));
    expect(mocked.updateStory).toHaveBeenCalledWith(
      expect.objectContaining({ expectedRevision: 2 }),
    );
  });

  it("ignora los avisos de otro proyecto o de otra historia", async () => {
    claimedByRobo();
    renderEditor();
    expect(await screen.findByText("Reclamó")).toBeInTheDocument();
    await screen.findByLabelText("Título");

    agentDelivered();
    await emitAgentChange({ projectId: "p2", storyIds: ["s1"] });
    await emitAgentChange({ projectId: "p1", storyIds: ["s9"] });
    await act(async () => {
      await Promise.resolve();
    });

    expect(mocked.getStory).toHaveBeenCalledTimes(1);
    expect(mocked.listStoryActivity).toHaveBeenCalledTimes(1);
    expect(mocked.getStoryReview).toHaveBeenCalledTimes(1);
    expect(screen.queryByText("Entregó E2")).not.toBeInTheDocument();
  });
});

describe("StoriesSection", () => {
  it("muestra el estado vacío", async () => {
    mocked.listStories.mockResolvedValue([]);
    render(<StoriesSection projectId="p1" onOpenStory={vi.fn()} />);

    expect(await screen.findByText(/Aún no hay historias/)).toBeInTheDocument();
  });

  it("un aviso de un agente del proyecto recarga la lista sin perder lo escrito", async () => {
    const user = userEvent.setup();
    const parent = summaryOf(story());
    mocked.listStories.mockResolvedValue([parent]);
    render(<StoriesSection projectId="p1" onOpenStory={vi.fn()} />);
    await screen.findByRole("button", { name: /^#7 Exportar CSV/ });
    await user.type(
      screen.getByLabelText("Título de la nueva historia"),
      "Idea a medias",
    );

    mocked.listStories.mockResolvedValue([
      parent,
      {
        ...parent,
        id: "s12",
        number: 12,
        title: "Subtarea del agente",
        parentId: "s1",
      },
    ]);
    await emitAgentChange({ projectId: "p1", storyIds: ["s1", "s12"] });

    const sub = await screen.findByRole("button", {
      name: /^#12 Subtarea del agente/,
    });
    expect(sub).toHaveTextContent("subtarea de #7");
    expect(screen.getByLabelText("Título de la nueva historia")).toHaveValue(
      "Idea a medias",
    );
    expect(screen.queryByText("Cargando historias…")).not.toBeInTheDocument();
    expect(mocked.listStories).toHaveBeenCalledTimes(2);
  });

  it("ignora los avisos de agentes de otro proyecto", async () => {
    mocked.listStories.mockResolvedValue([summaryOf(story())]);
    render(<StoriesSection projectId="p1" onOpenStory={vi.fn()} />);
    await screen.findByRole("button", { name: /^#7 Exportar CSV/ });

    await emitAgentChange({ projectId: "p2", storyIds: ["s1"] });

    expect(mocked.listStories).toHaveBeenCalledTimes(1);
  });
});

describe("StoryEditor · intentos (HU-017)", () => {
  const queued: RunSummary = {
    id: "r1",
    projectId: "p1",
    storyId: "s1",
    number: 1,
    state: "queued",
    agentId: "a1",
    agentNumber: 2,
    agentName: "Robo",
    createdAt: "2026-11-12T10:00:00Z",
    updatedAt: "2026-11-12T10:00:00Z",
    endedAt: null,
    endReason: null,
    usage: { inputTokens: null, outputTokens: null, costUsdMicros: null },
    lastSequence: 1,
    snapshotSha256: "ab".repeat(32),
    startRequestedAt: null,
    cancelRequestedAt: null,
  };
  const frozen: RunDetail = {
    run: queued,
    snapshot: {
      schemaVersion: 1,
      createdAt: "2026-11-12T10:00:00Z",
      story: {
        id: "s1",
        number: 7,
        title: "Título al preparar",
        description: "Descripción al preparar",
        priority: "medium",
        revision: 1,
        column: { name: "Listo", category: "ready" },
      },
      criteria: [{ number: 1, text: "Criterio al preparar" }],
      epic: null,
      parent: null,
      prerequisites: [],
      agent: {
        id: "a1",
        number: 2,
        name: "Robo",
        role: "Rol",
        provider: "P",
        model: "M",
        capabilities: [],
      },
      repositories: [],
      instructions: "# Instrucciones",
      note: null,
    },
    snapshotIntact: true,
    events: [],
    eventsTruncated: false,
    startRequest: null,
    cancelReason: null,
    worktree: null,
    approvals: [],
  };

  it("el detalle muestra la instantánea, no la historia actual", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns.mockResolvedValue([queued]);
    mocked.getRun.mockResolvedValue(frozen);
    renderEditor();

    // La historia actual ya no se parece a la instantánea.
    expect(await screen.findByDisplayValue("Exportar CSV")).toBeInTheDocument();
    await user.click(
      await screen.findByRole("button", { name: "Ver detalle del intento #1" }),
    );
    const region = await screen.findByRole("region", {
      name: "Detalle del intento #1",
    });
    expect(region).toHaveTextContent("Título al preparar");
    expect(region).toHaveTextContent("Descripción al preparar");
    expect(region).toHaveTextContent("CA-1: Criterio al preparar");
    expect(region).not.toHaveTextContent("Exportar CSV");
    expect(region).not.toHaveTextContent("Texto inicial");
    expect(region).not.toHaveTextContent("Descarga un archivo");
  });

  it("archivar la historia vuelve a consultar los intentos", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns.mockResolvedValueOnce([queued]).mockResolvedValue([
      {
        ...queued,
        state: "cancelled",
        endedAt: "2026-11-12T11:00:00Z",
        endReason: "La historia se archivó.",
      },
    ]);
    mocked.archiveStory.mockResolvedValue(summaryOf(story()));
    mocked.getStory
      .mockResolvedValueOnce(story())
      .mockResolvedValue(
        story({ archivedAt: "2026-11-12T11:00:00Z", position: null }),
      );
    renderEditor();

    expect(await screen.findByText("En cola")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Archivar" }));
    await user.click(
      within(
        screen.getByRole("group", { name: "Confirmar archivo" }),
      ).getByRole("button", { name: "Archivar" }),
    );

    expect(
      await screen.findByText("La historia se archivó."),
    ).toBeInTheDocument();
    expect(screen.getByText("Cancelado")).toBeInTheDocument();
    expect(
      screen.getByText(/está archivada: restáurala para preparar un intento/),
    ).toBeInTheDocument();
    expect(mocked.listStoryRuns).toHaveBeenCalledTimes(2);
  });

  it("una nota sin enviar cuenta como cambio sin guardar", async () => {
    const user = userEvent.setup();
    const { onBack } = renderEditor();

    await user.type(
      await screen.findByLabelText("Nota para el agente (opcional)"),
      "hola",
    );
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);

    await user.click(screen.getByRole("button", { name: "← Proyecto" }));
    expect(
      screen.getByText("Tienes cambios sin guardar. ¿Salir y descartarlos?"),
    ).toBeInTheDocument();
    expect(onBack).not.toHaveBeenCalled();
  });
});
