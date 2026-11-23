// HU-025: limpieza de ejecuciones en «Sistema» (plan a demanda, casillas, confirmación y
// resultados), los días de retención en «Ejecuciones» y el aviso de salida eliminada.
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DiskUsage } from "../../contracts/generated/DiskUsage";
import type { ErrorPayload } from "../../contracts/generated/ErrorPayload";
import type { OrphanWorktree } from "../../contracts/generated/OrphanWorktree";
import type { RetentionItem } from "../../contracts/generated/RetentionItem";
import type { RetentionPlan } from "../../contracts/generated/RetentionPlan";
import type { RunDetail } from "../../contracts/generated/RunDetail";
import type { RunSettings } from "../../contracts/generated/RunSettings";
import type { RunWorktree } from "../../contracts/generated/RunWorktree";
import * as commands from "../../shared/ipc/commands";
import { formatDateTime } from "../../shared/ui/format";
import { RetentionPanel, formatBytes, formatUsage } from "./RetentionPanel";
import { RunDetailView } from "./RunDetailView";
import { RETENTION_LIMITS, RunSettingsPanel } from "./RunSettingsPanel";

// Solo se simulan las llamadas IPC; errores y helpers son los reales.
vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  getRetentionPlan: vi.fn(),
  cleanRun: vi.fn(),
  removeOrphanWorktree: vi.fn(),
  getRun: vi.fn(),
  getRunOutput: vi.fn(),
  getRunSettings: vi.fn(),
  updateRunSettings: vi.fn(),
  checkAcpAdapter: vi.fn(),
  getCodexHomeStatus: vi.fn(),
  setUnsavedChanges: vi.fn(),
}));

const mocked = vi.mocked(commands);

const KB = 1024;
const MB = 1024 * 1024;

function usage(bytes: number, complete = true, files = 3): DiskUsage {
  return { bytes, files, complete };
}

function item(overrides: Partial<RetentionItem> = {}): RetentionItem {
  return {
    runId: "r1",
    projectId: "p1",
    projectName: "Recetario",
    storyNumber: 7,
    storyTitle: "Buscar recetas",
    runNumber: 2,
    state: "completed",
    endedAt: "2026-09-01T10:00:00Z",
    output: usage(2 * MB),
    worktree: {
      path: "/datos/worktrees/p1/r1",
      branch: "kanbania/h7-i2",
      usage: usage(10 * MB),
      check: { kind: "clean" },
    },
    branch: {
      name: "kanbania/h7-i2",
      unintegratedCommits: 0,
      worktreeLive: true,
    },
    ...overrides,
  };
}

function orphan(overrides: Partial<OrphanWorktree> = {}): OrphanWorktree {
  return {
    path: "/datos/worktrees/p1/huerfano",
    branch: "kanbania/h3-i1",
    usage: usage(5 * MB),
    check: { kind: "clean" },
    ...overrides,
  };
}

function plan(overrides: Partial<RetentionPlan> = {}): RetentionPlan {
  return {
    retentionDays: 30,
    cutoff: "2026-10-21T10:00:00Z",
    runsUsage: usage(1536),
    worktreesUsage: usage(3 * 1024 * MB, false, 1000),
    items: [],
    itemsTruncated: false,
    orphans: [],
    orphansTruncated: false,
    ...overrides,
  };
}

function validation(field: string, message: string): ErrorPayload {
  return { code: "validation", message, recoverable: true, field };
}

const CALC = "Calcular qué se puede limpiar";

async function renderAndCalculate(p: RetentionPlan) {
  const user = userEvent.setup();
  mocked.getRetentionPlan.mockResolvedValue(p);
  render(<RetentionPanel />);
  await user.click(screen.getByRole("button", { name: CALC }));
  await screen.findByRole("button", { name: "Volver a calcular" });
  return user;
}

function group(name: RegExp) {
  return screen.getByRole("group", { name });
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("formato de tamaños", () => {
  it("usa B, KB, MB y GB con un decimal y «al menos» si la medición no terminó", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1,5 KB");
    expect(formatBytes(2 * MB)).toBe("2,0 MB");
    expect(formatBytes(3.25 * 1024 * MB)).toBe("3,3 GB");
    expect(formatUsage(usage(KB, false))).toBe("al menos 1,0 KB");
    expect(formatUsage(usage(KB))).toBe("1,0 KB");
  });
});

describe("Limpieza de ejecuciones", () => {
  it("no calcula al montar: espera al botón y muestra un estado mientras calcula", async () => {
    const user = userEvent.setup();
    let resolve: (p: RetentionPlan) => void = () => undefined;
    mocked.getRetentionPlan.mockReturnValue(
      new Promise((r) => {
        resolve = r;
      }),
    );
    render(<RetentionPanel />);
    expect(
      screen.getByRole("heading", { name: "Limpieza de ejecuciones" }),
    ).toBeInTheDocument();
    expect(mocked.getRetentionPlan).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: CALC }));
    expect(mocked.getRetentionPlan).toHaveBeenCalledTimes(1);
    expect(
      screen.getByText(/Calculando qué se puede limpiar/),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: CALC })).toBeDisabled();

    resolve(plan());
    expect(
      await screen.findByText(
        "No hay nada que limpiar con la antigüedad configurada.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Calculando/)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Limpiar lo seleccionado…" }),
    ).not.toBeInTheDocument();
  });

  it("muestra cuánto ocupa todo, con «al menos» si la medición se cortó", async () => {
    await renderAndCalculate(plan());
    expect(
      screen.getByText("Salida de los intentos").nextElementSibling,
    ).toHaveTextContent("1,5 KB (3 archivos)");
    expect(screen.getByText("Worktrees").nextElementSibling).toHaveTextContent(
      "al menos 3,0 GB (al menos 1000 archivos)",
    );
  });

  it("un error al calcular se muestra y se puede reintentar", async () => {
    const user = userEvent.setup();
    mocked.getRetentionPlan.mockRejectedValueOnce({
      code: "internal",
      message: "Git no respondió.",
      recoverable: true,
    } satisfies ErrorPayload);
    render(<RetentionPanel />);
    await user.click(screen.getByRole("button", { name: CALC }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "No se pudo calcular la limpieza: Git no respondió.",
    );
    mocked.getRetentionPlan.mockResolvedValue(plan());
    await user.click(screen.getByRole("button", { name: CALC }));
    expect(
      await screen.findByText(
        "No hay nada que limpiar con la antigüedad configurada.",
      ),
    ).toBeInTheDocument();
  });

  it("lista los intentos con su descripción y las casillas por defecto", async () => {
    await renderAndCalculate(
      plan({
        items: [
          item(),
          item({
            runId: "r2",
            storyNumber: 8,
            runNumber: 1,
            state: "failed",
            output: null,
            worktree: {
              path: "/datos/worktrees/p1/r2",
              branch: "kanbania/h8-i1",
              usage: usage(MB),
              check: {
                kind: "needsForce",
                reason: "Tiene 2 cambios sin confirmar.",
              },
            },
            branch: null,
          }),
          item({
            runId: "r3",
            storyNumber: 9,
            output: null,
            worktree: {
              path: "/datos/worktrees/p1/r3",
              branch: "kanbania/h9-i2",
              usage: null,
              check: {
                kind: "unverifiable",
                reason: "Git no pudo leer el worktree.",
              },
            },
            branch: null,
          }),
        ],
      }),
    );

    expect(
      screen.getByRole("heading", {
        name: `Intentos terminados antes del ${formatDateTime("2026-10-21T10:00:00Z")} (más de 30 días)`,
      }),
    ).toBeInTheDocument();

    const first = group(/^#7 Buscar recetas · intento #2/);
    expect(first).toHaveAccessibleName(
      `#7 Buscar recetas · intento #2 · Recetario · Completado · terminó ${formatDateTime("2026-09-01T10:00:00Z")}`,
    );
    expect(
      within(first).getByRole("checkbox", { name: "Salida (2,0 MB)" }),
    ).toBeChecked();
    expect(
      within(first).getByRole("checkbox", { name: "Worktree (10,0 MB)" }),
    ).toBeChecked();
    expect(
      within(first).getByRole("checkbox", {
        name: "Borrar la rama kanbania/h7-i2",
      }),
    ).not.toBeChecked();

    const second = group(/^#8 /);
    expect(second).toHaveAccessibleName(/Fallido/);
    expect(
      within(second).getByText("Sin salida guardada."),
    ).toBeInTheDocument();
    const wt = within(second).getByRole("checkbox", {
      name: "Worktree (1,0 MB)",
    });
    expect(wt).not.toBeChecked();
    expect(
      within(second).getByText("Tiene 2 cambios sin confirmar."),
    ).toBeInTheDocument();

    const third = group(/^#9 /);
    expect(within(third).queryByRole("checkbox")).not.toBeInTheDocument();
    expect(third).toHaveTextContent(
      "no se puede eliminar desde aquí. Git no pudo leer el worktree.",
    );
  });

  it("«Descartar» solo se habilita con el worktree marcado y es necesario para limpiarlo", async () => {
    const user = await renderAndCalculate(
      plan({
        items: [
          item({
            output: null,
            worktree: {
              path: "/datos/worktrees/p1/r1",
              branch: "kanbania/h7-i2",
              usage: usage(MB),
              check: {
                kind: "needsForce",
                reason: "Tiene archivos ignorados.",
              },
            },
            branch: null,
          }),
        ],
      }),
    );
    const row = group(/^#7 /);
    const wt = within(row).getByRole("checkbox", { name: /^Worktree/ });
    const discard = within(row).getByRole("checkbox", {
      name: "Descartar lo que tiene y eliminar",
    });
    const cleanButton = screen.getByRole("button", {
      name: "Limpiar lo seleccionado…",
    });
    expect(discard).toBeDisabled();
    expect(discard).toHaveAccessibleDescription("Tiene archivos ignorados.");
    expect(cleanButton).toBeDisabled();
    expect(cleanButton).toHaveAccessibleDescription(
      "Marca al menos un elemento para poder limpiar.",
    );

    await user.click(wt);
    expect(discard).toBeEnabled();
    // Marcado sin «Descartar» no funcionaría: no cuenta y se explica.
    expect(
      within(row).getByText(/Sin «Descartar lo que tiene y eliminar»/),
    ).toBeInTheDocument();
    expect(cleanButton).toBeDisabled();

    await user.click(discard);
    expect(cleanButton).toBeEnabled();

    // Desmarcar el worktree desmarca y deshabilita «Descartar».
    await user.click(wt);
    expect(discard).not.toBeChecked();
    expect(discard).toBeDisabled();
  });

  it("borrar una rama con worktree vivo exige marcar el worktree; avisa del trabajo sin integrar", async () => {
    const user = await renderAndCalculate(
      plan({
        items: [
          item({
            output: null,
            branch: {
              name: "kanbania/h7-i2",
              unintegratedCommits: 3,
              worktreeLive: true,
            },
          }),
          item({
            runId: "r2",
            storyNumber: 8,
            output: null,
            worktree: null,
            branch: {
              name: "kanbania/h8-i1",
              unintegratedCommits: null,
              worktreeLive: false,
            },
          }),
          item({
            runId: "r3",
            storyNumber: 9,
            output: null,
            worktree: null,
            branch: {
              name: "kanbania/h9-i1",
              unintegratedCommits: 1,
              worktreeLive: false,
            },
          }),
        ],
      }),
    );
    const row = group(/^#7 /);
    const wt = within(row).getByRole("checkbox", { name: /^Worktree/ });
    const branch = within(row).getByRole("checkbox", {
      name: "Borrar la rama kanbania/h7-i2",
    });
    expect(branch).toHaveAccessibleDescription(
      /Trabajo sin integrar: 3 commits que no están en otra rama\..*Para borrar la rama hay que eliminar también su worktree/,
    );
    expect(wt).toBeChecked();
    expect(branch).toBeEnabled();

    await user.click(branch);
    expect(branch).toBeChecked();
    await user.click(wt);
    expect(branch).not.toBeChecked();
    expect(branch).toBeDisabled();

    const unknown = within(group(/^#8 /)).getByRole("checkbox", {
      name: "Borrar la rama kanbania/h8-i1",
    });
    expect(unknown).toBeEnabled();
    expect(unknown).not.toBeChecked();
    expect(unknown).toHaveAccessibleDescription(
      /No se pudo saber si tiene trabajo sin integrar/,
    );
    expect(
      within(group(/^#9 /)).getByRole("checkbox", {
        name: "Borrar la rama kanbania/h9-i1",
      }),
    ).toHaveAccessibleDescription(
      "Trabajo sin integrar: 1 commit que no está en otra rama.",
    );
  });

  it("los worktrees sin intento se explican y no se marcan solos", async () => {
    await renderAndCalculate(
      plan({
        orphans: [
          orphan(),
          orphan({
            path: "/datos/worktrees/p1/raro",
            branch: null,
            usage: null,
            check: { kind: "unverifiable", reason: "No es un worktree." },
          }),
        ],
        orphansTruncated: true,
      }),
    );
    expect(
      screen.getByRole("heading", { name: "Worktrees sin intento" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Carpetas de worktrees que la base no conoce (p. ej. tras restaurar una copia anterior). La rama se conserva.",
      ),
    ).toBeInTheDocument();
    const first = group(/huerfano$/);
    expect(first).toHaveTextContent(
      "Rama: kanbania/h3-i1 · 5,0 MB · sin cambios",
    );
    expect(
      within(first).getByRole("checkbox", { name: "Eliminar" }),
    ).not.toBeChecked();
    const odd = group(/raro$/);
    expect(within(odd).queryByRole("checkbox")).not.toBeInTheDocument();
    expect(odd).toHaveTextContent("No es un worktree.");
    expect(
      screen.getByText(/Hay más carpetas sin intento de las que se revisan/),
    ).toBeInTheDocument();
  });

  it("avisa si hay más intentos de los mostrados", async () => {
    await renderAndCalculate(plan({ items: [item()], itemsTruncated: true }));
    expect(
      screen.getByText(
        "Hay más intentos para limpiar de los que se muestran: limpia estos y vuelve a calcular.",
      ),
    ).toBeInTheDocument();
  });

  /** Plan con de todo: limpio con rama sin integrar, con cambios y huérfanos. */
  function fullPlan(): RetentionPlan {
    return plan({
      items: [
        item(),
        item({
          runId: "r2",
          projectId: "p2",
          projectName: "Otro",
          storyNumber: 8,
          runNumber: 1,
          output: usage(KB, false),
          worktree: {
            path: "/datos/worktrees/p2/r2",
            branch: "kanbania/h8-i1",
            usage: usage(MB),
            check: { kind: "needsForce", reason: "Tiene cambios." },
          },
          branch: {
            name: "kanbania/h8-i1",
            unintegratedCommits: 2,
            worktreeLive: true,
          },
        }),
        // Nada marcado por defecto (sin salida, worktree sin verificar): se omite.
        item({
          runId: "r3",
          storyNumber: 9,
          output: null,
          worktree: {
            path: "/datos/worktrees/p1/r3",
            branch: "kanbania/h9-i1",
            usage: null,
            check: { kind: "unverifiable", reason: "Sin Git." },
          },
          branch: null,
        }),
      ],
      orphans: [
        orphan(),
        orphan({
          path: "/datos/worktrees/p1/sucio",
          check: { kind: "needsForce", reason: "Tiene commits fuera de rama." },
        }),
      ],
    });
  }

  async function selectAll(user: ReturnType<typeof userEvent.setup>) {
    const second = group(/^#8 /);
    await user.click(
      within(second).getByRole("checkbox", { name: /^Worktree/ }),
    );
    await user.click(
      within(second).getByRole("checkbox", {
        name: "Descartar lo que tiene y eliminar",
      }),
    );
    await user.click(
      within(second).getByRole("checkbox", { name: /^Borrar la rama/ }),
    );
    await user.click(
      within(group(/huerfano$/)).getByRole("checkbox", { name: "Eliminar" }),
    );
    const dirty = group(/sucio$/);
    await user.click(within(dirty).getByRole("checkbox", { name: "Eliminar" }));
    await user.click(
      within(dirty).getByRole("checkbox", { name: "Descartar lo que tiene" }),
    );
  }

  it("la confirmación resume lo marcado y cancelar no borra nada", async () => {
    const user = await renderAndCalculate(fullPlan());
    await selectAll(user);
    await user.click(
      screen.getByRole("button", { name: "Limpiar lo seleccionado…" }),
    );

    const dialog = screen.getByRole("alertdialog", {
      name: "¿Limpiar lo seleccionado? No se puede deshacer.",
    });
    // r1: salida 2 MB + worktree 10 MB; r2: salida ≥1 KB + worktree 1 MB; huérfanos 5 + 5 MB.
    expect(dialog).toHaveAccessibleDescription(
      "Se borrará: 2 salidas de intentos. 2 worktrees (1 descartando lo que tiene). 1 rama (1 con trabajo sin integrar). 2 worktrees sin intento (1 descartando lo que tiene). Se liberarán aproximadamente al menos 23,0 MB.",
    );
    const cancel = within(dialog).getByRole("button", { name: "Cancelar" });
    expect(cancel).toHaveFocus();
    // Mientras se confirma, las casillas no cambian.
    expect(
      within(group(/^#7 /)).getByRole("checkbox", { name: /^Salida/ }),
    ).toBeDisabled();

    await user.click(cancel);
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(mocked.cleanRun).not.toHaveBeenCalled();
    expect(mocked.removeOrphanWorktree).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: "Limpiar lo seleccionado…" }),
    ).toHaveFocus();

    // Escape también cancela.
    await user.click(
      screen.getByRole("button", { name: "Limpiar lo seleccionado…" }),
    );
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(mocked.cleanRun).not.toHaveBeenCalled();
  });

  it("limpia en orden con lo marcado, sigue tras un error, muestra resultados y recalcula", async () => {
    const user = await renderAndCalculate(fullPlan());
    await selectAll(user);
    mocked.cleanRun
      .mockResolvedValueOnce({
        outputRemoved: true,
        worktreeRemoval: "clean",
        branchDeleted: false,
      })
      .mockRejectedValueOnce(
        validation("force", "El worktree cambió: vuelve a calcular."),
      );
    mocked.removeOrphanWorktree
      .mockResolvedValueOnce("clean")
      .mockResolvedValueOnce("discarded");
    // Tras limpiar, el nuevo cálculo ya no propone nada.
    mocked.getRetentionPlan.mockResolvedValue(plan());

    await user.click(
      screen.getByRole("button", { name: "Limpiar lo seleccionado…" }),
    );
    await user.click(screen.getByRole("button", { name: "Limpiar" }));

    expect(
      await screen.findByRole("heading", { name: "Resultado de la limpieza" }),
    ).toBeInTheDocument();
    expect(mocked.cleanRun).toHaveBeenCalledTimes(2);
    expect(mocked.cleanRun).toHaveBeenNthCalledWith(1, {
      projectId: "p1",
      runId: "r1",
      output: true,
      worktree: true,
      discardWorktreeChanges: false,
      branch: false,
    });
    expect(mocked.cleanRun).toHaveBeenNthCalledWith(2, {
      projectId: "p2",
      runId: "r2",
      output: true,
      worktree: true,
      discardWorktreeChanges: true,
      branch: true,
    });
    expect(mocked.removeOrphanWorktree).toHaveBeenNthCalledWith(1, {
      path: "/datos/worktrees/p1/huerfano",
      discardChanges: false,
    });
    expect(mocked.removeOrphanWorktree).toHaveBeenNthCalledWith(2, {
      path: "/datos/worktrees/p1/sucio",
      discardChanges: true,
    });

    const results = screen
      .getAllByRole("listitem")
      .filter((li) => li.closest(".retention__results"));
    expect(results.map((li) => li.textContent)).toEqual([
      "✓ #7 Buscar recetas · intento #2: salida eliminada, worktree eliminado",
      "Error: #8 Buscar recetas · intento #1: El worktree cambió: vuelve a calcular.",
      "✓ Worktree sin intento /datos/worktrees/p1/huerfano: worktree eliminado",
      "✓ Worktree sin intento /datos/worktrees/p1/sucio: worktree descartado y eliminado",
    ]);
    expect(
      screen.getByText("3 elementos limpiados, 1 con error."),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Resultado de la limpieza" }),
    ).toHaveFocus();

    // Se recalcula y los resultados siguen visibles.
    await waitFor(() => {
      expect(mocked.getRetentionPlan).toHaveBeenCalledTimes(2);
    });
    expect(
      await screen.findByText(
        "No hay nada que limpiar con la antigüedad configurada.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Resultado de la limpieza" }),
    ).toBeInTheDocument();
  });
});

describe("Sistema → Ejecuciones: días de retención", () => {
  function settings(overrides: Partial<RunSettings> = {}): RunSettings {
    return {
      maxConcurrency: 2,
      timeoutMinutes: 30,
      claudeAcpPath: null,
      codexAcpPath: null,
      nodePath: null,
      approvalNotifications: false,
      retentionDays: 30,
      revision: 1,
      ...overrides,
    };
  }

  beforeEach(() => {
    mocked.getCodexHomeStatus.mockResolvedValue({
      path: "/datos/codex-home",
      signedIn: false,
    });
  });

  it("valida el rango en local, lo envía siempre y marca el cambio sin guardar", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(settings());
    mocked.updateRunSettings.mockResolvedValue(
      settings({ retentionDays: 90, revision: 2 }),
    );
    render(<RunSettingsPanel />);

    const days = await screen.findByLabelText(
      "Antigüedad para proponer limpieza (días)",
    );
    expect(days).toHaveValue(30);
    expect(days).toHaveAccessibleDescription(
      `Entre ${String(RETENTION_LIMITS.min)} y ${String(RETENTION_LIMITS.max)}. Al limpiar se proponen los intentos terminados hace más días que esto. Nada se borra solo.`,
    );

    await user.clear(days);
    await user.type(days, "3651");
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    expect(mocked.updateRunSettings).not.toHaveBeenCalled();
    expect(days).toHaveAttribute("aria-invalid", "true");
    expect(days).toHaveAccessibleDescription(/entre 1 y 3650\.$/);

    await user.clear(days);
    await user.type(days, "90");
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    expect(mocked.updateRunSettings).toHaveBeenCalledWith({
      expectedRevision: 1,
      maxConcurrency: 2,
      timeoutMinutes: 30,
      claudeAcpPath: null,
      codexAcpPath: null,
      nodePath: null,
      approvalNotifications: false,
      retentionDays: 90,
    });
    expect(
      await screen.findByText("✓ Configuración guardada."),
    ).toBeInTheDocument();
    expect(days).toHaveValue(90);
    await waitFor(() => {
      expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(false);
    });
  });

  it("un error del núcleo en `retentionDays` aparece junto al campo", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(settings());
    mocked.updateRunSettings.mockRejectedValue(
      validation("retentionDays", "Fuera de rango."),
    );
    render(<RunSettingsPanel />);
    const days = await screen.findByLabelText(
      "Antigüedad para proponer limpieza (días)",
    );
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    await waitFor(() => {
      expect(days).toHaveAttribute("aria-invalid", "true");
    });
    expect(days).toHaveAccessibleDescription(/Fuera de rango\.$/);
    expect(screen.queryByText(/No se pudo guardar/)).not.toBeInTheDocument();
  });
});

describe("Detalle del intento tras limpiar", () => {
  function worktree(overrides: Partial<RunWorktree> = {}): RunWorktree {
    return {
      runId: "r1",
      repositoryId: "g1",
      path: "/datos/worktrees/p1/r1",
      branch: "kanbania/h7-i1",
      baseCommit: "0123456789abcdef0123456789abcdef01234567",
      createdAt: "2026-09-01T10:00:00Z",
      removedAt: "2026-11-20T10:00:00Z",
      removal: "clean",
      branchDeletedAt: null,
      ...overrides,
    };
  }

  function detail(overrides: Partial<RunDetail> = {}): RunDetail {
    return {
      run: {
        id: "r1",
        projectId: "p1",
        storyId: "s1",
        number: 1,
        state: "completed",
        agentId: "a1",
        agentNumber: 3,
        agentName: "Jean Claude",
        createdAt: "2026-09-01T10:00:00Z",
        updatedAt: "2026-09-01T10:05:00Z",
        endedAt: "2026-09-01T10:05:00Z",
        endReason: "Terminó con código 0.",
        usage: { inputTokens: null, outputTokens: null, costUsdMicros: null },
        lastSequence: 3,
        snapshotSha256: "ab".repeat(32),
        startRequestedAt: "2026-09-01T10:00:00Z",
        cancelRequestedAt: null,
      },
      snapshot: {
        schemaVersion: 1,
        createdAt: "2026-09-01T10:00:00Z",
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
        repositories: [],
        instructions: "# Instrucciones",
        note: null,
      },
      snapshotIntact: true,
      events: [],
      eventsTruncated: false,
      startRequest: null,
      cancelReason: null,
      worktree: worktree(),
      approvals: [],
      outputRemovedAt: null,
      ...overrides,
    };
  }

  function renderDetail() {
    render(
      <RunDetailView
        projectId="p1"
        runId="r1"
        refreshSignal={0}
        focusOnLoad={null}
        onClose={vi.fn()}
        onStarted={vi.fn()}
        onChanged={vi.fn()}
      />,
    );
  }

  it("con la salida eliminada lo dice y no la lee; con la rama borrada lo indica", async () => {
    const removed = "2026-11-20T10:00:00Z";
    mocked.getRun.mockResolvedValue(
      detail({
        outputRemovedAt: removed,
        worktree: worktree({ branchDeletedAt: removed }),
      }),
    );
    renderDetail();
    expect(
      await screen.findByText(/La salida de este intento se eliminó/),
    ).toHaveTextContent(
      `La salida de este intento se eliminó al limpiar (${formatDateTime(removed)}); sus eventos se conservan.`,
    );
    expect(mocked.getRunOutput).not.toHaveBeenCalled();
    expect(screen.getByText("kanbania/h7-i1").closest("dd")).toHaveTextContent(
      `kanbania/h7-i1 · ya no existe (anotado al limpiar el ${formatDateTime(removed)})`,
    );
    expect(
      screen.getByText(/La rama del intento ya no existe/),
    ).toBeInTheDocument();
  });

  it("sin limpiar, la salida se lee como siempre y la rama se conserva", async () => {
    mocked.getRun.mockResolvedValue(detail());
    mocked.getRunOutput.mockResolvedValue({
      lines: [],
      skippedCorrupt: 0,
      earlierDropped: false,
    });
    renderDetail();
    expect(await screen.findByText("Sin salida todavía.")).toBeInTheDocument();
    expect(screen.queryByText(/se eliminó al limpiar/)).not.toBeInTheDocument();
    expect(
      screen.queryByText(/ya no existe \(anotado/),
    ).not.toBeInTheDocument();
  });
});
