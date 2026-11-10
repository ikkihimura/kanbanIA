import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Epic } from "../../contracts/generated/Epic";
import type { StoryRelations } from "../../contracts/generated/StoryRelations";
import type { StorySummary } from "../../contracts/generated/StorySummary";
import * as commands from "../../shared/ipc/commands";
import { EpicsSection } from "../epics/EpicsSection";
import { RelationsPanel } from "./RelationsPanel";

vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  getStoryRelations: vi.fn(),
  listEpics: vi.fn(),
  listStories: vi.fn(),
  setStoryEpic: vi.fn(),
  setStoryParent: vi.fn(),
  addStoryDependency: vi.fn(),
  removeStoryDependency: vi.fn(),
  createEpic: vi.fn(),
  listAgents: vi.fn(),
  setStoryAgent: vi.fn(),
  updateEpic: vi.fn(),
}));

const mocked = vi.mocked(commands);

function relations(overrides: Partial<StoryRelations> = {}): StoryRelations {
  return {
    storyId: "a",
    epic: null,
    agent: null,
    parent: null,
    children: [],
    prerequisites: [],
    dependents: [],
    pendingPrerequisites: 0,
    ...overrides,
  };
}

function summary(id: string, number: number, title: string): StorySummary {
  return {
    id,
    projectId: "p1",
    number,
    columnId: "c1",
    position: number,
    title,
    priority: "medium",
    archivedAt: null,
    acceptedAt: null,
    epicId: null,
    parentId: null,
    agentId: null,
    blockerReason: null,
    pendingPrerequisites: 0,
    criteriaCount: 0,
    revision: 1,
  };
}

function epic(overrides: Partial<Epic> = {}): Epic {
  return {
    id: "e1",
    projectId: "p1",
    number: 1,
    title: "Pagos",
    description: "",
    revision: 1,
    progress: { accepted: 0, total: 0, percent: null },
    ...overrides,
  };
}

function renderPanel(readOnly = false, onOpenStory = vi.fn()) {
  render(
    <RelationsPanel
      projectId="p1"
      storyId="a"
      readOnly={readOnly}
      onOpenStory={onOpenStory}
    />,
  );
  return onOpenStory;
}

beforeEach(() => {
  mocked.listAgents.mockResolvedValue([
    {
      id: "g1",
      number: 1,
      name: "Robota",
      role: "Backend",
      provider: "",
      model: "",
      capabilities: [],
      avatar: {
        head: 2,
        skin: 6,
        eyes: 4,
        mouth: 3,
        hair: 6,
        hairColor: 7,
        accessory: 5,
        background: 3,
      },
      assignedStories: 0,
      revision: 1,
    },
  ]);
  mocked.getStoryRelations.mockResolvedValue(relations());
  mocked.listEpics.mockResolvedValue([epic()]);
  mocked.listStories.mockResolvedValue([
    summary("a", 1, "Funcionalidad"),
    summary("b", 2, "API"),
  ]);
});

describe("RelationsPanel", () => {
  it("muestra los prerequisitos sin aceptar y el bloqueo", async () => {
    mocked.getStoryRelations.mockResolvedValue(
      relations({
        prerequisites: [
          {
            id: "b",
            number: 2,
            title: "API",
            state: "pending",
            archived: false,
          },
          {
            id: "c",
            number: 3,
            title: "Esquema",
            state: "accepted",
            archived: false,
          },
        ],
        pendingPrerequisites: 1,
      }),
    );
    renderPanel();

    expect(
      await screen.findByText(/Bloqueada: 1 prerequisito sin aceptar/),
    ).toBeInTheDocument();
    expect(screen.getByText("Sin aceptar")).toBeInTheDocument();
    expect(screen.getByText("Aceptada")).toBeInTheDocument();
  });

  it("muestra el error de ciclo que devuelve el núcleo y no cambia nada", async () => {
    const user = userEvent.setup();
    mocked.addStoryDependency.mockRejectedValue({
      code: "cycle",
      message:
        "#2 ya depende (directa o indirectamente) de #1: añadir esta dependencia crearía un ciclo.",
      recoverable: true,
      field: "dependsOnId",
    });
    renderPanel();

    await user.selectOptions(
      await screen.findByLabelText("Añadir prerequisito"),
      "b",
    );
    await user.click(screen.getByRole("button", { name: "Añadir" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "crearía un ciclo",
    );
    expect(mocked.addStoryDependency).toHaveBeenCalledWith({
      projectId: "p1",
      storyId: "a",
      dependsOnId: "b",
    });
    expect(screen.getByText("Sin prerequisitos.")).toBeInTheDocument();
  });

  it("asigna épica y refleja la respuesta", async () => {
    const user = userEvent.setup();
    mocked.setStoryEpic.mockResolvedValue(
      relations({ epic: { id: "e1", number: 1, title: "Pagos" } }),
    );
    renderPanel();

    await user.selectOptions(await screen.findByLabelText("Épica"), "e1");

    await waitFor(() => {
      expect(screen.getByLabelText("Épica")).toHaveValue("e1");
    });
    expect(mocked.setStoryEpic).toHaveBeenCalledWith({
      projectId: "p1",
      storyId: "a",
      epicId: "e1",
    });
  });

  it("en solo lectura no permite cambiar épica, padre ni añadir prerequisitos", async () => {
    renderPanel(true);

    expect(await screen.findByLabelText("Épica")).toBeDisabled();
    expect(screen.getByLabelText("Subtarea de")).toBeDisabled();
    expect(screen.queryByLabelText("Añadir prerequisito")).toBeNull();
  });

  it("abre una historia relacionada", async () => {
    const user = userEvent.setup();
    mocked.getStoryRelations.mockResolvedValue(
      relations({
        children: [
          {
            id: "b",
            number: 2,
            title: "API",
            state: "pending",
            archived: false,
          },
        ],
      }),
    );
    const onOpen = renderPanel();

    await user.click(await screen.findByRole("button", { name: "#2 API" }));
    expect(onOpen).toHaveBeenCalledWith("b");
  });
});

describe("RelationsPanel: confirmación y foco", () => {
  it("confirma visiblemente y no pierde el foco al cambiar la épica", async () => {
    const user = userEvent.setup();
    mocked.setStoryEpic.mockResolvedValue(
      relations({ epic: { id: "e1", number: 1, title: "Pagos" } }),
    );
    renderPanel();

    const select = await screen.findByLabelText("Épica");
    await user.selectOptions(select, "e1");

    expect(await screen.findByText("✓ Épica guardada.")).toBeVisible();
    expect(select).toBeEnabled();
    expect(select).toHaveFocus();
  });

  it("tras añadir o quitar un prerequisito, el foco vuelve al selector", async () => {
    const user = userEvent.setup();
    const withB = relations({
      prerequisites: [
        { id: "b", number: 2, title: "API", state: "pending", archived: false },
      ],
      pendingPrerequisites: 1,
    });
    mocked.addStoryDependency.mockResolvedValue(withB);
    mocked.removeStoryDependency.mockResolvedValue(relations());
    renderPanel();

    const select = await screen.findByLabelText("Añadir prerequisito");
    await user.selectOptions(select, "b");
    await user.click(screen.getByRole("button", { name: "Añadir" }));
    expect(await screen.findByText("✓ Prerequisito guardado.")).toBeVisible();
    await waitFor(() => {
      expect(screen.getByLabelText("Añadir prerequisito")).toHaveFocus();
    });

    await user.click(
      screen.getByRole("button", { name: "Quitar dependencia de #2 API" }),
    );
    expect(await screen.findByText("✓ Ya no depende de #2 API.")).toBeVisible();
    await waitFor(() => {
      expect(screen.getByLabelText("Añadir prerequisito")).toHaveFocus();
    });
  });

  it("muestra el error de ciclo al elegir historia padre", async () => {
    const user = userEvent.setup();
    mocked.setStoryParent.mockRejectedValue({
      code: "cycle",
      message:
        "#2 ya es subtarea (directa o indirecta) de #1: no puede ser su padre.",
      recoverable: true,
      field: "parentId",
    });
    renderPanel();

    await user.selectOptions(await screen.findByLabelText("Subtarea de"), "b");

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "no puede ser su padre",
    );
    // La relación no cambió: el selector vuelve a mostrar la vigente.
    expect(screen.getByLabelText("Subtarea de")).toHaveValue("");
  });
});

describe("RelationsPanel: agente asignado", () => {
  it("asigna un agente sin simular actividad", async () => {
    const user = userEvent.setup();
    mocked.setStoryAgent.mockResolvedValue(
      relations({
        agent: {
          id: "g1",
          number: 1,
          name: "Robota",
          role: "Backend",
          avatar: {
            head: 2,
            skin: 6,
            eyes: 4,
            mouth: 3,
            hair: 6,
            hairColor: 7,
            accessory: 5,
            background: 3,
          },
        },
      }),
    );
    renderPanel();

    const select = await screen.findByLabelText("Agente asignado");
    expect(select).toHaveAccessibleDescription(
      "Asignar no inicia ninguna ejecución ni actividad.",
    );
    await user.selectOptions(select, "g1");

    expect(await screen.findByText("✓ Agente asignado.")).toBeVisible();
    expect(mocked.setStoryAgent).toHaveBeenCalledWith({
      projectId: "p1",
      storyId: "a",
      agentId: "g1",
    });
    expect(screen.queryByText(/trabajando|en curso|activo/i)).toBeNull();
  });
});

describe("EpicsSection", () => {
  it("define el avance sin historias y con historias", async () => {
    mocked.listEpics.mockResolvedValue([
      epic(),
      epic({
        id: "e2",
        number: 2,
        title: "Informes",
        progress: { accepted: 2, total: 3, percent: 66 },
      }),
    ]);
    render(<EpicsSection projectId="p1" />);

    expect(await screen.findByText("Sin historias")).toBeInTheDocument();
    expect(screen.getByText("2 de 3 aceptadas (66 %)")).toBeInTheDocument();
    // Solo la épica con historias tiene barra; la vacía no muestra 0 % engañoso.
    const bars = screen.getAllByRole("progressbar");
    expect(bars).toHaveLength(1);
    expect(bars[0]).toHaveAccessibleName("Avance de E-2 Informes");
    expect(bars[0]).toHaveAttribute("value", "2");
    expect(bars[0]).toHaveAttribute("max", "3");
  });
});
