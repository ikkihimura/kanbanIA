import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { StoryDetail } from "../../contracts/generated/StoryDetail";
import type { StorySummary } from "../../contracts/generated/StorySummary";
import * as commands from "../../shared/ipc/commands";
import { Board } from "./Board";
import { applyMove } from "./boardModel";
import { card, column } from "./boardFixtures";

vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  listColumns: vi.fn(),
  listStories: vi.fn(),
  listEpics: vi.fn(),
  listAgents: vi.fn(),
  moveStory: vi.fn(),
  getStory: vi.fn(),
  acceptStory: vi.fn(),
}));

const mocked = vi.mocked(commands);

const columns = [
  column("todo", 0, { name: "Por hacer", category: "ready" }),
  column("doing", 1, { name: "En curso", wipLimit: 2 }),
  column("done", 2, { name: "Hecho", category: "done" }),
];

let server: StorySummary[];

function onServer(id: string): StorySummary {
  const found = server.find((s) => s.id === id);
  if (!found) throw new Error(`sin historia ${id}`);
  return found;
}

function cardIn(columnName: string, number: number) {
  return within(screen.getByRole("group", { name: columnName })).getByRole(
    "button",
    { name: new RegExp(`^#${String(number)} `) },
  );
}

function queryCardIn(columnName: string, number: number) {
  return within(screen.getByRole("group", { name: columnName })).queryByRole(
    "button",
    { name: new RegExp(`^#${String(number)} `) },
  );
}

function renderBoard() {
  const onOpenStory = vi.fn();
  render(<Board projectId="p1" onOpenStory={onOpenStory} />);
  return { onOpenStory };
}

beforeEach(() => {
  vi.clearAllMocks();
  server = [
    card("s1", "todo", 0, {
      title: "Login",
      priority: "high",
      epicId: "e1",
      agentId: "a1",
    }),
    card("s2", "todo", 1, { title: "API", blockerReason: "Falta acceso" }),
    card("s3", "doing", 0, { title: "Pantalla", pendingPrerequisites: 1 }),
    card("s4", "done", 0, {
      title: "Base",
      acceptedAt: "2026-11-01T00:00:00Z",
    }),
    card("s5", "todo", null, { title: "Archivada" }),
  ];
  mocked.listColumns.mockResolvedValue(columns);
  mocked.listStories.mockImplementation(() => Promise.resolve(server));
  mocked.listEpics.mockResolvedValue([
    {
      id: "e1",
      projectId: "p1",
      number: 1,
      title: "Acceso",
      description: "",
      revision: 1,
    } as never,
  ]);
  mocked.listAgents.mockResolvedValue([
    {
      id: "a1",
      number: 1,
      name: "Jean Claude",
      avatar: { base: 0, hair: 0, eyes: 0, accessory: 0, palette: 0 },
    } as never,
  ]);
});

describe("Board", () => {
  it("muestra columnas, WIP y la información de cada tarjeta", async () => {
    renderBoard();
    const login = await screen.findByRole("button", { name: /^#1 Login/ });
    const item = login.closest("li");
    if (!item) throw new Error("sin tarjeta");
    expect(item).toHaveTextContent("Prioridad alta · E-1");
    expect(item).toHaveTextContent("Jean Claude");
    expect(screen.getByText("Bloqueada: Falta acceso")).toBeInTheDocument();
    expect(screen.getByText("Espera 1 prerequisito")).toBeInTheDocument();
    expect(screen.getByText("✓ Aceptada")).toBeInTheDocument();
    expect(screen.getByText("1 / 2 WIP")).toBeInTheDocument();
    expect(screen.queryByText(/Archivada/)).not.toBeInTheDocument();
  });

  it("filtra por texto y por épica, y agrupa en carriles", async () => {
    const user = userEvent.setup();
    renderBoard();
    await screen.findByRole("button", { name: /^#1 Login/ });
    await user.type(screen.getByLabelText("Buscar"), "api");
    expect(
      screen.queryByRole("button", { name: /^#1 Login/ }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^#2 API/ })).toBeInTheDocument();
    expect(screen.getByText(/Mostrando 1 de 4 tarjetas/)).toBeInTheDocument();
    await user.clear(screen.getByLabelText("Buscar"));

    await user.selectOptions(screen.getByLabelText("Épica"), "e1");
    expect(
      screen.queryByRole("button", { name: /^#2 API/ }),
    ).not.toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText("Épica"), "");

    await user.selectOptions(screen.getByLabelText("Carriles"), "epic");
    expect(
      screen.getByRole("group", { name: "Carril E-1 Acceso" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("group", { name: "Carril Sin épica" }),
    ).toBeInTheDocument();
    expect(
      within(
        screen.getByRole("group", { name: "Por hacer, carril E-1 Acceso" }),
      ).getByRole("button", { name: /^#1 Login/ }),
    ).toBeInTheDocument();
  });

  it("mueve con Alt+flecha, guarda de forma atómica y mantiene el foco", async () => {
    const user = userEvent.setup();
    mocked.moveStory.mockImplementation((input) => {
      server = applyMove(server, input.id, input.toColumnId, input.toIndex);
      return Promise.resolve(onServer(input.id));
    });
    renderBoard();
    const login = await screen.findByRole("button", { name: /^#1 Login/ });
    login.focus();
    await user.keyboard("{Alt>}{ArrowRight}{/Alt}");

    expect(mocked.moveStory).toHaveBeenCalledWith({
      projectId: "p1",
      id: "s1",
      expectedRevision: 1,
      toColumnId: "doing",
      toIndex: 1,
    });
    expect(
      await screen.findByText(/#1 movida a «En curso», posición 2/),
    ).toBeInTheDocument();
    await waitFor(() => {
      expect(cardIn("En curso", 1)).toHaveFocus();
    });
    expect(queryCardIn("Por hacer", 1)).not.toBeInTheDocument();
    // La lista se recarga tras guardar.
    expect(mocked.listStories).toHaveBeenCalledTimes(2);

    // Botón visible alternativo: subir una posición.
    await user.click(
      screen.getByRole("button", { name: "Mover #1 una posición arriba" }),
    );
    expect(mocked.moveStory).toHaveBeenLastCalledWith(
      expect.objectContaining({ id: "s1", toColumnId: "doing", toIndex: 0 }),
    );
  });

  it("revierte el movimiento optimista si se supera el WIP", async () => {
    const user = userEvent.setup();
    let reject: (reason: unknown) => void = () => undefined;
    mocked.moveStory.mockImplementation(
      () =>
        new Promise((_, fail) => {
          reject = fail;
        }),
    );
    renderBoard();
    (await screen.findByRole("button", { name: /^#2 API/ })).focus();
    await user.keyboard("{Alt>}{ArrowRight}{/Alt}");
    // Vista optimista: ya está en la columna destino mientras se guarda.
    expect(cardIn("En curso", 2)).toBeInTheDocument();

    reject({
      code: "wip_limit_exceeded",
      message: "La columna «En curso» alcanzó su límite WIP (2).",
      recoverable: true,
    });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "No se movió #2: La columna «En curso» alcanzó su límite WIP (2).",
    );
    expect(cardIn("Por hacer", 2)).toBeInTheDocument();
    expect(queryCardIn("En curso", 2)).not.toBeInTheDocument();
    // El foco vuelve a la tarjeta en su sitio original.
    await waitFor(() => {
      expect(cardIn("Por hacer", 2)).toHaveFocus();
    });
  });

  it("ante un conflicto revierte y recarga el tablero", async () => {
    const user = userEvent.setup();
    mocked.moveStory.mockRejectedValue({
      code: "conflict",
      message: "Otra edición cambió estos datos.",
      recoverable: true,
    });
    renderBoard();
    (await screen.findByRole("button", { name: /^#1 Login/ })).focus();
    await user.keyboard("{Alt>}{ArrowDown}{/Alt}");
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Se recargó el tablero",
    );
    expect(mocked.listStories).toHaveBeenCalledTimes(2);
    expect(mocked.listColumns).toHaveBeenCalledTimes(2);
    const todo = within(screen.getByRole("group", { name: "Por hacer" }));
    expect(
      todo.getAllByRole("button", { name: /^#\d/ }).map((b) => b.textContent),
    ).toEqual(["#1 Login", "#2 API"]);
  });

  it("arrastra y suelta antes de otra tarjeta", async () => {
    mocked.moveStory.mockImplementation((input) => {
      server = applyMove(server, input.id, input.toColumnId, input.toIndex);
      return Promise.resolve(onServer(input.id));
    });
    renderBoard();
    const api = (
      await screen.findByRole("button", { name: /^#2 API/ })
    ).closest("li");
    const pantalla = cardIn("En curso", 3).closest("li");
    if (!api || !pantalla) throw new Error("sin tarjetas");
    const dataTransfer = { setData: vi.fn(), effectAllowed: "" };
    fireEvent.dragStart(api, { dataTransfer });
    fireEvent.dragOver(pantalla, { dataTransfer });
    expect(pantalla).toHaveClass("card--drop-before");
    fireEvent.drop(pantalla, { dataTransfer });
    expect(mocked.moveStory).toHaveBeenCalledWith(
      expect.objectContaining({ id: "s2", toColumnId: "doing", toIndex: 0 }),
    );
    await waitFor(() => {
      expect(cardIn("En curso", 2)).toBeInTheDocument();
    });
  });

  it("soltar una tarjeta sobre sí misma no la mueve", async () => {
    renderBoard();
    const login = (
      await screen.findByRole("button", { name: /^#1 Login/ })
    ).closest("li");
    if (!login) throw new Error("sin tarjeta");
    const dataTransfer = { setData: vi.fn(), effectAllowed: "" };
    fireEvent.dragStart(login, { dataTransfer });
    fireEvent.dragOver(login, { dataTransfer });
    fireEvent.drop(login, { dataTransfer });
    expect(mocked.moveStory).not.toHaveBeenCalled();
    expect(cardIn("Por hacer", 1)).toBeInTheDocument();
  });

  it("soltar en «hecho» abre la aceptación; cancelar deja la tarjeta", async () => {
    const user = userEvent.setup();
    const detail = {
      id: "s3",
      projectId: "p1",
      number: 3,
      columnId: "doing",
      position: 0,
      title: "Pantalla",
      description: "",
      priority: "medium",
      archivedAt: null,
      acceptedAt: null,
      criteria: [{ id: "c1", number: 1, text: "Se ve bien" }],
      revision: 4,
      createdAt: "2026-11-01T00:00:00Z",
      updatedAt: "2026-11-01T00:00:00Z",
    } as StoryDetail;
    mocked.getStory.mockResolvedValue(detail);
    renderBoard();
    (await screen.findByRole("button", { name: /^#3 Pantalla/ })).focus();
    await user.keyboard("{Alt>}{ArrowRight}{/Alt}");

    const dialog = await screen.findByRole("dialog", {
      name: "Aceptar #3 Pantalla",
    }); // El foco queda dentro del diálogo modal, no en la tarjeta de detrás.
    await waitFor(() => {
      expect(dialog.contains(document.activeElement)).toBe(true);
    });

    expect(mocked.moveStory).not.toHaveBeenCalled();
    expect(cardIn("En curso", 3)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Cancelar" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(cardIn("En curso", 3)).toHaveFocus();

    // Ahora sí: marcar el criterio y aceptar.
    mocked.acceptStory.mockImplementation(() => {
      server = server.map((s) =>
        s.id === "s3"
          ? {
              ...s,
              columnId: "done",
              position: 1,
              acceptedAt: "2026-11-10T00:00:00Z",
              revision: 5,
            }
          : s,
      );
      return Promise.resolve(onServer("s3"));
    });
    await user.keyboard("{Alt>}{ArrowRight}{/Alt}");
    const again = await screen.findByRole("dialog", {
      name: "Aceptar #3 Pantalla",
    });
    await user.click(within(again).getByRole("checkbox", { name: /CA-1/ }));
    await user.click(
      within(again).getByRole("button", { name: "Aceptar historia" }),
    );
    expect(mocked.acceptStory).toHaveBeenCalledWith({
      projectId: "p1",
      storyId: "s3",
      expectedRevision: 4,
      satisfiedCriteria: ["c1"],
      columnId: "done",
      toIndex: 1,
    });
    expect(
      await screen.findByText(/#3 aceptada y movida a «Hecho»/),
    ).toBeInTheDocument();
    // Ya está al final de «Hecho»: no hace falta otro movimiento.
    expect(mocked.moveStory).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(cardIn("Hecho", 3)).toHaveFocus();
    });
  });

  it("dibuja 300 tarjetas (medición, sin SLA)", async () => {
    server = Array.from({ length: 300 }, (_, i) =>
      card(
        `s${String(i + 1)}`,
        ["todo", "doing", "done"][i % 3] ?? "todo",
        Math.floor(i / 3),
        {
          title: `Carga ${String(i + 1)}`,
          acceptedAt: i % 3 === 2 ? "2026-11-01T00:00:00Z" : null,
        },
      ),
    );
    const started = performance.now();
    renderBoard();
    await screen.findByRole("button", { name: /^#300 Carga 300/ });
    const elapsed = performance.now() - started;
    expect(screen.getAllByRole("listitem")).toHaveLength(300);
    // Medición informativa (jsdom, no navegador real).
    console.info(
      `[medición] tablero con 300 tarjetas: ${elapsed.toFixed(0)} ms en jsdom`,
    );
  });
});
