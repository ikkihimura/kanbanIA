// HU-018/019/021: iniciar con el proceso de prueba, Claude Code o Codex (ACP), estados, cancelación
// solicitada, worktree, salida, sesión del agente, avisos `run-change` y configuración.
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
import type { RunSettings } from "../../contracts/generated/RunSettings";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import type { RunWorktree } from "../../contracts/generated/RunWorktree";
import type { SnapshotRepository } from "../../contracts/generated/SnapshotRepository";
import type { StoryRelations } from "../../contracts/generated/StoryRelations";
import * as commands from "../../shared/ipc/commands";
import { OUTPUT_POLL_MS, RunOutputPanel } from "./RunOutputPanel";
import {
  RunSettingsPanel,
  codexLoginCommand,
  codexLoginCommandPowerShell,
} from "./RunSettingsPanel";
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
  checkAcpAdapter: vi.fn(),
  getCodexHomeStatus: vi.fn(),
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
    approvals: [],
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

const CODEX_HOME =
  "/Users/ana/Library/Application Support/dev.ikkidev.kanbania/codex-home";

function settings(overrides: Partial<RunSettings> = {}): RunSettings {
  return {
    maxConcurrency: 2,
    timeoutMinutes: 30,
    claudeAcpPath: null,
    codexAcpPath: null,
    approvalNotifications: false,
    revision: 1,
    ...overrides,
  };
}

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
  mocked.getRunSettings.mockResolvedValue(settings());
  mocked.getCodexHomeStatus.mockResolvedValue({
    path: CODEX_HOME,
    signedIn: false,
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
          executor: { kind: "testProcess", scenario: "fail" },
          requestedAt: "2026-11-12T10:02:00Z",
          dispatchedAt: null,
          voidedAt: null,
        },
      }),
    );
    mocked.startRun.mockResolvedValue(requested);
    renderPanel();

    // «Iniciar…» en la lista abre el detalle con el foco en el ejecutor elegido.
    const item = within(
      await screen.findByRole("list", { name: /^Intentos/ }),
    ).getByRole("listitem");
    await user.click(
      within(item).getByRole("button", { name: "Iniciar… el intento #1" }),
    );
    const executors = await screen.findByRole("group", { name: "Ejecutor" });
    // Opciones siempre visibles; por defecto y con el foco, el proceso de prueba.
    expect(
      within(executors).getByRole("radio", {
        name: "Proceso de prueba (sin IA)",
      }),
    ).toHaveFocus();
    const scenario = screen.getByRole("group", {
      name: "Escenario del proceso de prueba",
    });
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
      executor: { kind: "testProcess", scenario: "fail" },
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
      expect.objectContaining({
        repositoryId: "g3",
        executor: { kind: "testProcess", scenario: "success" },
      }),
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
    mocked.getRun.mockResolvedValue(
      detail(finished, {
        startRequest: {
          repositoryId: "g1",
          executor: { kind: "testProcess", scenario: "success" },
          requestedAt: "2026-11-12T10:02:00Z",
          dispatchedAt: "2026-11-12T10:02:01Z",
          voidedAt: null,
        },
      }),
    );
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
          executor: { kind: "testProcess", scenario: "slow" },
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
      "Se detendrán el proceso y sus procesos hijos",
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
      {
        atMs: 1,
        stream: "system",
        text: "Proceso iniciado",
        cut: false,
        kind: null,
      },
      {
        atMs: 2,
        stream: "stdout",
        text: '<script>alert("x")</script>',
        cut: false,
        kind: null,
      },
      {
        atMs: 3,
        stream: "stderr",
        text: "**negrita** y [enlace](http://x)",
        cut: true,
        kind: null,
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
      "[app] Proceso iniciado",
      '[stdout] <script>alert("x")</script>',
      "[stderr] **negrita** y [enlace](http://x) [cortada]",
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
        kind: null,
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
    mocked.getRunSettings.mockResolvedValue(settings());
  });

  it("muestra los valores y la ayuda, valida los límites y guarda con expectedRevision", async () => {
    const user = userEvent.setup();
    mocked.updateRunSettings.mockResolvedValue(
      settings({ maxConcurrency: 3, timeoutMinutes: 45, revision: 2 }),
    );
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
      claudeAcpPath: null,
      codexAcpPath: null,
      approvalNotifications: false,
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
        current: settings({
          maxConcurrency: 4,
          timeoutMinutes: 60,
          revision: 5,
        }),
      } satisfies ErrorPayload)
      .mockResolvedValue(settings({ maxConcurrency: 1, revision: 6 }));
    render(<RunSettingsPanel />);

    const concurrency = await screen.findByLabelText("Ejecuciones a la vez");
    await user.clear(concurrency);
    await user.type(concurrency, "1");
    await user.click(screen.getByRole("button", { name: "Guardar" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "La configuración cambió mientras la editabas. Ahora es: 4 a la vez, 60 min, sin adaptador de Claude Code, sin adaptador de Codex y sin notificaciones de permisos.",
    );
    // Lo escrito se conserva.
    expect(concurrency).toHaveValue(1);

    await user.click(screen.getByRole("button", { name: "Guardar" }));
    expect(mocked.updateRunSettings).toHaveBeenLastCalledWith({
      expectedRevision: 5,
      maxConcurrency: 1,
      timeoutMinutes: 30,
      claudeAcpPath: null,
      codexAcpPath: null,
      approvalNotifications: false,
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
    mocked.updateRunSettings.mockResolvedValue(
      settings({ maxConcurrency: 3, revision: 2 }),
    );
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

// --- HU-019: Claude Code vía ACP ---

const ACP_PATH = "/Users/ana/bin/claude-agent-acp";

describe("Iniciar con Claude Code (ACP)", () => {
  it("sin adaptador configurado, la opción está deshabilitada y explica dónde configurarlo", async () => {
    const user = userEvent.setup();
    renderPanel();

    const region = await openDetail(user);
    const executors = await within(region).findByRole("group", {
      name: "Ejecutor",
    });
    const acp = within(executors).getByRole("radio", {
      name: "Claude Code (ACP)",
    });
    await waitFor(() => {
      expect(acp).toHaveAccessibleDescription(
        /No disponible: configura el adaptador de Claude Code en Sistema → Ejecuciones \(ruta de claude-agent-acp/,
      );
    });
    expect(acp).toBeDisabled();
    expect(
      within(executors).getByRole("radio", {
        name: "Proceso de prueba (sin IA)",
      }),
    ).toBeChecked();
    // El proceso de prueba sigue siendo elegible y muestra sus escenarios.
    expect(
      within(region).getByRole("group", {
        name: "Escenario del proceso de prueba",
      }),
    ).toBeInTheDocument();
  });

  it("si no se puede leer la configuración, lo dice y permite volver a leerla", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings
      .mockRejectedValueOnce({
        code: "storage_unavailable",
        message: "El almacenamiento no está disponible.",
        recoverable: true,
      } satisfies ErrorPayload)
      .mockResolvedValue(settings({ claudeAcpPath: ACP_PATH }));
    renderPanel();

    const region = await openDetail(user);
    const acp = await within(region).findByRole("radio", {
      name: "Claude Code (ACP)",
    });
    await waitFor(() => {
      expect(acp).toHaveAccessibleDescription(
        /Claude Code y Codex no disponibles: no se pudo leer la configuración \(El almacenamiento no está disponible\.\)/,
      );
    });
    await user.click(
      within(region).getByRole("button", {
        name: "Volver a leer la configuración",
      }),
    );
    await waitFor(() => {
      expect(acp).toBeEnabled();
    });
  });

  it("con adaptador, elegir ACP oculta los escenarios, explica qué hace y envía claudeAcp", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(
      settings({ claudeAcpPath: ACP_PATH }),
    );
    mocked.startRun.mockResolvedValue(
      summary({ startRequestedAt: "2026-11-12T10:02:00Z" }),
    );
    renderPanel();

    const region = await openDetail(user);
    const acp = await within(region).findByRole("radio", {
      name: "Claude Code (ACP)",
    });
    await waitFor(() => {
      expect(acp).toBeEnabled();
    });
    expect(acp).not.toHaveAccessibleDescription();
    await user.click(acp);

    const form = within(region).getByRole("form", {
      name: "Iniciar el intento #1",
    });
    expect(
      within(form).queryByRole("group", {
        name: "Escenario del proceso de prueba",
      }),
    ).not.toBeInTheDocument();
    expect(form).toHaveTextContent(
      "Lanza Claude Code en el worktree con las instrucciones de la instantánea. El agente trabaja con tu sesión de Claude Code. Cada petición de permiso (editar, ejecutar…) aparece en el detalle del intento para que la permitas una vez o la rechaces; sin respuesta en 10 minutos, se rechaza.",
    );
    expect(form).not.toHaveTextContent("No trabaja ningún agente ni IA");
    const submit = within(form).getByRole("button", { name: "Iniciar" });
    expect(submit).toHaveAccessibleDescription("Claude Code (ACP)");

    await user.click(submit);
    expect(mocked.startRun).toHaveBeenCalledWith({
      requestId: expect.any(String) as string,
      projectId: "p1",
      runId: "r1",
      executor: { kind: "claudeAcp" },
    });
    expect(
      await screen.findByText(
        "✓ Intento #1: inicio pedido con Claude Code (ACP). Espera turno en la cola.",
      ),
    ).toBeInTheDocument();
  });

  it("volver al proceso de prueba envía su escenario", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(
      settings({ claudeAcpPath: ACP_PATH }),
    );
    mocked.startRun.mockResolvedValue(
      summary({ startRequestedAt: "2026-11-12T10:02:00Z" }),
    );
    renderPanel();

    const region = await openDetail(user);
    const acp = await within(region).findByRole("radio", {
      name: "Claude Code (ACP)",
    });
    await waitFor(() => {
      expect(acp).toBeEnabled();
    });
    await user.click(acp);
    await user.click(
      within(region).getByRole("radio", { name: "Proceso de prueba (sin IA)" }),
    );
    await user.click(
      within(region).getByRole("radio", { name: "Produce mucha salida" }),
    );
    await user.click(within(region).getByRole("button", { name: "Iniciar" }));
    expect(mocked.startRun).toHaveBeenCalledWith(
      expect.objectContaining({
        executor: { kind: "testProcess", scenario: "flood" },
      }),
    );
  });

  it("un error del núcleo en el ejecutor aparece en el grupo", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(
      settings({ claudeAcpPath: ACP_PATH }),
    );
    const message =
      "No hay ningún adaptador de Claude Code configurado: indica su ruta en Sistema → Ejecuciones.";
    mocked.startRun.mockRejectedValue(validation("executor", message));
    renderPanel();

    const region = await openDetail(user);
    const acp = await within(region).findByRole("radio", {
      name: "Claude Code (ACP)",
    });
    await waitFor(() => {
      expect(acp).toBeEnabled();
    });
    await user.click(acp);
    await user.click(within(region).getByRole("button", { name: "Iniciar" }));

    const error = await within(region).findByText(message);
    expect(error).toHaveAttribute("role", "alert");
    const group = within(region).getByRole("group", { name: "Ejecutor" });
    expect(group).toContainElement(error);
    expect(group.getAttribute("aria-describedby")).toBe(error.id);
    expect(region).not.toHaveTextContent("No se pudo iniciar el intento");
  });
});

describe("Detalle con Claude Code (ACP)", () => {
  const ended = summary({
    state: "completed",
    endedAt: "2026-11-12T10:05:00Z",
    endReason: "El agente terminó su turno.",
  });
  const acpDetail = detail(ended, {
    startRequest: {
      repositoryId: "g1",
      executor: { kind: "claudeAcp" },
      requestedAt: "2026-11-12T10:02:00Z",
      dispatchedAt: "2026-11-12T10:02:01Z",
      voidedAt: null,
    },
    events: [
      {
        sequence: 1,
        at: "2026-11-12T10:00:00Z",
        event: { kind: "state_changed", to: "queued", reason: null },
      },
      {
        sequence: 2,
        at: "2026-11-12T10:02:05Z",
        event: {
          kind: "acp_session",
          protocolVersion: 1,
          agentName: "@agentclientprotocol/claude-agent-acp",
          agentTitle: "Claude <b>Agent</b>",
          agentVersion: "0.107.0",
          capabilities: [
            "loadSession",
            "image",
            "embeddedContext",
            "mcpHttp",
            "mcpSse",
            "modes",
          ],
          mode: "default",
          promptSha256:
            "0123456789abcccccccccccccccccccccccccccccccccccccccccccccccccccc",
          promptChars: 12345,
          model: "claude-opus-4-5",
        },
      },
    ],
  });

  it("muestra el ejecutor, la sesión reportada por el agente y no dice que no trabaje IA", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns.mockResolvedValue([ended]);
    mocked.getRun.mockResolvedValue(acpDetail);
    const { container } = renderPanel();

    const region = await openDetail(user);
    const execution = within(region).getByRole("region", { name: "Ejecución" });
    expect(execution).toHaveTextContent("EjecutorClaude Code (ACP)");
    expect(execution).not.toHaveTextContent("Escenario");
    expect(region).not.toHaveTextContent("Ningún agente ni IA trabaja");
    expect(region).toHaveTextContent(
      "Ejecutor: Claude Code por ACP. Lo que aparece como «agente (reportado)» es lo que el agente dice, no algo comprobado por la app. Un intento completado no acepta la historia.",
    );
    expect(region).toHaveTextContent(
      "(lo ejecuta Claude Code por ACP con las instrucciones de la instantánea).",
    );

    const session = within(region).getByRole("region", {
      name: "Sesión del agente (reportado por el agente)",
    });
    // El texto del agente se muestra literal, sin interpretar HTML.
    expect(session).toHaveTextContent("AgenteClaude <b>Agent</b> 0.107.0");
    expect(container.querySelector("b")).toBeNull();
    expect(session).toHaveTextContent(
      "Modelo (reportado por el agente)claude-opus-4-5",
    );
    expect(session).toHaveTextContent("ProtocoloACP v1");
    expect(session).toHaveTextContent(
      "Modo fijado por la appManual: pide permiso",
    );
    expect(session).toHaveTextContent(
      "Capacidadesretomar sesión, imágenes, contexto incrustado, MCP por HTTP, MCP por SSE, modos",
    );
    // Huella del texto enviado: abreviada a la vista, completa para lectores de pantalla.
    expect(session).toHaveTextContent(
      "Instrucciones enviadas por la app12.345 caracteres · SHA-256 0123456789ab… (huella completa: 0123456789abcccccccccccccccccccccccccccccccccccccccccccccccccccc)",
    );
    expect(
      within(session).getByTitle(
        "0123456789abcccccccccccccccccccccccccccccccccccccccccccccccccccc",
      ),
    ).toHaveTextContent("0123456789ab…");
    // Las instrucciones congeladas no se presentan como lo que recibió el agente.
    expect(region).toHaveTextContent(
      "Instrucciones congeladas en la instantánea. Con Claude Code (ACP) o Codex (ACP) se envían las mismas, salvo la sección de repositorios",
    );
    expect(region).not.toHaveTextContent("lo que recibiría el agente");

    const events = within(region).getByRole("list", {
      name: "Eventos en orden",
    });
    expect(events).toHaveTextContent(
      "Sesión del agente iniciada (reportado): Claude <b>Agent</b> 0.107.0 · ACP v1",
    );
  });

  it("el proceso de prueba muestra su escenario y no la sesión del agente", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns.mockResolvedValue([finished]);
    mocked.getRun.mockResolvedValue(
      detail(finished, {
        startRequest: {
          repositoryId: "g1",
          executor: { kind: "testProcess", scenario: "write" },
          requestedAt: "2026-11-12T10:02:00Z",
          dispatchedAt: "2026-11-12T10:02:01Z",
          voidedAt: null,
        },
      }),
    );
    renderPanel();

    const region = await openDetail(user);
    const execution = within(region).getByRole("region", { name: "Ejecución" });
    expect(execution).toHaveTextContent("EjecutorProceso de prueba (sin IA)");
    expect(execution).toHaveTextContent(
      "EscenarioModifica archivos del worktree",
    );
    expect(
      within(region).queryByRole("region", {
        name: "Sesión del agente (reportado por el agente)",
      }),
    ).not.toBeInTheDocument();
    expect(region).not.toHaveTextContent("Claude Code por ACP");
  });

  it("sin petición de inicio, el ejecutor aún no está elegido", async () => {
    const user = userEvent.setup();
    renderPanel();
    const region = await openDetail(user);
    expect(
      within(region).getByRole("region", { name: "Ejecución" }),
    ).toHaveTextContent("EjecutorSe elige al iniciar");
  });
});

describe("Salida con líneas del agente", () => {
  it("cada línea del agente indica «agente (reportado)» y su tipo, en texto plano", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns.mockResolvedValue([finished]);
    mocked.getRun.mockResolvedValue(detail(finished));
    const agent = (
      atMs: number,
      kind: NonNullable<RunOutputView["lines"][number]["kind"]>,
      text: string,
    ): RunOutputView["lines"][number] => ({
      atMs,
      stream: "agent",
      text,
      cut: false,
      kind,
    });
    mocked.getRunOutput.mockResolvedValue({
      lines: [
        agent(1, "message", "Hola <b>mundo</b>\n**sin** [Markdown](http://x)"),
        agent(2, "thought", "(pensando… 120 caracteres)"),
        agent(3, "tool", "Leer README.md · read · completed"),
        agent(4, "plan", "Leer el README · pending"),
        agent(5, "usage", "usado 1200 de 200000"),
        agent(
          6,
          "permission",
          "Permiso pedido para «Editar src/main.rs»: rechazado una vez por la política de la app (hasta HU-020 se rechazan todos).",
        ),
        agent(
          8,
          "permission",
          "Permiso pedido para «Ejecutar npm test»: respondido como cancelado (se canceló el intento).",
        ),
        {
          atMs: 7,
          stream: "stderr",
          text: "[session/create] ok",
          cut: false,
          kind: null,
        },
      ],
      skippedCorrupt: 0,
      earlierDropped: false,
    });
    const { container } = renderPanel();

    const region = await openDetail(user);
    const lines = await within(region).findByRole("region", {
      name: "Líneas de salida",
    });
    const items = within(lines).getAllByRole("listitem");
    expect(items.map((li) => li.textContent)).toEqual([
      "[agente (reportado) · mensaje] Hola <b>mundo</b>\n**sin** [Markdown](http://x)",
      "[agente (reportado) · pensamiento] (pensando… 120 caracteres)",
      "[agente (reportado) · herramienta] Leer README.md · read · completed",
      "[agente (reportado) · plan] Leer el README · pending",
      "[agente (reportado) · uso] usado 1200 de 200000",
      "[agente (reportado) · permiso] Permiso: Permiso pedido para «Editar src/main.rs»: rechazado una vez por la política de la app (hasta HU-020 se rechazan todos).",
      "[agente (reportado) · permiso] Permiso: Permiso pedido para «Ejecutar npm test»: respondido como cancelado (se canceló el intento).",
      "[stderr] [session/create] ok",
    ]);
    // Literal: ni HTML ni Markdown interpretados.
    expect(container.querySelector("b")).toBeNull();
    expect(lines.querySelector("a, em")).toBeNull();
    // Solo la línea de permiso lleva la marca destacada (además del texto).
    expect(lines.querySelectorAll("strong")).toHaveLength(2);
    expect(items[5]).toHaveClass("run-output__line--permission");
    expect(items[6]).toHaveClass("run-output__line--permission");
    expect(items[7]).not.toHaveClass("run-output__line--permission");
  });
});

describe("Sistema → Ejecuciones: adaptador de Claude Code", () => {
  it("muestra la ruta guardada y la ayuda, y la envía siempre al guardar (vacía = null)", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(
      settings({ claudeAcpPath: ACP_PATH }),
    );
    mocked.updateRunSettings
      .mockResolvedValueOnce(settings({ claudeAcpPath: null, revision: 2 }))
      .mockResolvedValue(
        settings({ claudeAcpPath: "/opt/claude-agent-acp", revision: 3 }),
      );
    render(<RunSettingsPanel />);

    const path = await screen.findByLabelText("Adaptador de Claude Code (ACP)");
    expect(path).toHaveValue(ACP_PATH);
    expect(path).toHaveAccessibleDescription(
      /La app no lo incluye ni lo instala\..*necesita node en el PATH.*desde Finder.*binario autónomo.*terminal.*puede consumir tu suscripción o tu saldo.*«Always ask».*permitas una vez o la rechaces.*no cargue tu configuración de Claude Code ni la del repositorio/,
    );

    // Cambiar otro campo también envía la ruta (si faltara, el núcleo la borraría).
    const concurrency = screen.getByLabelText("Ejecuciones a la vez");
    await user.clear(concurrency);
    await user.type(concurrency, "3");
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);
    await user.clear(path);
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    expect(mocked.updateRunSettings).toHaveBeenLastCalledWith({
      expectedRevision: 1,
      maxConcurrency: 3,
      timeoutMinutes: 30,
      claudeAcpPath: null,
      codexAcpPath: null,
      approvalNotifications: false,
    });
    await screen.findByText("✓ Configuración guardada.");

    await user.type(path, "  /opt/claude-agent-acp  ");
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    expect(mocked.updateRunSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({
        expectedRevision: 2,
        claudeAcpPath: "/opt/claude-agent-acp",
      }),
    );
    await waitFor(() => {
      expect(path).toHaveValue("/opt/claude-agent-acp");
    });
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(false);
  });

  it("un error del núcleo en la ruta aparece junto al campo", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(settings());
    const message = "La ruta del adaptador debe ser absoluta.";
    mocked.updateRunSettings.mockRejectedValue(
      validation("claudeAcpPath", message),
    );
    render(<RunSettingsPanel />);

    const path = await screen.findByLabelText("Adaptador de Claude Code (ACP)");
    await user.type(path, "bin/claude-agent-acp");
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    const error = await screen.findByText(message);
    expect(path).toHaveAttribute("aria-invalid", "true");
    expect(path.getAttribute("aria-describedby")).toContain(error.id);
    expect(screen.queryByText(/No se pudo guardar/)).not.toBeInTheDocument();
  });

  it("«Comprobar adaptador» prueba la ruta escrita y muestra versión y capacidades", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(settings());
    mocked.checkAcpAdapter.mockResolvedValue({
      protocolVersion: 1,
      agentName: "@agentclientprotocol/claude-agent-acp",
      agentTitle: "Claude Agent",
      agentVersion: "0.107.0",
      capabilities: ["loadSession", "image", "embeddedContext", "mcpHttp"],
    });
    render(<RunSettingsPanel />);

    const path = await screen.findByLabelText("Adaptador de Claude Code (ACP)");
    await user.type(path, ACP_PATH);
    const button = screen.getByRole("button", {
      name: "Comprobar adaptador de Claude Code",
    });
    await user.click(button);
    expect(mocked.checkAcpAdapter).toHaveBeenCalledWith("claudeCode", ACP_PATH);
    expect(
      await screen.findByText(
        "✓ Responde: Claude Agent 0.107.0 · ACP v1 · capacidades: retomar sesión, imágenes, contexto incrustado, MCP por HTTP",
      ),
    ).toBeInTheDocument();
    expect(button).toHaveAccessibleDescription(/Responde: Claude Agent/);
    // Comprobar no guarda.
    expect(mocked.updateRunSettings).not.toHaveBeenCalled();

    // Al editar la ruta, el resultado anterior desaparece.
    await user.type(path, "x");
    expect(screen.queryByText(/Responde:/)).not.toBeInTheDocument();
  });

  it("con el campo vacío no se puede comprobar y lo explica", async () => {
    mocked.getRunSettings.mockResolvedValue(settings());
    render(<RunSettingsPanel />);

    await screen.findByLabelText("Adaptador de Claude Code (ACP)");
    const button = screen.getByRole("button", {
      name: "Comprobar adaptador de Claude Code",
    });
    expect(button).toBeDisabled();
    expect(button).toHaveAccessibleDescription(
      "Escribe la ruta para comprobarla.",
    );
    expect(mocked.checkAcpAdapter).not.toHaveBeenCalled();
  });

  it("un fallo al comprobar se muestra con su motivo", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(
      settings({ claudeAcpPath: ACP_PATH }),
    );
    const message =
      "La ruta del adaptador no existe o no es un archivo ejecutable.";
    mocked.checkAcpAdapter.mockRejectedValue(
      validation("claudeAcpPath", message),
    );
    render(<RunSettingsPanel />);

    await screen.findByLabelText("Adaptador de Claude Code (ACP)");
    const button = screen.getByRole("button", {
      name: "Comprobar adaptador de Claude Code",
    });
    expect(button).toBeEnabled();
    // El aviso de campo vacío solo queda en el adaptador de Codex (vacío).
    expect(button).not.toHaveAccessibleDescription(
      "Escribe la ruta para comprobarla.",
    );
    await user.click(button);
    // La ruta mostrada (la guardada) es la que se comprueba.
    expect(mocked.checkAcpAdapter).toHaveBeenCalledWith("claudeCode", ACP_PATH);
    expect(
      await screen.findByText(`No se pudo comprobar el adaptador: ${message}`),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Responde:/)).not.toBeInTheDocument();
  });
});

// --- HU-021: segundo agente real (Codex por ACP) ---

const CODEX_PATH = "/Users/ana/bin/codex-acp";

describe("Iniciar con Codex (ACP)", () => {
  it("sin adaptador de Codex, la opción está deshabilitada y explica dónde configurarlo", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(
      settings({ claudeAcpPath: ACP_PATH }),
    );
    mocked.getCodexHomeStatus.mockResolvedValue({
      path: CODEX_HOME,
      signedIn: true,
    });
    renderPanel();

    const region = await openDetail(user);
    const executors = await within(region).findByRole("group", {
      name: "Ejecutor",
    });
    const claude = within(executors).getByRole("radio", {
      name: "Claude Code (ACP)",
    });
    const codex = within(executors).getByRole("radio", {
      name: "Codex (ACP)",
    });
    await waitFor(() => {
      expect(claude).toBeEnabled();
    });
    expect(codex).toBeDisabled();
    expect(codex).toHaveAccessibleDescription(
      "No disponible: configura el adaptador de Codex en Sistema → Ejecuciones (ruta de codex-acp, al final de la ventana).",
    );
  });

  it("con adaptador pero sin sesión en la carpeta de Codex, sigue deshabilitada y se puede volver a comprobar", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(
      settings({ codexAcpPath: CODEX_PATH }),
    );
    mocked.getCodexHomeStatus
      .mockResolvedValueOnce({ path: CODEX_HOME, signedIn: false })
      .mockResolvedValue({ path: CODEX_HOME, signedIn: true });
    renderPanel();

    const region = await openDetail(user);
    const codex = await within(region).findByRole("radio", {
      name: "Codex (ACP)",
    });
    await waitFor(() => {
      expect(codex).toHaveAccessibleDescription(
        "No disponible: inicia sesión en la carpeta de Codex de la app (Sistema → Ejecuciones).",
      );
    });
    expect(codex).toBeDisabled();
    // Claude Code sin adaptador: deshabilitado por su propio motivo.
    expect(
      within(region).getByRole("radio", { name: "Claude Code (ACP)" }),
    ).toHaveAccessibleDescription(
      /configura el adaptador de Claude Code en Sistema → Ejecuciones/,
    );

    await user.click(
      within(region).getByRole("button", {
        name: "Volver a comprobar la sesión de Codex",
      }),
    );
    await waitFor(() => {
      expect(codex).toBeEnabled();
    });
    expect(codex).not.toHaveAccessibleDescription();
  });

  it("si Codex deja de estar disponible tras elegirlo, «Iniciar» no lo envía y lo explica", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(
      settings({ codexAcpPath: CODEX_PATH }),
    );
    mocked.getCodexHomeStatus
      .mockResolvedValueOnce({ path: CODEX_HOME, signedIn: true })
      .mockResolvedValue({ path: CODEX_HOME, signedIn: false });
    renderPanel();

    const region = await openDetail(user);
    const codex = await within(region).findByRole("radio", {
      name: "Codex (ACP)",
    });
    await waitFor(() => {
      expect(codex).toBeEnabled();
    });
    await user.click(codex);
    expect(codex).toBeChecked();

    // Se cerró la sesión en la terminal y se vuelve a comprobar con Codex elegido.
    await user.click(
      within(region).getByRole("button", {
        name: "Volver a comprobar la sesión de Codex",
      }),
    );
    await waitFor(() => {
      expect(codex).toBeDisabled();
    });
    expect(codex).toHaveAccessibleDescription(
      "No disponible: inicia sesión en la carpeta de Codex de la app (Sistema → Ejecuciones).",
    );

    await user.click(within(region).getByRole("button", { name: "Iniciar" }));
    expect(mocked.startRun).not.toHaveBeenCalled();
    const message =
      "Codex ya no está disponible (el motivo está junto a su opción): elige otro ejecutor.";
    const error = within(region).getByText(message);
    expect(error).toHaveAttribute("role", "alert");
    const group = within(region).getByRole("group", { name: "Ejecutor" });
    expect(group.getAttribute("aria-describedby")).toBe(error.id);

    // Elegir otro ejecutor quita el aviso y permite iniciar.
    await user.click(
      within(region).getByRole("radio", { name: "Proceso de prueba (sin IA)" }),
    );
    expect(within(region).queryByText(message)).not.toBeInTheDocument();
    mocked.startRun.mockResolvedValue(
      summary({ startRequestedAt: "2026-11-12T10:02:00Z" }),
    );
    await user.click(within(region).getByRole("button", { name: "Iniciar" }));
    expect(mocked.startRun).toHaveBeenCalledWith(
      expect.objectContaining({
        executor: { kind: "testProcess", scenario: "success" },
      }),
    );
  });

  it("elegir Codex avisa de lo que no pide permiso y envía codexAcp", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(
      settings({ codexAcpPath: CODEX_PATH }),
    );
    mocked.getCodexHomeStatus.mockResolvedValue({
      path: CODEX_HOME,
      signedIn: true,
    });
    mocked.startRun.mockResolvedValue(
      summary({ startRequestedAt: "2026-11-12T10:02:00Z" }),
    );
    renderPanel();

    const region = await openDetail(user);
    const codex = await within(region).findByRole("radio", {
      name: "Codex (ACP)",
    });
    await waitFor(() => {
      expect(codex).toBeEnabled();
    });
    await user.click(codex);

    const form = within(region).getByRole("form", {
      name: "Iniciar el intento #1",
    });
    expect(
      within(form).queryByRole("group", {
        name: "Escenario del proceso de prueba",
      }),
    ).not.toBeInTheDocument();
    expect(form).toHaveTextContent(
      "Atención (comprobado con Codex real): lee archivos sin pedir permiso, también fuera del worktree (podría leer los datos de la app), y rechazar una escritura detiene su turno.",
    );
    expect(form).toHaveTextContent(
      "su nombre y su avatar no cambian según el ejecutor que elijas.",
    );
    const submit = within(form).getByRole("button", { name: "Iniciar" });
    expect(submit).toHaveAccessibleDescription("Codex (ACP)");

    await user.click(submit);
    expect(mocked.startRun).toHaveBeenCalledWith({
      requestId: expect.any(String) as string,
      projectId: "p1",
      runId: "r1",
      executor: { kind: "codexAcp" },
    });
    expect(
      await screen.findByText(
        "✓ Intento #1: inicio pedido con Codex (ACP). Espera turno en la cola.",
      ),
    ).toBeInTheDocument();
  });

  it("la comparación dice qué admite cada agente, en texto y sin afirmar tokens, coste ni reanudación", async () => {
    const user = userEvent.setup();
    renderPanel();

    const region = await openDetail(user);
    const table = await within(region).findByRole("table", {
      name: "Qué admite cada agente en la app",
    });
    const headers = within(table)
      .getAllByRole("columnheader")
      .map((h) => h.textContent);
    expect(headers).toEqual(["Aspecto", "Claude Code", "Codex"]);
    const row = (name: string) =>
      within(table)
        .getByRole("rowheader", { name })
        .closest("tr")
        ?.textContent.replace(name, "");
    expect(row("Leer archivos")).toContain(
      "Lee sin preguntar, también fuera del worktree",
    );
    expect(row("Leer archivos")).toContain(
      "para leer fuera del worktree pide permiso",
    );
    expect(row("Al rechazar una escritura")).toContain(
      "Su turno se detiene: Codex no admite «rechazar y continuar» al escribir archivos.",
    );
    expect(row("Tokens y coste")).toBe(
      "No se guardan en el intento (no disponible).No se guardan en el intento (no disponible).",
    );
    expect(row("Retomar una sesión")).toBe(
      "No disponible en la app.No disponible en la app.",
    );
    expect(row("Red")).toBe(
      "No comprobado.Pide permiso; sin permiso, su sandbox la bloquea. * (según el código del adaptador 2.5.0; sin comprobar en real)",
    );
    // Lo de Claude Code comprobado en real no lleva la marca; lo de Codex deducido del
    // código, sí (marca visible + leyenda; lectores de pantalla oyen la leyenda en la celda).
    expect(row("Al rechazar una escritura")).toBe(
      "El agente sigue con su turno.Su turno se detiene: Codex no admite «rechazar y continuar» al escribir archivos.",
    );
    expect(row("Leer archivos")).not.toContain("según el código");
    expect(row("Llamadas adicionales al modelo")).toMatch(
      /sesión\. \* \(según/,
    );
    expect(region).toHaveTextContent(
      "* Dato según el código del adaptador 2.5.0; sin comprobar en real.",
    );
    expect(row("Tu configuración")).toContain(
      "No se lanza si el repositorio contiene .codex/.",
    );
    expect(row("Llamadas adicionales al modelo")).toContain(
      "Una, para poner título a la sesión.",
    );
  });
});

describe("Detalle con Codex (ACP)", () => {
  function codexDetail(model: string | null): RunDetail {
    const ended = summary({
      state: "completed",
      endedAt: "2026-11-12T10:05:00Z",
      endReason: "El agente detuvo su turno tras un permiso rechazado.",
    });
    return detail(ended, {
      startRequest: {
        repositoryId: "g1",
        executor: { kind: "codexAcp" },
        requestedAt: "2026-11-12T10:02:00Z",
        dispatchedAt: "2026-11-12T10:02:01Z",
        voidedAt: null,
      },
      events: [
        {
          sequence: 1,
          at: "2026-11-12T10:02:05Z",
          event: {
            kind: "acp_session",
            protocolVersion: 1,
            agentName: "codex-acp",
            agentTitle: "Codex <i>ACP</i>",
            agentVersion: "0.4.0",
            capabilities: ["image"],
            mode: "auto",
            promptSha256: null,
            promptChars: null,
            model,
          },
        },
      ],
    });
  }

  it("muestra el ejecutor, el motivo y el modelo declarado como texto plano", async () => {
    const user = userEvent.setup();
    mocked.getRun.mockResolvedValue(codexDetail("gpt-5-codex <b>x</b>"));
    const { container } = renderPanel();

    const region = await openDetail(user);
    const execution = within(region).getByRole("region", { name: "Ejecución" });
    expect(execution).toHaveTextContent("EjecutorCodex (ACP)");
    expect(region).toHaveTextContent(
      "(lo ejecuta Codex por ACP con las instrucciones de la instantánea).",
    );
    expect(region).toHaveTextContent(
      "Motivo: El agente detuvo su turno tras un permiso rechazado.",
    );
    expect(region).toHaveTextContent("Ejecutor: Codex por ACP.");
    expect(region).not.toHaveTextContent("Claude Code por ACP");
    const session = within(region).getByRole("region", {
      name: "Sesión del agente (reportado por el agente)",
    });
    expect(session).toHaveTextContent(
      "Modelo (reportado por el agente)gpt-5-codex <b>x</b>",
    );
    expect(session).toHaveTextContent("AgenteCodex <i>ACP</i> 0.4.0");
    expect(container.querySelector("b, i")).toBeNull();
  });

  it("sin modelo declarado dice «no informado»", async () => {
    const user = userEvent.setup();
    mocked.getRun.mockResolvedValue(codexDetail(null));
    renderPanel();

    const region = await openDetail(user);
    const session = within(region).getByRole("region", {
      name: "Sesión del agente (reportado por el agente)",
    });
    expect(session).toHaveTextContent(
      "Modelo (reportado por el agente)no informado",
    );
  });
});

describe("Sistema → Ejecuciones: Codex", () => {
  it("guarda la ruta de Codex sin tocar la de Claude Code", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(
      settings({ claudeAcpPath: ACP_PATH }),
    );
    mocked.updateRunSettings.mockResolvedValue(
      settings({
        claudeAcpPath: ACP_PATH,
        codexAcpPath: CODEX_PATH,
        revision: 2,
      }),
    );
    render(<RunSettingsPanel />);

    const path = await screen.findByLabelText("Adaptador de Codex (ACP)");
    expect(path).toHaveValue("");
    expect(path).toHaveAccessibleDescription(
      /Ruta absoluta del ejecutable codex-acp.*Comprobado con Codex real: lee archivos sin preguntar, también fuera del worktree, y rechazar una escritura detiene su turno\./,
    );
    await user.type(path, `  ${CODEX_PATH} `);
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    expect(mocked.updateRunSettings).toHaveBeenCalledWith({
      expectedRevision: 1,
      maxConcurrency: 2,
      timeoutMinutes: 30,
      claudeAcpPath: ACP_PATH,
      codexAcpPath: CODEX_PATH,
      approvalNotifications: false,
    });
    expect(
      await screen.findByText("✓ Configuración guardada."),
    ).toBeInTheDocument();
    expect(path).toHaveValue(CODEX_PATH);
  });

  it("un error del núcleo en codexAcpPath aparece junto al campo de Codex", async () => {
    const user = userEvent.setup();
    const message = "La ruta del adaptador de Codex debe ser absoluta.";
    mocked.updateRunSettings.mockRejectedValue(
      validation("codexAcpPath", message),
    );
    render(<RunSettingsPanel />);

    const codex = await screen.findByLabelText("Adaptador de Codex (ACP)");
    const claude = screen.getByLabelText("Adaptador de Claude Code (ACP)");
    await user.type(codex, "bin/codex-acp");
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    const error = await screen.findByText(message);
    expect(codex).toHaveAttribute("aria-invalid", "true");
    expect(codex.getAttribute("aria-describedby")).toContain(error.id);
    expect(claude).not.toHaveAttribute("aria-invalid");
    expect(screen.queryByText(/No se pudo guardar/)).not.toBeInTheDocument();
  });

  it("cada adaptador se comprueba con su agente y su resultado es independiente", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(
      settings({ claudeAcpPath: ACP_PATH, codexAcpPath: CODEX_PATH }),
    );
    const failure = "El adaptador no respondió a tiempo.";
    // Primero se comprueba Codex (responde) y después Claude Code (falla).
    mocked.checkAcpAdapter
      .mockResolvedValueOnce({
        protocolVersion: 1,
        agentName: "codex-acp",
        agentTitle: "Codex",
        agentVersion: "0.4.0",
        capabilities: ["image"],
      })
      .mockRejectedValueOnce(validation("claudeAcpPath", failure));
    render(<RunSettingsPanel />);

    await screen.findByLabelText("Adaptador de Codex (ACP)");
    const checkCodex = screen.getByRole("button", {
      name: "Comprobar adaptador de Codex",
    });
    const checkClaude = screen.getByRole("button", {
      name: "Comprobar adaptador de Claude Code",
    });
    await user.click(checkCodex);
    expect(mocked.checkAcpAdapter).toHaveBeenCalledWith("codex", CODEX_PATH);
    await waitFor(() => {
      expect(checkCodex).toHaveAccessibleDescription(
        "✓ Responde: Codex 0.4.0 · ACP v1 · capacidades: imágenes",
      );
    });
    expect(checkClaude).not.toHaveAccessibleDescription(/Responde/);

    await user.click(checkClaude);
    expect(mocked.checkAcpAdapter).toHaveBeenLastCalledWith(
      "claudeCode",
      ACP_PATH,
    );
    await waitFor(() => {
      expect(checkClaude).toHaveAccessibleDescription(
        `No se pudo comprobar el adaptador: ${failure}`,
      );
    });
    // El resultado de Codex sigue ahí.
    expect(checkCodex).toHaveAccessibleDescription(/Responde: Codex 0\.4\.0/);
  });

  it("muestra la carpeta de Codex, su sesión y el comando para iniciarla, y vuelve a comprobar", async () => {
    const user = userEvent.setup();
    mocked.getCodexHomeStatus
      .mockResolvedValueOnce({ path: CODEX_HOME, signedIn: false })
      .mockResolvedValue({ path: CODEX_HOME, signedIn: true });
    render(<RunSettingsPanel />);

    const section = await screen.findByRole("region", {
      name: "Carpeta de Codex de la app",
    });
    expect(section).toHaveTextContent(
      "no con tu ~/.codex: no carga tu configuración, tus servidores MCP, tus reglas ni tu historial de Codex.",
    );
    expect(await within(section).findByText(CODEX_HOME)).toBeInTheDocument();
    expect(section).toHaveTextContent(
      "SesiónNo hay credenciales guardadas en la carpeta de Codex",
    );
    expect(section).toHaveTextContent(
      "Necesitas la CLI de Codex (codex) instalada.",
    );
    const posix = within(section).getByText(
      `CODEX_HOME='${CODEX_HOME}' codex login`,
    );
    expect(posix).toHaveClass("codex-home__command");
    expect(posix).toHaveAccessibleDescription("macOS / Linux (zsh, bash)");
    const pwsh = within(section).getByText(
      `$env:CODEX_HOME='${CODEX_HOME}'; codex login`,
    );
    expect(pwsh).toHaveClass("codex-home__command");
    expect(pwsh).toHaveAccessibleDescription("Windows (PowerShell)");

    await user.click(
      within(section).getByRole("button", { name: "Volver a comprobar" }),
    );
    await waitFor(() => {
      expect(section).toHaveTextContent(
        "SesiónHay credenciales guardadas en la carpeta de Codex (no se comprueba si siguen siendo válidas)",
      );
    });
    expect(mocked.getCodexHomeStatus).toHaveBeenCalledTimes(2);
  });

  it("si no se puede comprobar la carpeta de Codex, lo dice y permite reintentar", async () => {
    const user = userEvent.setup();
    mocked.getCodexHomeStatus
      .mockRejectedValueOnce({
        code: "storage_unavailable",
        message: "El almacenamiento no está disponible.",
        recoverable: true,
      } satisfies ErrorPayload)
      .mockResolvedValue({ path: CODEX_HOME, signedIn: true });
    render(<RunSettingsPanel />);

    expect(
      await screen.findByText(
        "No se pudo comprobar la carpeta de Codex: El almacenamiento no está disponible.",
      ),
    ).toHaveAttribute("role", "alert");
    await user.click(
      screen.getByRole("button", { name: "Volver a comprobar" }),
    );
    expect(
      await screen.findByText(
        "Hay credenciales guardadas en la carpeta de Codex (no se comprueba si siguen siendo válidas)",
      ),
    ).toBeInTheDocument();
  });
});

describe("comandos de inicio de sesión de Codex", () => {
  it("entrecomilla la ruta y escapa lo que la shell interpretaría", () => {
    // POSIX: comillas simples (nada se expande); una ' se escribe '\''.
    expect(codexLoginCommand("/a b/c")).toBe("CODEX_HOME='/a b/c' codex login");
    expect(codexLoginCommand('/x/$HOME/"q"/`id`/\\/it\'s')).toBe(
      "CODEX_HOME='/x/$HOME/\"q\"/`id`/\\/it'\\''s' codex login",
    );
    // PowerShell: comillas simples; una ' se escribe ''.
    expect(codexLoginCommandPowerShell("C:\\Users\\o'brien\\$x")).toBe(
      "$env:CODEX_HOME='C:\\Users\\o''brien\\$x'; codex login",
    );
  });
});
