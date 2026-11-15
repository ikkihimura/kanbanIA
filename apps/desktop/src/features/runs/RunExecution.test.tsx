// HU-018: iniciar con el proceso de prueba, estados, cancelación solicitada, worktree, salida,
// avisos `run-change` y configuración de ejecuciones.
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ErrorPayload } from "../../contracts/generated/ErrorPayload";
import type { RunChange } from "../../contracts/generated/RunChange";
import type { RunDetail } from "../../contracts/generated/RunDetail";
import type { RunOutputView } from "../../contracts/generated/RunOutputView";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import type { RunWorktree } from "../../contracts/generated/RunWorktree";
import type { SnapshotRepository } from "../../contracts/generated/SnapshotRepository";
import type { StoryRelations } from "../../contracts/generated/StoryRelations";
import * as commands from "../../shared/ipc/commands";
import { OUTPUT_POLL_MS, RunOutputPanel } from "./RunOutputPanel";
import { RunSettingsPanel } from "./RunSettingsPanel";
import { useUnsavedChangesGuard } from "../../shared/ui/useUnsavedChangesGuard";
import { RunsPanel } from "./RunsPanel";
import { RUN_CHANGE_BATCH_MS } from "./useRunChanges";

// Solo se simulan las llamadas IPC; errores y helpers son los reales.
vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  listStoryRuns: vi.fn(),
  createRun: vi.fn(),
  cancelRun: vi.fn(),
  getRun: vi.fn(),
  getStoryRelations: vi.fn(),
  listAgents: vi.fn(),
  startRun: vi.fn(),
  getRunOutput: vi.fn(),
  getWorktreeChanges: vi.fn(),
  removeRunWorktree: vi.fn(),
  getRunSettings: vi.fn(),
  updateRunSettings: vi.fn(),
  onRunChange: vi.fn(),
  setUnsavedChanges: vi.fn(),
}));

const mocked = vi.mocked(commands);

// --- aviso `run-change` simulado ---

type Handler = (change: RunChange) => void;
const runHandlers = new Set<Handler>();

async function sleep(ms: number): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

/** Entrega los avisos a los suscriptores y espera a que cierre la ventana de agrupación. */
async function emitRunChange(...changes: RunChange[]): Promise<void> {
  await waitFor(() => {
    expect(runHandlers.size).toBeGreaterThan(0);
  });
  for (const change of changes) {
    act(() => {
      for (const handler of runHandlers) handler(change);
    });
  }
  await sleep(RUN_CHANGE_BATCH_MS + 20);
}

// --- datos ---

const NO_USAGE = { inputTokens: null, outputTokens: null, costUsdMicros: null };
const HEAD_A = "0123456789abcdef0123456789abcdef01234567";
const HEAD_B = "fedcba9876543210fedcba9876543210fedcba98";
const BRANCH = "kanbania/h7-i1-0a1b2c3d";

function summary(overrides: Partial<RunSummary> = {}): RunSummary {
  return {
    id: "r1",
    projectId: "p1",
    storyId: "s1",
    number: 1,
    state: "queued",
    agentId: "a1",
    agentNumber: 3,
    agentName: "Jean Claude",
    createdAt: "2026-11-12T10:00:00Z",
    updatedAt: "2026-11-12T10:00:00Z",
    endedAt: null,
    endReason: null,
    usage: NO_USAGE,
    lastSequence: 1,
    snapshotSha256: "ab".repeat(32),
    startRequestedAt: null,
    cancelRequestedAt: null,
    ...overrides,
  };
}

const ONE_REPO: SnapshotRepository[] = [
  { id: "g1", path: "/repos/app", head: HEAD_A, branch: "main" },
  { id: "g2", path: "/repos/sin-git", head: null, branch: null },
];

function detail(
  run: RunSummary = summary(),
  overrides: Partial<RunDetail> = {},
  repositories: SnapshotRepository[] = ONE_REPO,
): RunDetail {
  return {
    run,
    snapshot: {
      schemaVersion: 1,
      createdAt: "2026-11-12T10:00:00Z",
      story: {
        id: "s1",
        number: 7,
        title: "Título congelado",
        description: "Descripción",
        priority: "high",
        revision: 4,
        column: { name: "Listo", category: "ready" },
      },
      criteria: [],
      epic: null,
      parent: null,
      prerequisites: [],
      agent: {
        id: "a1",
        number: 3,
        name: "Jean Claude",
        role: "Implementa",
        provider: "Anthropic",
        model: "claude",
        capabilities: [],
      },
      repositories,
      instructions: "# Instrucciones",
      note: null,
    },
    snapshotIntact: true,
    events: [],
    eventsTruncated: false,
    startRequest: null,
    cancelReason: null,
    worktree: null,
    ...overrides,
  };
}

function worktree(overrides: Partial<RunWorktree> = {}): RunWorktree {
  return {
    runId: "r1",
    repositoryId: "g1",
    path: "/Users/ana/Library/Application Support/dev.ikkidev.kanbania/worktrees/p1/r1",
    branch: BRANCH,
    baseCommit: HEAD_A,
    createdAt: "2026-11-12T10:01:00Z",
    removedAt: null,
    removal: null,
    ...overrides,
  };
}

const finished = summary({
  state: "completed",
  endedAt: "2026-11-12T10:05:00Z",
  endReason: "Terminó con código 0.",
});

const relations: StoryRelations = {
  storyId: "s1",
  epic: null,
  agent: null,
  parent: null,
  children: [],
  prerequisites: [],
  dependents: [],
  pendingPrerequisites: 0,
};

function validation(field: string, message: string): ErrorPayload {
  return { code: "validation", message, recoverable: true, field };
}

function renderPanel() {
  return render(
    <RunsPanel
      projectId="p1"
      storyId="s1"
      closed={null}
      onDirtyChange={vi.fn()}
    />,
  );
}

async function openDetail(user: ReturnType<typeof userEvent.setup>) {
  await user.click(
    await screen.findByRole("button", { name: "Ver detalle del intento #1" }),
  );
  return screen.findByRole("region", { name: "Detalle del intento #1" });
}

beforeEach(() => {
  vi.clearAllMocks();
  runHandlers.clear();
  mocked.onRunChange.mockImplementation((handler) => {
    runHandlers.add(handler);
    return Promise.resolve(() => {
      runHandlers.delete(handler);
    });
  });
  mocked.listStoryRuns.mockResolvedValue([summary()]);
  mocked.getStoryRelations.mockResolvedValue(relations);
  mocked.listAgents.mockResolvedValue([]);
  mocked.getRun.mockResolvedValue(detail());
  mocked.getRunOutput.mockResolvedValue({
    lines: [],
    skippedCorrupt: 0,
    earlierDropped: false,
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("Iniciar (proceso de prueba)", () => {
  it("con un solo repositorio con commit, inicia el escenario elegido sin pedir repositorio", async () => {
    const user = userEvent.setup();
    const requested = summary({ startRequestedAt: "2026-11-12T10:02:00Z" });
    mocked.listStoryRuns
      .mockResolvedValueOnce([summary()])
      .mockResolvedValue([requested]);
    mocked.getRun.mockResolvedValueOnce(detail()).mockResolvedValue(
      detail(requested, {
        startRequest: {
          repositoryId: "g1",
          scenario: "fail",
          requestedAt: "2026-11-12T10:02:00Z",
          dispatchedAt: null,
          voidedAt: null,
        },
      }),
    );
    mocked.startRun.mockResolvedValue(requested);
    renderPanel();

    // «Iniciar…» en la lista abre el detalle con el foco en el escenario.
    const item = within(
      await screen.findByRole("list", { name: /^Intentos/ }),
    ).getByRole("listitem");
    expect(item).toHaveTextContent("Proceso de prueba (sin IA)");
    await user.click(
      within(item).getByRole("button", { name: "Iniciar… el intento #1" }),
    );
    const scenario = await screen.findByRole("group", {
      name: "Escenario del proceso de prueba",
    });
    // Opciones siempre visibles; el foco va a la elegida («Termina bien»).
    expect(
      within(scenario).getByRole("radio", { name: "Termina bien" }),
    ).toHaveFocus();
    expect(
      within(scenario).getByRole("radio", { name: "Termina bien" }),
    ).toBeChecked();

    const form = screen.getByRole("form", { name: "Iniciar el intento #1" });
    expect(form).toHaveTextContent("No trabaja ningún agente ni IA");
    const submit = within(form).getByRole("button", { name: "Iniciar" });
    // La etiqueta del ejecutor está junto al botón y lo describe.
    expect(submit).toHaveAccessibleDescription("Proceso de prueba (sin IA)");
    expect(
      within(scenario)
        .getAllByRole("radio")
        .map((o) => o.closest("label")?.textContent),
    ).toEqual([
      "Termina bien",
      "Falla (termina con error)",
      "Tarda y crea procesos hijos (unos 10 min)",
      "Modifica archivos del worktree",
      "Produce mucha salida",
      "Se queda colgado (hasta el tiempo máximo)",
    ]);
    expect(form).toHaveTextContent(`/repos/app (commit ${HEAD_A.slice(0, 8)})`);
    expect(
      within(form).queryByLabelText("Repositorio del worktree"),
    ).not.toBeInTheDocument();

    await user.click(
      within(scenario).getByRole("radio", {
        name: "Falla (termina con error)",
      }),
    );
    await user.click(submit);

    expect(mocked.startRun).toHaveBeenCalledWith({
      requestId: expect.any(String) as string,
      projectId: "p1",
      runId: "r1",
      scenario: "fail",
    });
    expect(
      await screen.findByText(
        /Intento #1: inicio pedido con el proceso de prueba \(sin IA\)/,
      ),
    ).toBeInTheDocument();
    const region = screen.getByRole("region", {
      name: "Detalle del intento #1",
    });
    await within(region).findByText(/En cola · esperando turno: arrancará/);
    expect(
      screen.queryByRole("form", { name: "Iniciar el intento #1" }),
    ).not.toBeInTheDocument();
    expect(region).toHaveTextContent("Falla (termina con error)");
    expect(
      screen.getByRole("heading", { name: "Detalle del intento #1" }),
    ).toHaveFocus();
    // La lista muestra el estado en texto y ya no ofrece iniciar.
    expect(
      within(screen.getByRole("list", { name: /^Intentos/ })).getByText(
        "En cola · esperando turno",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /^Iniciar…/ }),
    ).not.toBeInTheDocument();
  });

  it("con varios repositorios con commit exige elegir uno y lo envía", async () => {
    const user = userEvent.setup();
    mocked.getRun.mockResolvedValue(
      detail(summary(), {}, [
        ...ONE_REPO,
        { id: "g3", path: "/repos/api", head: HEAD_B, branch: null },
      ]),
    );
    mocked.startRun.mockResolvedValue(
      summary({ startRequestedAt: "2026-11-12T10:02:00Z" }),
    );
    renderPanel();

    await openDetail(user);
    const repo = await screen.findByLabelText("Repositorio del worktree");
    // Solo los que tienen commit.
    expect(
      within(repo)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual([
      "Elige un repositorio…",
      `/repos/app (${HEAD_A.slice(0, 8)})`,
      `/repos/api (${HEAD_B.slice(0, 8)})`,
    ]);

    await user.click(screen.getByRole("button", { name: "Iniciar" }));
    const error = await screen.findByText(
      "Elige el repositorio donde crear el worktree.",
    );
    expect(repo).toHaveAttribute("aria-invalid", "true");
    expect(repo.getAttribute("aria-describedby")).toBe(error.id);
    expect(mocked.startRun).not.toHaveBeenCalled();

    await user.selectOptions(repo, "g3");
    expect(
      screen.queryByText("Elige el repositorio donde crear el worktree."),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Iniciar" }));
    expect(mocked.startRun).toHaveBeenCalledWith(
      expect.objectContaining({ repositoryId: "g3", scenario: "success" }),
    );
  });

  it("sin repositorio con commit, el error del núcleo aparece junto al repositorio y repite el requestId", async () => {
    const user = userEvent.setup();
    mocked.getRun.mockResolvedValue(
      detail(summary(), {}, [
        { id: "g2", path: "/repos/sin-git", head: null, branch: null },
      ]),
    );
    const message =
      "La instantánea del intento no tiene ningún repositorio con commit: vincula un repositorio Git con al menos un commit al proyecto y prepara un intento nuevo.";
    mocked.startRun.mockRejectedValue(validation("repositoryId", message));
    renderPanel();

    const region = await openDetail(user);
    expect(region).toHaveTextContent(
      "Ningún repositorio de la instantánea tiene commit: vincula al proyecto un repositorio con commits y prepara otro intento.",
    );
    const submit = await within(region).findByRole("button", {
      name: "Iniciar",
    });
    await user.click(submit);

    const error = await within(region).findByText(message);
    expect(error).toHaveAttribute("role", "alert");
    expect(submit.getAttribute("aria-describedby")).toContain(error.id);
    expect(region).not.toHaveTextContent("No se pudo iniciar el intento");

    await user.click(submit);
    const [first, second] = mocked.startRun.mock.calls.map(([i]) => i);
    expect(second?.requestId).toBe(first?.requestId);
  });

  it("si la ejecución no está disponible, el aviso del núcleo describe el botón Iniciar", async () => {
    const user = userEvent.setup();
    const message =
      "No se pueden ejecutar intentos ahora: cierra Kanban IA y vuelve a abrirlo.";
    mocked.startRun.mockRejectedValue(validation("runId", message));
    renderPanel();

    const region = await openDetail(user);
    const submit = await within(region).findByRole("button", {
      name: "Iniciar",
    });
    await user.click(submit);

    const form = within(region).getByRole("form", {
      name: "Iniciar el intento #1",
    });
    expect(await within(form).findByRole("alert")).toHaveTextContent(
      `No se pudo iniciar el intento: ${message}`,
    );
    expect(submit).toHaveAccessibleDescription(
      new RegExp(
        `Proceso de prueba \\(sin IA\\) .*cierra Kanban IA y vuelve a abrirlo`,
      ),
    );
    // El intento se vuelve a leer, pero el aviso sigue junto al botón.
    await waitFor(() => {
      expect(mocked.getRun).toHaveBeenCalledTimes(2);
    });
    expect(within(form).getByRole("alert")).toHaveTextContent(message);
  });

  it("un rechazo de la historia (bloqueada…) es una alerta general", async () => {
    const user = userEvent.setup();
    mocked.startRun.mockRejectedValue(
      validation("storyId", "La historia #7 está bloqueada: quita el bloqueo."),
    );
    renderPanel();

    const region = await openDetail(user);
    await user.click(
      await within(region).findByRole("button", { name: "Iniciar" }),
    );
    expect(await within(region).findByRole("alert")).toHaveTextContent(
      "No se pudo iniciar el intento: La historia #7 está bloqueada",
    );
  });
});

describe("Estados del intento", () => {
  it("cada estado se muestra con texto, incluido esperando turno y cancelación solicitada", async () => {
    mocked.listStoryRuns.mockResolvedValue([
      summary({
        id: "r7",
        number: 7,
        state: "running",
        cancelRequestedAt: "2026-11-12T10:10:00Z",
      }),
      summary({ id: "r6", number: 6, state: "running" }),
      summary({ id: "r5", number: 5, state: "preparing" }),
      summary({
        id: "r4",
        number: 4,
        startRequestedAt: "2026-11-12T10:02:00Z",
      }),
      summary({
        id: "r3",
        number: 3,
        state: "failed",
        endedAt: "2026-11-12T10:40:00Z",
        endReason: "Superó el tiempo máximo (30 min).",
      }),
      summary({
        id: "r2",
        number: 2,
        state: "interrupted",
        endedAt: "2026-11-12T10:40:00Z",
        endReason:
          "La app se cerró durante la ejecución: no se puede verificar el proceso.",
      }),
      finished,
    ]);
    renderPanel();

    const items = within(
      await screen.findByRole("list", { name: /^Intentos/ }),
    ).getAllByRole("listitem");
    const badges = items.map(
      (item) => item.querySelector(".badge")?.textContent,
    );
    expect(badges).toEqual([
      "Cancelación solicitada",
      "En ejecución",
      "Preparando",
      "En cola · esperando turno",
      "Fallido",
      "Interrumpido",
      "Completado",
    ]);
    const [cancelling, running, , waiting, failed, interrupted] = items as [
      HTMLElement,
      HTMLElement,
      HTMLElement,
      HTMLElement,
      HTMLElement,
      HTMLElement,
    ];
    // Con la cancelación pedida no se vuelve a ofrecer cancelar.
    expect(
      within(cancelling).queryByRole("button", { name: /Cancelar/ }),
    ).not.toBeInTheDocument();
    expect(cancelling).toHaveTextContent("se está deteniendo el proceso");
    expect(
      within(running).getByRole("button", { name: "Cancelar intento…" }),
    ).toBeInTheDocument();
    expect(waiting).toHaveTextContent("Esperando turno");
    expect(
      within(waiting).queryByRole("button", { name: /^Iniciar/ }),
    ).not.toBeInTheDocument();
    expect(failed).toHaveTextContent("Superó el tiempo máximo (30 min).");
    expect(interrupted).toHaveTextContent(
      "La app se cerró durante la ejecución",
    );
  });

  it("completado no acepta la historia: el detalle lo dice", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns.mockResolvedValue([finished]);
    mocked.getRun.mockResolvedValue(detail(finished));
    renderPanel();

    const region = await openDetail(user);
    expect(region).toHaveTextContent(
      "El proceso terminó bien. No cambia la historia ni su columna: aceptarla sigue siendo una decisión humana.",
    );
    expect(region).not.toHaveTextContent("pendiente de revisión");
    // El agente asignado no participa: lo ejecuta el proceso de prueba.
    expect(region).toHaveTextContent(
      "Agente asignado al intento: A-3 Jean Claude (no participa: lo ejecuta el proceso de prueba).",
    );
    expect(region).toHaveTextContent("Proceso de prueba (sin IA)");
    expect(region).toHaveTextContent("Ningún agente ni IA trabaja");
  });

  it("una petición anulada al reiniciar pide volver a iniciar", async () => {
    const user = userEvent.setup();
    mocked.getRun.mockResolvedValue(
      detail(summary(), {
        startRequest: {
          repositoryId: "g1",
          scenario: "slow",
          requestedAt: "2026-11-12T10:02:00Z",
          dispatchedAt: null,
          voidedAt: "2026-11-12T11:00:00Z",
        },
      }),
    );
    renderPanel();

    const region = await openDetail(user);
    expect(region).toHaveTextContent(
      /La petición de inicio se anuló al reiniciar la ejecución \(al abrir la app, tras restaurar una copia o al reintentar el almacenamiento; .*\): nada arranca solo\. Pulsa «Iniciar» de nuevo/,
    );
    expect(
      within(region).getByRole("button", { name: "Iniciar" }),
    ).toBeInTheDocument();
  });

  it("en «Preparando», la cancelación no promete segundos", async () => {
    const user = userEvent.setup();
    const preparing = summary({ state: "preparing" });
    mocked.listStoryRuns.mockResolvedValue([preparing]);
    mocked.cancelRun.mockResolvedValue(
      summary({
        state: "preparing",
        cancelRequestedAt: "2026-11-12T10:10:00Z",
      }),
    );
    renderPanel();

    await user.click(
      await screen.findByRole("button", { name: "Cancelar intento…" }),
    );
    await user.click(
      within(screen.getByRole("alertdialog")).getByRole("button", {
        name: "Cancelar intento",
      }),
    );
    const notice = await screen.findByText(
      /Cancelación solicitada para el intento #1: se detendrá cuando termine el paso de preparación en curso/,
    );
    expect(notice).not.toHaveTextContent("segundos");
  });

  it("si el intento termina con la confirmación de cancelar abierta, se cierra y el foco va al título", async () => {
    const user = userEvent.setup();
    const running = summary({ state: "running" });
    mocked.listStoryRuns
      .mockResolvedValueOnce([running])
      .mockResolvedValue([finished]);
    renderPanel();

    await user.click(
      await screen.findByRole("button", { name: "Cancelar intento…" }),
    );
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();

    await emitRunChange({ runIds: ["r1"], storyIds: ["s1"] });

    await waitFor(() => {
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    });
    expect(screen.getByRole("heading", { name: "Intentos" })).toHaveFocus();
    expect(
      screen.getByText(/El intento #1 ya terminó: no hace falta cancelarlo\./),
    ).toBeInTheDocument();
    expect(mocked.cancelRun).not.toHaveBeenCalled();
  });

  it("cancelar un intento en marcha solo lo solicita y se muestra hasta que termina", async () => {
    const user = userEvent.setup();
    const running = summary({ state: "running" });
    const requested = summary({
      state: "running",
      cancelRequestedAt: "2026-11-12T10:10:00Z",
    });
    mocked.listStoryRuns
      .mockResolvedValueOnce([running])
      .mockResolvedValue([requested]);
    mocked.getRun
      .mockResolvedValueOnce(detail(running))
      .mockResolvedValue(
        detail(requested, { cancelReason: "Cancelado por una persona." }),
      );
    mocked.cancelRun.mockResolvedValue(requested);
    renderPanel();

    const region = await openDetail(user);
    await user.click(screen.getByRole("button", { name: "Cancelar intento…" }));
    const dialog = screen.getByRole("alertdialog");
    expect(dialog).toHaveTextContent(
      "Se detendrán el proceso de prueba y sus procesos hijos",
    );
    await user.click(
      within(dialog).getByRole("button", { name: "Cancelar intento" }),
    );

    expect(
      await screen.findByText(
        /Cancelación solicitada para el intento #1: se detendrán el proceso y sus hijos en unos segundos\./,
      ),
    ).toBeInTheDocument();
    expect(
      await within(region).findByText(
        /se está deteniendo el proceso y sus procesos hijos…/,
      ),
    ).toBeInTheDocument();
    expect(region).toHaveTextContent("(Cancelado por una persona.)");
    expect(
      screen.queryByRole("button", { name: "Cancelar intento…" }),
    ).not.toBeInTheDocument();
    expect(
      within(screen.getByRole("list", { name: /^Intentos/ })).getByText(
        "Cancelación solicitada",
      ),
    ).toBeInTheDocument();
  });
});

describe("Worktree", () => {
  beforeEach(() => {
    mocked.listStoryRuns.mockResolvedValue([finished]);
    mocked.getRun.mockResolvedValue(detail(finished, { worktree: worktree() }));
  });

  it("muestra ruta, rama y commit base, y dice que la rama se conserva", async () => {
    const user = userEvent.setup();
    renderPanel();

    const region = await openDetail(user);
    const section = within(region).getByRole("region", { name: "Worktree" });
    expect(section).toHaveTextContent(worktree().path);
    expect(section).toHaveTextContent(BRANCH);
    expect(section).toHaveTextContent(HEAD_A);
    expect(section).toHaveTextContent("La rama se conserva siempre");
  });

  it("limpio: confirma «Eliminar worktree» sin forzar", async () => {
    const user = userEvent.setup();
    mocked.getWorktreeChanges.mockResolvedValue({
      missing: false,
      changes: 0,
      ignored: 0,
      commitsAtRisk: false,
      complete: true,
    });
    mocked.removeRunWorktree.mockResolvedValue(
      worktree({ removedAt: "2026-11-12T11:00:00Z", removal: "clean" }),
    );
    renderPanel();

    const region = await openDetail(user);
    await user.click(
      within(region).getByRole("button", { name: "Eliminar worktree…" }),
    );
    expect(mocked.getWorktreeChanges).toHaveBeenCalledWith("p1", "r1");
    const dialog = await within(region).findByRole("alertdialog");
    expect(dialog).toHaveTextContent("No tiene cambios sin confirmar");
    expect(dialog).toHaveTextContent(`La rama «${BRANCH}» se conserva.`);
    expect(dialog).not.toHaveTextContent("ignorados");
    expect(
      within(dialog).getByRole("button", { name: "Mantener" }),
    ).toHaveFocus();
    expect(
      within(dialog).queryByRole("button", { name: /Descartar/ }),
    ).not.toBeInTheDocument();

    await user.click(
      within(dialog).getByRole("button", { name: "Eliminar worktree" }),
    );
    expect(mocked.removeRunWorktree).toHaveBeenCalledWith({
      projectId: "p1",
      runId: "r1",
      force: false,
    });
    expect(
      await within(region).findByText(
        `✓ Worktree eliminado. La rama «${BRANCH}» se conserva.`,
      ),
    ).toBeInTheDocument();
    expect(
      within(region).getByRole("heading", { name: "Worktree" }),
    ).toHaveFocus();
    // El detalle se vuelve a leer del núcleo.
    await waitFor(() => {
      expect(mocked.getRun).toHaveBeenCalledTimes(2);
    });
  });

  it("con cambios exige la acción explícita «Descartar N cambios y eliminar» (force)", async () => {
    const user = userEvent.setup();
    mocked.getWorktreeChanges.mockResolvedValue({
      missing: false,
      changes: 3,
      ignored: 0,
      commitsAtRisk: false,
      complete: true,
    });
    mocked.removeRunWorktree.mockResolvedValue(
      worktree({ removedAt: "2026-11-12T11:00:00Z", removal: "discarded" }),
    );
    renderPanel();

    const region = await openDetail(user);
    await user.click(
      within(region).getByRole("button", { name: "Eliminar worktree…" }),
    );
    const dialog = await within(region).findByRole("alertdialog");
    expect(dialog).toHaveTextContent("3 cambios sin confirmar");
    expect(dialog).toHaveTextContent("los descarta para siempre");
    expect(dialog).toHaveTextContent(`La rama «${BRANCH}» se conserva`);
    // No hay un «Eliminar worktree» que descarte sin decirlo.
    expect(
      within(dialog).queryByRole("button", { name: "Eliminar worktree" }),
    ).not.toBeInTheDocument();

    await user.click(
      within(dialog).getByRole("button", {
        name: "Descartar 3 cambios y eliminar",
      }),
    );
    expect(mocked.removeRunWorktree).toHaveBeenCalledWith({
      projectId: "p1",
      runId: "r1",
      force: true,
    });
  });

  it("si aparecen cambios tras comprobar, muestra el error junto a la acción y el nuevo recuento", async () => {
    const user = userEvent.setup();
    mocked.getWorktreeChanges
      .mockResolvedValueOnce({
        missing: false,
        changes: 0,
        ignored: 0,
        commitsAtRisk: false,
        complete: true,
      })
      .mockResolvedValue({
        missing: false,
        changes: 1,
        ignored: 0,
        commitsAtRisk: false,
        complete: true,
      });
    mocked.removeRunWorktree.mockRejectedValue(
      validation("force", "El worktree tiene cambios sin confirmar."),
    );
    renderPanel();

    const region = await openDetail(user);
    await user.click(
      within(region).getByRole("button", { name: "Eliminar worktree…" }),
    );
    const dialog = await within(region).findByRole("alertdialog");
    await user.click(
      within(dialog).getByRole("button", { name: "Eliminar worktree" }),
    );

    const error = await within(dialog).findByText(
      "El worktree tiene cambios sin confirmar.",
    );
    expect(error).toHaveAttribute("role", "alert");
    const discard = await within(dialog).findByRole("button", {
      name: "Descartar 1 cambio y eliminar",
    });
    expect(discard.getAttribute("aria-describedby")).toBe(error.id);
  });

  it("si la carpeta ya no existía, lo dice", async () => {
    const user = userEvent.setup();
    mocked.getWorktreeChanges.mockResolvedValue({
      missing: true,
      changes: 0,
      ignored: 0,
      commitsAtRisk: false,
      complete: true,
    });
    mocked.removeRunWorktree.mockResolvedValue(
      worktree({ removedAt: "2026-11-12T11:00:00Z", removal: "missing" }),
    );
    mocked.getRun
      .mockResolvedValueOnce(detail(finished, { worktree: worktree() }))
      .mockResolvedValue(
        detail(finished, {
          worktree: worktree({
            removedAt: "2026-11-12T11:00:00Z",
            removal: "missing",
          }),
        }),
      );
    renderPanel();

    const region = await openDetail(user);
    await user.click(
      within(region).getByRole("button", { name: "Eliminar worktree…" }),
    );
    const dialog = await within(region).findByRole("alertdialog");
    expect(dialog).toHaveTextContent("La carpeta del worktree ya no existe");
    await user.click(
      within(dialog).getByRole("button", { name: "Eliminar worktree" }),
    );
    expect(
      await within(region).findByText(
        /La carpeta ya no existía: el worktree queda marcado como eliminado/,
      ),
    ).toBeInTheDocument();
    // Tras recargar: eliminado, con el motivo, y sin botón para eliminar otra vez.
    await within(region).findByText(/La carpeta ya no existía\.$/);
    expect(
      within(region).queryByRole("button", { name: "Eliminar worktree…" }),
    ).not.toBeInTheDocument();
  });

  it("solo archivos ignorados: exige la acción explícita de descartarlos (force)", async () => {
    const user = userEvent.setup();
    mocked.getWorktreeChanges.mockResolvedValue({
      missing: false,
      changes: 0,
      ignored: 4,
      commitsAtRisk: false,
      complete: true,
    });
    mocked.removeRunWorktree.mockResolvedValue(
      worktree({ removedAt: "2026-11-12T11:00:00Z", removal: "discarded" }),
    );
    renderPanel();

    const region = await openDetail(user);
    await user.click(
      within(region).getByRole("button", { name: "Eliminar worktree…" }),
    );
    const dialog = await within(region).findByRole("alertdialog");
    expect(dialog).toHaveAccessibleName(
      /El worktree tiene 4 archivos ignorados por Git \(p\. ej\. \.env o compilados\): eliminarlo los borra para siempre\./,
    );
    expect(dialog).not.toHaveTextContent("No tiene cambios");
    expect(
      within(dialog).queryByRole("button", { name: "Eliminar worktree" }),
    ).not.toBeInTheDocument();
    await user.click(
      within(dialog).getByRole("button", {
        name: "Descartar 4 archivos ignorados por Git y eliminar",
      }),
    );
    expect(mocked.removeRunWorktree).toHaveBeenCalledWith({
      projectId: "p1",
      runId: "r1",
      force: true,
    });
  });

  it("commits en riesgo: texto propio y acción explícita «Descartar y eliminar» (force)", async () => {
    const user = userEvent.setup();
    mocked.getWorktreeChanges.mockResolvedValue({
      missing: false,
      changes: 0,
      ignored: 1,
      commitsAtRisk: true,
      complete: true,
    });
    mocked.removeRunWorktree.mockResolvedValue(
      worktree({ removedAt: "2026-11-12T11:00:00Z", removal: "discarded" }),
    );
    renderPanel();

    const region = await openDetail(user);
    await user.click(
      within(region).getByRole("button", { name: "Eliminar worktree…" }),
    );
    const dialog = await within(region).findByRole("alertdialog");
    expect(dialog).toHaveTextContent(
      "Hay commits que no están en ninguna rama (o el worktree ya no está en la rama del intento): si lo eliminas se perderán.",
    );
    expect(dialog).toHaveTextContent(
      "El worktree tiene 1 archivo ignorado por Git",
    );
    // No promete que no se pierde nada ni habla de «N cambios».
    expect(dialog).toHaveTextContent(
      `La rama «${BRANCH}» se conserva, pero solo con los commits que ya estén en ella.`,
    );
    expect(dialog).not.toHaveTextContent("cambios sin confirmar");
    expect(dialog).not.toHaveTextContent("No tiene cambios");
    expect(
      within(dialog).queryByRole("button", { name: "Eliminar worktree" }),
    ).not.toBeInTheDocument();

    await user.click(
      within(dialog).getByRole("button", { name: "Descartar y eliminar" }),
    );
    expect(mocked.removeRunWorktree).toHaveBeenCalledWith({
      projectId: "p1",
      runId: "r1",
      force: true,
    });
  });

  it("si aparecen commits en riesgo tras comprobar, el error va junto a «Descartar y eliminar»", async () => {
    const user = userEvent.setup();
    mocked.getWorktreeChanges
      .mockResolvedValueOnce({
        missing: false,
        changes: 0,
        ignored: 0,
        commitsAtRisk: false,
        complete: true,
      })
      .mockResolvedValue({
        missing: false,
        changes: 0,
        ignored: 0,
        commitsAtRisk: true,
        complete: true,
      });
    mocked.removeRunWorktree.mockRejectedValue(
      validation("force", "El worktree tiene commits que se perderían."),
    );
    renderPanel();

    const region = await openDetail(user);
    await user.click(
      within(region).getByRole("button", { name: "Eliminar worktree…" }),
    );
    const dialog = await within(region).findByRole("alertdialog");
    await user.click(
      within(dialog).getByRole("button", { name: "Eliminar worktree" }),
    );
    const error = await within(dialog).findByText(
      "El worktree tiene commits que se perderían.",
    );
    const discard = await within(dialog).findByRole("button", {
      name: "Descartar y eliminar",
    });
    expect(discard.getAttribute("aria-describedby")).toBe(error.id);
    expect(dialog).toHaveTextContent("si lo eliminas se perderán");
  });

  it("el texto fijo no promete conservar commits fuera de la rama", async () => {
    const user = userEvent.setup();
    renderPanel();
    const region = await openDetail(user);
    const section = within(region).getByRole("region", { name: "Worktree" });
    expect(section).toHaveTextContent(
      "La rama se conserva siempre al eliminar el worktree: los commits que estén en ella no se pierden.",
    );
    expect(section).not.toHaveTextContent("sus commits no se pierden");
  });

  it("Escape cierra la confirmación sin cerrar el detalle y el foco vuelve al disparador", async () => {
    const user = userEvent.setup();
    mocked.getWorktreeChanges.mockResolvedValue({
      missing: false,
      changes: 2,
      ignored: 0,
      commitsAtRisk: false,
      complete: false,
    });
    renderPanel();

    const region = await openDetail(user);
    const trigger = within(region).getByRole("button", {
      name: "Eliminar worktree…",
    });
    trigger.focus();
    await user.keyboard("{Enter}");
    const dialog = await within(region).findByRole("alertdialog");
    expect(dialog).toHaveTextContent("al menos 2 cambios sin confirmar");
    expect(
      within(dialog).getByRole("button", {
        name: "Descartar al menos 2 cambios y eliminar",
      }),
    ).toBeInTheDocument();

    await user.keyboard("{Escape}");
    expect(within(region).queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(
      within(region).getByRole("button", { name: "Eliminar worktree…" }),
    ).toHaveFocus();
    expect(
      screen.getByRole("region", { name: "Detalle del intento #1" }),
    ).toBeInTheDocument();
    expect(mocked.removeRunWorktree).not.toHaveBeenCalled();
  });

  it("en un intento en marcha no se puede eliminar", async () => {
    const user = userEvent.setup();
    const running = summary({ state: "running" });
    mocked.listStoryRuns.mockResolvedValue([running]);
    mocked.getRun.mockResolvedValue(detail(running, { worktree: worktree() }));
    renderPanel();

    const region = await openDetail(user);
    expect(region).toHaveTextContent(
      "Podrás eliminar el worktree cuando el intento termine.",
    );
    expect(
      within(region).queryByRole("button", { name: "Eliminar worktree…" }),
    ).not.toBeInTheDocument();
  });
});

describe("Salida del proceso", () => {
  const output: RunOutputView = {
    lines: [
      { atMs: 1, stream: "system", text: "Proceso iniciado", cut: false },
      {
        atMs: 2,
        stream: "stdout",
        text: '<script>alert("x")</script>',
        cut: false,
      },
      {
        atMs: 3,
        stream: "stderr",
        text: "**negrita** y [enlace](http://x)",
        cut: true,
      },
    ],
    skippedCorrupt: 2,
    earlierDropped: true,
  };

  it("se muestra como texto plano con el origen de cada línea y los avisos", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns.mockResolvedValue([finished]);
    mocked.getRun.mockResolvedValue(detail(finished));
    mocked.getRunOutput.mockResolvedValue(output);
    const { container } = renderPanel();

    const region = await openDetail(user);
    const lines = await within(region).findByRole("region", {
      name: "Líneas de salida",
    });
    expect(mocked.getRunOutput).toHaveBeenCalledWith("p1", "r1");
    expect(
      within(lines)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual([
      "[sistema] Proceso iniciado",
      '[salida] <script>alert("x")</script>',
      "[error] **negrita** y [enlace](http://x) [cortada]",
    ]);
    expect(container.querySelector("script")).toBeNull();
    expect(lines.querySelector("strong, a, em")).toBeNull();
    expect(lines).toHaveAttribute("tabindex", "0");
    expect(region).toHaveTextContent("La salida anterior se descartó");
    expect(region).toHaveTextContent("Se saltaron 2 líneas ilegibles.");
    expect(region).toHaveTextContent("se muestran cortadas");

    await user.click(
      within(region).getByRole("button", { name: "Actualizar la salida" }),
    );
    expect(mocked.getRunOutput).toHaveBeenCalledTimes(2);
  });

  it("en cola sin iniciar no hay sección de salida", async () => {
    const user = userEvent.setup();
    renderPanel();
    const region = await openDetail(user);
    expect(
      within(region).queryByRole("region", { name: "Salida del proceso" }),
    ).not.toBeInTheDocument();
    expect(mocked.getRunOutput).not.toHaveBeenCalled();
  });

  it("al cargar muestra el final sin robar el foco, y no salta si la persona subió", async () => {
    const heightSpy = vi
      .spyOn(HTMLElement.prototype, "scrollHeight", "get")
      .mockReturnValue(1000);
    const clientSpy = vi
      .spyOn(HTMLElement.prototype, "clientHeight", "get")
      .mockReturnValue(100);
    const lines = (n: number): RunOutputView => ({
      lines: Array.from({ length: n }, (_, i) => ({
        atMs: i,
        stream: "stdout" as const,
        text: `línea ${String(i)}`,
        cut: false,
      })),
      skippedCorrupt: 0,
      earlierDropped: false,
    });
    mocked.getRunOutput
      .mockResolvedValueOnce(lines(3))
      .mockResolvedValueOnce(lines(4))
      .mockResolvedValue(lines(5));
    const before = document.activeElement;
    const view = render(
      <RunOutputPanel
        projectId="p1"
        runId="r1"
        live={false}
        refreshSignal={0}
      />,
    );
    const region = await screen.findByRole("region", {
      name: "Líneas de salida",
    });
    expect(region.scrollTop).toBe(1000);
    expect(document.activeElement).toBe(before);

    // La persona sube para leer: una recarga no la mueve.
    region.scrollTop = 200;
    fireEvent.scroll(region);
    view.rerender(
      <RunOutputPanel
        projectId="p1"
        runId="r1"
        live={false}
        refreshSignal={1}
      />,
    );
    await screen.findByText("línea 3");
    expect(region.scrollTop).toBe(200);

    // De vuelta al final: la siguiente recarga la sigue.
    region.scrollTop = 900;
    fireEvent.scroll(region);
    view.rerender(
      <RunOutputPanel
        projectId="p1"
        runId="r1"
        live={false}
        refreshSignal={2}
      />,
    );
    await screen.findByText("línea 4");
    expect(region.scrollTop).toBe(1000);
    expect(document.activeElement).toBe(before);
    heightSpy.mockRestore();
    clientSpy.mockRestore();
  });

  it("mientras corre se refresca con un solo temporizador, que se detiene al parar", async () => {
    vi.useFakeTimers();
    const view = render(
      <RunOutputPanel projectId="p1" runId="r1" live refreshSignal={0} />,
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mocked.getRunOutput).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(OUTPUT_POLL_MS);
    });
    expect(mocked.getRunOutput).toHaveBeenCalledTimes(2);
    // Un aviso (cambio de señal) recarga una vez sin crear otro temporizador.
    view.rerender(
      <RunOutputPanel projectId="p1" runId="r1" live refreshSignal={1} />,
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mocked.getRunOutput).toHaveBeenCalledTimes(3);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(OUTPUT_POLL_MS);
    });
    expect(mocked.getRunOutput).toHaveBeenCalledTimes(4);

    view.rerender(
      <RunOutputPanel
        projectId="p1"
        runId="r1"
        live={false}
        refreshSignal={1}
      />,
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(OUTPUT_POLL_MS * 3);
    });
    expect(mocked.getRunOutput).toHaveBeenCalledTimes(4);
  });
});

describe("Avisos run-change", () => {
  it("un aviso del intento recarga la lista, el detalle y la salida (una vez por ráfaga)", async () => {
    const user = userEvent.setup();
    const running = summary({ state: "running" });
    mocked.listStoryRuns
      .mockResolvedValueOnce([running])
      .mockResolvedValue([finished]);
    mocked.getRun
      .mockResolvedValueOnce(detail(running))
      .mockResolvedValue(detail(finished));
    renderPanel();

    const region = await openDetail(user);
    await within(region).findByRole("region", { name: "Salida del proceso" });
    await waitFor(() => {
      expect(mocked.getRunOutput).toHaveBeenCalledTimes(1);
    });

    await emitRunChange(
      { runIds: ["r1"], storyIds: [] },
      { runIds: ["r1"], storyIds: ["s1"] },
    );

    expect(
      await within(region).findByText(/No cambia la historia ni su columna/),
    ).toBeInTheDocument();
    expect(mocked.listStoryRuns).toHaveBeenCalledTimes(2);
    expect(mocked.getRun).toHaveBeenCalledTimes(2);
    await waitFor(() => {
      expect(mocked.getRunOutput).toHaveBeenCalledTimes(2);
    });
  });

  it("ignora los avisos de otras historias e intentos", async () => {
    renderPanel();
    await screen.findByRole("list", { name: /^Intentos/ });

    await emitRunChange({ runIds: ["otro"], storyIds: ["s9"] });

    expect(mocked.listStoryRuns).toHaveBeenCalledTimes(1);
  });
});

describe("Sistema → Ejecuciones", () => {
  beforeEach(() => {
    mocked.getRunSettings.mockResolvedValue({
      maxConcurrency: 2,
      timeoutMinutes: 30,
      revision: 1,
    });
  });

  it("muestra los valores y la ayuda, valida los límites y guarda con expectedRevision", async () => {
    const user = userEvent.setup();
    mocked.updateRunSettings.mockResolvedValue({
      maxConcurrency: 3,
      timeoutMinutes: 45,
      revision: 2,
    });
    render(<RunSettingsPanel />);

    expect(
      screen.getByRole("heading", { name: "Ejecuciones" }),
    ).toBeInTheDocument();
    const concurrency = await screen.findByLabelText("Ejecuciones a la vez");
    const timeout = screen.getByLabelText(
      "Tiempo máximo por ejecución (minutos)",
    );
    expect(concurrency).toHaveValue(2);
    expect(timeout).toHaveValue(30);
    expect(concurrency).toHaveAccessibleDescription(
      /Bajar el límite no detiene los que ya corren\./,
    );

    await user.clear(concurrency);
    await user.type(concurrency, "9");
    await user.clear(timeout);
    await user.type(timeout, "0");
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    expect(mocked.updateRunSettings).not.toHaveBeenCalled();
    expect(concurrency).toHaveAttribute("aria-invalid", "true");
    expect(concurrency).toHaveAccessibleDescription(/entre 1 y 8/);
    expect(timeout).toHaveAttribute("aria-invalid", "true");
    expect(timeout).toHaveAccessibleDescription(/entre 1 y 240/);

    await user.clear(concurrency);
    await user.type(concurrency, "3");
    await user.clear(timeout);
    await user.type(timeout, "45");
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    expect(mocked.updateRunSettings).toHaveBeenCalledWith({
      expectedRevision: 1,
      maxConcurrency: 3,
      timeoutMinutes: 45,
    });
    expect(
      await screen.findByText("✓ Configuración guardada."),
    ).toBeInTheDocument();
    expect(concurrency).not.toHaveAttribute("aria-invalid");

    // El siguiente guardado usa la revisión nueva.
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    expect(mocked.updateRunSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({ expectedRevision: 2 }),
    );
  });

  it("ante un conflicto muestra la configuración vigente y reintenta sobre su revisión", async () => {
    const user = userEvent.setup();
    mocked.updateRunSettings
      .mockRejectedValueOnce({
        code: "conflict",
        message: "La configuración cambió.",
        recoverable: true,
        current: { maxConcurrency: 4, timeoutMinutes: 60, revision: 5 },
      } satisfies ErrorPayload)
      .mockResolvedValue({
        maxConcurrency: 1,
        timeoutMinutes: 30,
        revision: 6,
      });
    render(<RunSettingsPanel />);

    const concurrency = await screen.findByLabelText("Ejecuciones a la vez");
    await user.clear(concurrency);
    await user.type(concurrency, "1");
    await user.click(screen.getByRole("button", { name: "Guardar" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "La configuración cambió mientras la editabas. Ahora es: 4 a la vez y 60 min.",
    );
    // Lo escrito se conserva.
    expect(concurrency).toHaveValue(1);

    await user.click(screen.getByRole("button", { name: "Guardar" }));
    expect(mocked.updateRunSettings).toHaveBeenLastCalledWith({
      expectedRevision: 5,
      maxConcurrency: 1,
      timeoutMinutes: 30,
    });
    expect(
      await screen.findByText("✓ Configuración guardada."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("un error de campo del núcleo aparece junto al campo", async () => {
    const user = userEvent.setup();
    mocked.updateRunSettings.mockRejectedValue(
      validation(
        "timeoutMinutes",
        "El tiempo máximo debe estar entre 1 y 240.",
      ),
    );
    render(<RunSettingsPanel />);

    const timeout = await screen.findByLabelText(
      "Tiempo máximo por ejecución (minutos)",
    );
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    const error = await screen.findByText(
      "El tiempo máximo debe estar entre 1 y 240.",
    );
    expect(timeout.getAttribute("aria-describedby")).toContain(error.id);
    expect(screen.queryByText(/No se pudo guardar/)).not.toBeInTheDocument();
  });

  it("con valores sin guardar avisa al cerrar, sin borrar el borrador de otro editor", async () => {
    const user = userEvent.setup();
    mocked.updateRunSettings.mockResolvedValue({
      maxConcurrency: 3,
      timeoutMinutes: 30,
      revision: 2,
    });
    // Imita al editor de historias con un borrador abierto.
    function OtherEditor({ dirty }: { dirty: boolean }) {
      useUnsavedChangesGuard(dirty);
      return null;
    }
    const view = render(
      <>
        <OtherEditor dirty />
        <RunSettingsPanel />
      </>,
    );
    const concurrency = await screen.findByLabelText("Ejecuciones a la vez");
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);

    await user.clear(concurrency);
    await user.type(concurrency, "3");
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    await screen.findByText("✓ Configuración guardada.");
    // Guardar limpia el borrador de Sistema, pero el del otro editor sigue.
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);

    view.rerender(
      <>
        <OtherEditor dirty={false} />
        <RunSettingsPanel />
      </>,
    );
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(false);

    // Solo Sistema con borrador: también cuenta.
    await user.clear(concurrency);
    await user.type(concurrency, "5");
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);
    view.unmount();
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(false);
  });

  it("si no carga, lo dice y permite reintentar", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockRejectedValueOnce({
      code: "storage_unavailable",
      message: "El almacenamiento no está disponible.",
      recoverable: true,
    } satisfies ErrorPayload);
    render(<RunSettingsPanel />);

    expect(
      await screen.findByText(/No se pudo cargar la configuración/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reintentar" }));
    expect(await screen.findByLabelText("Ejecuciones a la vez")).toHaveValue(2);
  });
});
