import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent } from "../../contracts/generated/Agent";
import type { AgentRef } from "../../contracts/generated/AgentRef";
import type { ErrorPayload } from "../../contracts/generated/ErrorPayload";
import type { RunDetail } from "../../contracts/generated/RunDetail";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import type { StoryRelations } from "../../contracts/generated/StoryRelations";
import * as commands from "../../shared/ipc/commands";
import { RUNNER_NOTICE, RunsPanel } from "./RunsPanel";

// Solo se simulan las llamadas IPC; errores y helpers son los reales.
vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  listStoryRuns: vi.fn(),
  createRun: vi.fn(),
  cancelRun: vi.fn(),
  getRun: vi.fn(),
  getStoryRelations: vi.fn(),
  listAgents: vi.fn(),
}));

const mocked = vi.mocked(commands);

const AVATAR = {
  head: 0,
  skin: 0,
  eyes: 0,
  mouth: 0,
  hair: 0,
  hairColor: 0,
  outfit: 0,
  outfitColor: 0,
  accessory: 0,
  background: 0,
} as unknown as Agent["avatar"];

const jean: AgentRef = {
  id: "a1",
  number: 3,
  name: "Jean Claude",
  role: "Implementa",
  avatar: AVATAR,
};

function agent(id: string, number: number, name: string): Agent {
  return {
    id,
    number,
    name,
    role: "Rol",
    provider: "Anthropic",
    model: "claude",
    capabilities: [],
    avatar: AVATAR,
    assignedStories: 0,
    revision: 1,
  };
}

function relations(assigned: AgentRef | null): StoryRelations {
  return {
    storyId: "s1",
    epic: null,
    agent: assigned,
    parent: null,
    children: [],
    prerequisites: [],
    dependents: [],
    pendingPrerequisites: 0,
  };
}

const NO_USAGE = { inputTokens: null, outputTokens: null, costUsdMicros: null };

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

function detail(overrides: Partial<RunDetail> = {}): RunDetail {
  return {
    run: summary(),
    snapshot: {
      schemaVersion: 1,
      createdAt: "2026-11-12T10:00:00Z",
      story: {
        id: "s1",
        number: 7,
        title: "Título congelado",
        description: "Descripción **congelada**",
        priority: "high",
        revision: 4,
        column: { name: "Listo", category: "ready" },
      },
      criteria: [
        { number: 1, text: "Descarga un archivo" },
        { number: 3, text: "Avisa si falla" },
      ],
      epic: { number: 2, title: "Exportaciones", description: "" },
      parent: { id: "s0", number: 5, title: "Padre", accepted: false },
      prerequisites: [{ id: "s2", number: 4, title: "Base", accepted: true }],
      agent: {
        id: "a1",
        number: 3,
        name: "Jean Claude",
        role: "Implementa",
        provider: "Anthropic",
        model: "claude-opus",
        capabilities: ["implement", "test"],
      },
      repositories: [
        {
          id: "g1",
          path: "/repos/app",
          head: "0123456789abcdef0123456789abcdef01234567",
          branch: "main",
        },
        { id: "g2", path: "/repos/sin-git", head: null, branch: null },
      ],
      instructions:
        '# Instrucciones\n\nTrabaja en la historia.\n\n<script>alert("x")</script>\n\n[enlace](javascript:alert(1))',
      note: "Revisa *primero* los tests.",
    },
    snapshotIntact: true,
    events: [
      {
        sequence: 1,
        at: "2026-11-12T10:00:00Z",
        event: { kind: "state_changed", to: "queued", reason: null },
      },
      {
        sequence: 2,
        at: "2026-11-12T10:05:00Z",
        event: {
          kind: "usage_reported",
          inputTokens: 1200,
          outputTokens: null,
          costUsdMicros: null,
        },
      },
      {
        sequence: 3,
        at: "2026-11-12T10:06:00Z",
        event: {
          kind: "state_changed",
          to: "cancelled",
          reason: "Cancelado por una persona.",
        },
      },
    ],
    eventsTruncated: false,
    startRequest: null,
    cancelReason: null,
    worktree: null,
    approvals: [],
    ...overrides,
  };
}

function validation(field: string, message: string): ErrorPayload {
  return { code: "validation", message, recoverable: true, field };
}

function renderPanel(props: Partial<Parameters<typeof RunsPanel>[0]> = {}) {
  const onDirtyChange = vi.fn();
  const view = render(
    <RunsPanel
      projectId="p1"
      storyId="s1"
      closed={null}
      onDirtyChange={onDirtyChange}
      {...props}
    />,
  );
  return { ...view, onDirtyChange };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocked.listStoryRuns.mockResolvedValue([]);
  mocked.getStoryRelations.mockResolvedValue(relations(jean));
  mocked.listAgents.mockResolvedValue([
    agent("a1", 3, "Jean Claude"),
    agent("a2", 4, "Robo"),
  ]);
  mocked.getRun.mockResolvedValue(detail());
});

describe("RunsPanel", () => {
  it("sin intentos lo dice y avisa de que el ejecutor es un proceso de prueba sin IA", async () => {
    renderPanel();

    expect(
      await screen.findByText("Esta historia no tiene intentos todavía."),
    ).toBeInTheDocument();
    expect(mocked.listStoryRuns).toHaveBeenCalledWith("p1", "s1");
    expect(screen.getByText(RUNNER_NOTICE)).toBeInTheDocument();
    expect(RUNNER_NOTICE).toContain("proceso de prueba (sin IA)");
    expect(RUNNER_NOTICE).toContain("no acepta la historia");
  });

  it("muestra cada intento por separado, con su estado en texto", async () => {
    mocked.listStoryRuns.mockResolvedValue([
      summary({ id: "r3", number: 3, state: "queued" }),
      summary({
        id: "r2",
        number: 2,
        state: "cancelled",
        agentNumber: 4,
        agentName: "Robo",
        endedAt: "2026-11-11T09:00:00Z",
        endReason: "La historia se archivó.",
      }),
      summary({ id: "r1", number: 1, state: "interrupted" }),
    ]);
    renderPanel();

    const list = await screen.findByRole("list", {
      name: "Intentos, del más reciente al más antiguo",
    });
    const items = within(list).getAllByRole("listitem");
    expect(items).toHaveLength(3);

    const [third, second, first] = items as [
      HTMLElement,
      HTMLElement,
      HTMLElement,
    ];
    expect(third).toHaveTextContent("Intento #3");
    expect(within(third).getByText("En cola")).toBeInTheDocument();
    expect(third).toHaveTextContent("A-3 Jean Claude");
    expect(third).toHaveTextContent("En cola: no se ha iniciado.");
    expect(
      within(third).getByRole("button", { name: "Iniciar… el intento #3" }),
    ).toBeInTheDocument();
    // El ejecutor se elige al iniciar: la lista no da uno por supuesto.
    expect(third).not.toHaveTextContent("Proceso de prueba (sin IA)");
    expect(
      within(third).getByRole("button", { name: "Cancelar intento…" }),
    ).toBeInTheDocument();
    expect(third).not.toHaveTextContent("Terminado");

    expect(second).toHaveTextContent("Intento #2");
    expect(within(second).getByText("Cancelado")).toBeInTheDocument();
    expect(second).toHaveTextContent("A-4 Robo");
    expect(second).toHaveTextContent("Terminado");
    expect(second).toHaveTextContent("Motivo");
    expect(second).toHaveTextContent("La historia se archivó.");
    // Un intento terminado no se puede cancelar.
    expect(
      within(second).queryByRole("button", { name: /Cancelar/ }),
    ).not.toBeInTheDocument();

    expect(within(first).getByText("Interrumpido")).toBeInTheDocument();
  });

  it("el uso no informado se muestra «no disponible», nunca 0", async () => {
    mocked.listStoryRuns.mockResolvedValue([
      summary({ id: "r2", number: 2 }),
      summary({
        id: "r1",
        number: 1,
        state: "completed",
        usage: {
          inputTokens: 12345,
          outputTokens: null,
          costUsdMicros: 1_500_000,
        },
      }),
    ]);
    renderPanel();

    const items = within(
      await screen.findByRole("list", { name: /^Intentos/ }),
    ).getAllByRole("listitem");
    const [unknown, known] = items as [HTMLElement, HTMLElement];

    const facts = (item: HTMLElement) =>
      within(item)
        .getAllByRole("definition")
        .map((dd) => dd.textContent);
    expect(facts(unknown).filter((t) => t === "no disponible")).toHaveLength(3);
    for (const text of facts(unknown)) expect(text).not.toMatch(/^0/);
    expect(unknown).not.toHaveTextContent(/\b0\b(?![:\d])/);

    expect(known).toHaveTextContent("12.345");
    expect(known).toHaveTextContent(/1,50\s*US\$/);
    // Solo la salida falta.
    expect(facts(known).filter((t) => t === "no disponible")).toHaveLength(1);
  });

  it("prepara un intento con el agente asignado por defecto y la nota", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns
      .mockResolvedValueOnce([])
      .mockResolvedValue([summary({ id: "r9", number: 1 })]);
    mocked.createRun.mockResolvedValue(summary({ id: "r9", number: 1 }));
    mocked.getRun.mockResolvedValue(
      detail({ run: summary({ id: "r9", number: 1 }) }),
    );
    const { onDirtyChange } = renderPanel();

    const select = await screen.findByLabelText("Agente del intento");
    await waitFor(() => {
      expect(
        within(select).getByRole("option", {
          name: "Asignado a la historia: A-3 Jean Claude",
        }),
      ).toBeInTheDocument();
    });
    expect(select).toHaveValue("");
    // El asignado no se repite en la lista; el resto del equipo, sí.
    expect(within(select).getAllByRole("option")).toHaveLength(2);
    expect(
      within(select).getByRole("option", { name: "A-4 Robo" }),
    ).toBeInTheDocument();

    const note = screen.getByLabelText("Nota para el agente (opcional)");
    await user.type(note, "Empieza por los tests");
    expect(
      document.getElementById(
        (note.getAttribute("aria-describedby") ?? "").split(" ")[1] ?? "",
      ),
    ).toHaveTextContent(/^21 \/ 4000 caracteres$/);
    expect(onDirtyChange).toHaveBeenLastCalledWith(true);

    await user.click(screen.getByRole("button", { name: "Preparar intento" }));

    expect(mocked.createRun).toHaveBeenCalledTimes(1);
    const input = mocked.createRun.mock.calls[0]?.[0];
    expect(input).toEqual({
      requestId: expect.any(String) as string,
      projectId: "p1",
      storyId: "s1",
      note: "Empieza por los tests",
    });
    expect(input).not.toHaveProperty("agentId");
    expect(
      await screen.findByText(
        /Intento #1 preparado: queda en cola hasta que lo inicies\./,
      ),
    ).toBeInTheDocument();
    expect(note).toHaveValue("");
    expect(onDirtyChange).toHaveBeenLastCalledWith(false);
    // Lista recargada y detalle del nuevo intento abierto.
    expect(
      await screen.findByRole("heading", { name: "Detalle del intento #1" }),
    ).toBeInTheDocument();
    expect(mocked.getRun).toHaveBeenCalledWith("p1", "r9");
    expect(mocked.listStoryRuns).toHaveBeenCalledTimes(2);
  });

  it("puede elegir otro agente del equipo", async () => {
    const user = userEvent.setup();
    mocked.createRun.mockResolvedValue(summary({ agentId: "a2" }));
    renderPanel();

    const select = await screen.findByLabelText("Agente del intento");
    await screen.findByRole("option", { name: "A-4 Robo" });
    await user.selectOptions(select, "a2");
    await user.click(screen.getByRole("button", { name: "Preparar intento" }));

    expect(mocked.createRun.mock.calls[0]?.[0]).toMatchObject({
      agentId: "a2",
    });
    expect(mocked.createRun.mock.calls[0]?.[0]).not.toHaveProperty("note");
  });

  it("sin agente asignado lo indica y muestra el error junto al selector", async () => {
    const user = userEvent.setup();
    mocked.getStoryRelations.mockResolvedValue(relations(null));
    mocked.createRun.mockRejectedValue(
      validation(
        "agentId",
        "Asigna un agente a la historia o elige uno para el intento.",
      ),
    );
    renderPanel();

    await screen.findByRole("option", {
      name: "Asignado a la historia (no hay ninguno)",
    });
    await user.click(screen.getByRole("button", { name: "Preparar intento" }));

    const select = screen.getByLabelText("Agente del intento");
    const error = await screen.findByText(
      "Asigna un agente a la historia o elige uno para el intento.",
    );
    expect(select).toHaveAttribute("aria-invalid", "true");
    expect(select.getAttribute("aria-describedby")).toBe(error.id);
  });

  it("un error de la nota aparece junto al campo y el reintento repite el requestId", async () => {
    const user = userEvent.setup();
    mocked.createRun.mockRejectedValue(
      validation(
        "note",
        "La nota contiene algo con forma de credencial de Kanban IA (kbia_…).",
      ),
    );
    renderPanel();

    const note = await screen.findByLabelText("Nota para el agente (opcional)");
    await user.type(note, "usa kbia_secreto");
    const submit = screen.getByRole("button", { name: "Preparar intento" });
    await user.click(submit);

    const error = await screen.findByText(/forma de credencial/);
    expect(error).toHaveAttribute("role", "alert");
    expect(note).toHaveAttribute("aria-invalid", "true");
    expect(note.getAttribute("aria-describedby")).toContain(error.id);
    // No es una alerta general.
    expect(
      screen.queryByText(/No se pudo preparar el intento/),
    ).not.toBeInTheDocument();

    await user.click(submit);
    const [first, second] = mocked.createRun.mock.calls.map(
      ([input]) => input.requestId,
    );
    expect(second).toBe(first);

    // Editar es otro envío: nuevo requestId y el error desaparece.
    await user.type(note, "!");
    expect(screen.queryByText(/forma de credencial/)).not.toBeInTheDocument();
    await user.click(submit);
    expect(mocked.createRun.mock.calls[2]?.[0].requestId).not.toBe(first);
  });

  it("una nota de más de 4 000 caracteres no se envía", async () => {
    const user = userEvent.setup();
    renderPanel();

    const note = await screen.findByLabelText("Nota para el agente (opcional)");
    await user.click(note);
    await user.paste("x".repeat(4001));

    expect(screen.getByText(/sobran 1\./)).toBeInTheDocument();
    expect(note).toHaveAttribute("aria-invalid", "true");
    expect(
      screen.getByRole("button", { name: "Preparar intento" }),
    ).toBeDisabled();
    expect(mocked.createRun).not.toHaveBeenCalled();
  });

  it("con un intento activo, la alerta ofrece verlo", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns
      .mockResolvedValueOnce([])
      .mockResolvedValue([summary({ id: "r2", number: 2 })]);
    mocked.createRun.mockRejectedValue({
      code: "run_active",
      message:
        "La historia ya tiene un intento en curso (Intento #2). Espera a que termine o cancélalo antes de preparar otro.",
      recoverable: true,
    } satisfies ErrorPayload);
    mocked.getRun.mockResolvedValue(
      detail({ run: summary({ id: "r2", number: 2 }) }),
    );
    renderPanel();

    await screen.findByText("Esta historia no tiene intentos todavía.");
    await user.click(screen.getByRole("button", { name: "Preparar intento" }));

    const form = screen.getByRole("form", { name: "Preparar intento" });
    const alert = await within(form).findByRole("alert");
    expect(alert).toHaveTextContent("ya tiene un intento en curso");
    // La lista se recargó y la alerta ofrece abrir el intento activo.
    await user.click(
      await within(alert).findByRole("button", {
        name: "Ver el intento #2 activo",
      }),
    );
    expect(
      await screen.findByRole("heading", { name: "Detalle del intento #2" }),
    ).toHaveFocus();
    expect(mocked.getRun).toHaveBeenCalledWith("p1", "r2");
  });

  it("los rechazos de la historia (bloqueada, prerequisitos) son una alerta general", async () => {
    const user = userEvent.setup();
    mocked.createRun.mockRejectedValue(
      validation(
        "storyId",
        "La historia #7 está bloqueada: quita el bloqueo antes de preparar un intento.",
      ),
    );
    renderPanel();

    await user.click(
      await screen.findByRole("button", { name: "Preparar intento" }),
    );
    const alert = await screen.findByText(/No se pudo preparar el intento/);
    expect(alert).toHaveTextContent("quita el bloqueo");
    expect(
      screen.getByLabelText("Nota para el agente (opcional)"),
    ).not.toHaveAttribute("aria-invalid");
  });

  it("archivada o aceptada no ofrece preparar intentos", async () => {
    mocked.listStoryRuns.mockResolvedValue([
      summary({ state: "cancelled", endReason: "La historia se aceptó." }),
    ]);
    const { rerender } = renderPanel({ closed: "accepted" });

    expect(
      await screen.findByText(/está aceptada: reábrela/),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Preparar intento" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("La historia se aceptó.")).toBeInTheDocument();

    rerender(
      <RunsPanel
        projectId="p1"
        storyId="s1"
        closed="archived"
        onDirtyChange={vi.fn()}
      />,
    );
    expect(screen.getByText(/está archivada: restáurala/)).toBeInTheDocument();
  });

  it("el detalle muestra la instantánea con Markdown seguro, criterios, agente y eventos", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns.mockResolvedValue([summary()]);
    const { container } = renderPanel();

    await user.click(
      await screen.findByRole("button", { name: "Ver detalle del intento #1" }),
    );
    const heading = await screen.findByRole("heading", {
      name: "Detalle del intento #1",
    });
    const region = screen.getByRole("region", {
      name: "Detalle del intento #1",
    });
    expect(heading).toHaveFocus();

    expect(region).toHaveTextContent(
      /Instantánea congelada al preparar el intento \(.+\)/,
    );
    expect(region).toHaveTextContent("editar la historia después no la cambia");
    expect(within(region).getByText("Título congelado")).toBeInTheDocument();
    expect(within(region).getByText("congelada").tagName).toBe("STRONG");
    expect(region).toHaveTextContent("CA-1: Descarga un archivo");
    expect(region).toHaveTextContent("CA-3: Avisa si falla");
    expect(region).toHaveTextContent("E-2 Exportaciones");
    expect(region).toHaveTextContent("#5 Padre (sin aceptar)");
    expect(region).toHaveTextContent("#4 Base (aceptada)");
    expect(region).toHaveTextContent("Implementa");
    expect(region).toHaveTextContent("Anthropic");
    expect(region).toHaveTextContent("claude-opus");
    expect(region).toHaveTextContent("Implementar, Probar");
    expect(region).toHaveTextContent(
      "0123456789abcdef0123456789abcdef01234567",
    );
    expect(region).toHaveTextContent("Rama: main");
    expect(region).toHaveTextContent(
      "Commit: no disponible · Rama: no disponible",
    );
    expect(within(region).getByText("primero").tagName).toBe("EM");
    // Instrucciones: Markdown renderizado sin HTML crudo ni enlaces navegables.
    expect(
      within(region).getByRole("heading", { name: "Instrucciones" }),
    ).toBeInTheDocument();
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("a")).toBeNull();
    expect(region).not.toHaveTextContent('alert("x")');

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    const events = within(region).getByRole("list", {
      name: "Eventos en orden",
    });
    const rows = within(events).getAllByRole("listitem");
    expect(rows.map((r) => r.textContent)).toEqual([
      expect.stringMatching(/^1\. .*Estado → En cola$/),
      expect.stringMatching(
        /^2\. .*Uso reportado \(tokens de entrada: 1200; tokens de salida: no disponible; coste: no disponible\)$/,
      ),
      expect.stringMatching(
        /^3\. .*Estado → Cancelado: Cancelado por una persona\.$/,
      ),
    ]);
    expect(region).not.toHaveTextContent("eventos: los anteriores");
  });

  it("avisa si la instantánea no coincide con su huella y si faltan eventos", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns.mockResolvedValue([summary()]);
    mocked.getRun.mockResolvedValue(
      detail({ snapshotIntact: false, eventsTruncated: true }),
    );
    renderPanel();

    await user.click(
      await screen.findByRole("button", { name: "Ver detalle del intento #1" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "La instantánea no coincide con su huella: no es fiable.",
    );
    expect(
      screen.getByText(/Se muestran los últimos 3 eventos/),
    ).toBeInTheDocument();
  });

  it("se maneja con teclado: abrir y cerrar el detalle devuelve el foco", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns.mockResolvedValue([summary()]);
    renderPanel();

    const open = await screen.findByRole("button", {
      name: "Ver detalle del intento #1",
    });
    open.focus();
    await user.keyboard("{Enter}");
    expect(open).toHaveAttribute("aria-expanded", "true");
    expect(
      await screen.findByRole("heading", { name: "Detalle del intento #1" }),
    ).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(
      screen.queryByRole("heading", { name: /Detalle del intento/ }),
    ).not.toBeInTheDocument();
    const again = screen.getByRole("button", {
      name: "Ver detalle del intento #1",
    });
    expect(again).toHaveFocus();
    expect(again).toHaveAttribute("aria-expanded", "false");

    await user.keyboard(" ");
    await user.click(
      await screen.findByRole("button", { name: "Cerrar detalle" }),
    );
    expect(
      screen.getByRole("button", { name: "Ver detalle del intento #1" }),
    ).toHaveFocus();
  });

  it("cancelar pide confirmación; Escape la cierra sin cancelar", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns.mockResolvedValue([summary()]);
    renderPanel();

    const open = await screen.findByRole("button", {
      name: "Cancelar intento…",
    });
    await user.click(open);
    const dialog = screen.getByRole("alertdialog");
    expect(dialog).toHaveTextContent("¿Cancelar el intento #1?");
    expect(
      within(dialog).getByRole("button", { name: "Mantener" }),
    ).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(open).toHaveFocus();
    expect(mocked.cancelRun).not.toHaveBeenCalled();
  });

  it("cancelar llama al comando, repite el requestId al reintentar y recarga", async () => {
    const user = userEvent.setup();
    const cancelled = summary({
      state: "cancelled",
      endedAt: "2026-11-12T11:00:00Z",
      endReason: "Cancelado por una persona.",
    });
    mocked.listStoryRuns
      .mockResolvedValueOnce([summary()])
      .mockResolvedValue([cancelled]);
    mocked.cancelRun
      .mockRejectedValueOnce({
        code: "storage_unavailable",
        message: "El almacenamiento no está disponible.",
        recoverable: true,
      } satisfies ErrorPayload)
      .mockResolvedValue(cancelled);
    renderPanel();

    await user.click(
      await screen.findByRole("button", { name: "Cancelar intento…" }),
    );
    const dialog = screen.getByRole("alertdialog");
    await user.click(
      within(dialog).getByRole("button", { name: "Cancelar intento" }),
    );
    expect(
      await within(dialog).findByText("El almacenamiento no está disponible."),
    ).toHaveAttribute("role", "alert");

    await user.click(
      within(dialog).getByRole("button", { name: "Cancelar intento" }),
    );
    const [first, second] = mocked.cancelRun.mock.calls.map(([i]) => i);
    expect(first).toEqual({
      requestId: expect.any(String) as string,
      projectId: "p1",
      runId: "r1",
    });
    expect(second?.requestId).toBe(first?.requestId);

    expect(
      await screen.findByText(/Intento #1 cancelado\./),
    ).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Intentos" })).toHaveFocus();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(await screen.findByText("Cancelado")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Cancelar intento…" }),
    ).not.toBeInTheDocument();
  });

  it("recarga lista y detalle cuando cambia la señal (aceptar o archivar la historia)", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns
      .mockResolvedValueOnce([summary()])
      .mockResolvedValue([
        summary({ state: "cancelled", endReason: "La historia se archivó." }),
      ]);
    const onDirtyChange = vi.fn();
    const { rerender } = render(
      <RunsPanel
        projectId="p1"
        storyId="s1"
        closed={null}
        refreshSignal={0}
        onDirtyChange={onDirtyChange}
      />,
    );
    await user.click(
      await screen.findByRole("button", { name: "Ver detalle del intento #1" }),
    );
    await screen.findByRole("heading", { name: "Detalle del intento #1" });

    rerender(
      <RunsPanel
        projectId="p1"
        storyId="s1"
        closed="archived"
        refreshSignal={1}
        onDirtyChange={onDirtyChange}
      />,
    );
    expect(
      await screen.findByText("La historia se archivó."),
    ).toBeInTheDocument();
    expect(mocked.listStoryRuns).toHaveBeenCalledTimes(2);
    expect(mocked.getRun).toHaveBeenCalledTimes(2);
    expect(mocked.getStoryRelations).toHaveBeenCalledTimes(2);
  });

  it("si la lista no carga, lo dice y permite reintentar", async () => {
    const user = userEvent.setup();
    mocked.listStoryRuns.mockRejectedValueOnce({
      code: "internal",
      message: "Error interno.",
      recoverable: true,
    } satisfies ErrorPayload);
    renderPanel();

    expect(
      await screen.findByText(/No se pudieron cargar los intentos/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reintentar" }));
    expect(
      await screen.findByText("Esta historia no tiene intentos todavía."),
    ).toBeInTheDocument();
  });
});
