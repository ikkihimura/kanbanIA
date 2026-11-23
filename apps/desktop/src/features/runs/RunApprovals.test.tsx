// HU-020: peticiones de permiso del agente en el detalle del intento, eventos y la casilla de
// notificaciones en «Sistema → Ejecuciones».
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ErrorPayload } from "../../contracts/generated/ErrorPayload";
import type { RunApproval } from "../../contracts/generated/RunApproval";
import type { RunDetail } from "../../contracts/generated/RunDetail";
import type { RunSettings } from "../../contracts/generated/RunSettings";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import * as commands from "../../shared/ipc/commands";
import { formatTime } from "../../shared/ui/format";
import { RunDetailView } from "./RunDetailView";
import { RunSettingsPanel } from "./RunSettingsPanel";

// Solo se simulan las llamadas IPC; errores y helpers son los reales.
vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  getRun: vi.fn(),
  getRunOutput: vi.fn(),
  decideApproval: vi.fn(),
  getRunSettings: vi.fn(),
  updateRunSettings: vi.fn(),
  checkAcpAdapter: vi.fn(),
  getCodexHomeStatus: vi.fn(),
  setUnsavedChanges: vi.fn(),
}));

const mocked = vi.mocked(commands);

const FUTURE = "2099-11-12T10:10:00Z";
const PAST = "2020-11-12T10:10:00Z";
const SCOPE = "rm -rf build && cargo test";

function summary(overrides: Partial<RunSummary> = {}): RunSummary {
  return {
    id: "r1",
    projectId: "p1",
    storyId: "s1",
    number: 1,
    state: "waiting_approval",
    agentId: "a1",
    agentNumber: 3,
    agentName: "Jean Claude",
    createdAt: "2026-11-12T10:00:00Z",
    updatedAt: "2026-11-12T10:00:00Z",
    endedAt: null,
    endReason: null,
    usage: { inputTokens: null, outputTokens: null, costUsdMicros: null },
    lastSequence: 3,
    snapshotSha256: "ab".repeat(32),
    startRequestedAt: "2026-11-12T10:00:00Z",
    cancelRequestedAt: null,
    ...overrides,
  };
}

function approval(overrides: Partial<RunApproval> = {}): RunApproval {
  return {
    id: "ap1",
    runId: "r1",
    sequence: 1,
    toolTitle: "Ejecutar tests",
    toolKind: "execute",
    scope: SCOPE,
    canAllow: true,
    state: "pending",
    requestedAt: "2026-11-12T10:00:00Z",
    expiresAt: FUTURE,
    decidedAt: null,
    delivered: false,
    endReason: null,
    ...overrides,
  };
}

function detail(
  approvals: RunApproval[],
  overrides: Partial<RunDetail> = {},
): RunDetail {
  return {
    run: summary(),
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
    approvals,
    outputRemovedAt: null,
    ...overrides,
  };
}

function renderDetail(onChanged = vi.fn()) {
  render(
    <RunDetailView
      projectId="p1"
      runId="r1"
      refreshSignal={0}
      focusOnLoad={null}
      onClose={vi.fn()}
      onStarted={vi.fn()}
      onChanged={onChanged}
    />,
  );
  return { onChanged };
}

async function permissions() {
  return screen.findByRole("region", { name: /^Permisos del agente/ });
}

beforeEach(() => {
  vi.clearAllMocks();
  mocked.getCodexHomeStatus.mockResolvedValue({
    path: "/datos/codex-home",
    signedIn: false,
  });
  mocked.getRunOutput.mockResolvedValue({
    lines: [],
    skippedCorrupt: 0,
    earlierDropped: false,
  });
});

describe("Permisos del agente en el detalle", () => {
  it("una pendiente muestra acción, tipo, alcance en texto plano, plazo y los dos botones", async () => {
    mocked.getRun.mockResolvedValue(
      detail([approval({ scope: "<b>no es HTML</b>\nsegunda línea" })]),
    );
    renderDetail();

    const section = await permissions();
    expect(section).toHaveAccessibleName(
      "Permisos del agente (1 espera tu decisión)",
    );
    const item = within(section).getByRole("listitem");
    expect(
      within(item).getByRole("heading", {
        name: "Permiso #1: Ejecutar tests",
      }),
    ).toBeInTheDocument();
    expect(item).toHaveTextContent("Espera tu decisión");
    expect(item).toHaveTextContent("Tipo (según el agente)execute");
    // Texto plano: las etiquetas no se interpretan.
    const scope = within(item).getByText(/<b>no es HTML<\/b>/);
    expect(scope.tagName).toBe("PRE");
    expect(item.querySelector("b")).toBeNull();
    expect(section).toHaveTextContent(
      "La acción y el alcance los informa el agente: la app no los comprueba.",
    );
    expect(item).toHaveTextContent(`caduca a las ${formatTime(FUTURE)}`);

    // El botón de permitir describe la acción y su alcance.
    const allow = within(item).getByRole("button", {
      name: "Permitir una vez",
    });
    expect(allow).toHaveAccessibleDescription(
      /Permiso #1: Ejecutar tests.*<b>no es HTML<\/b>/,
    );
    expect(
      within(item).getByRole("button", { name: "Rechazar" }),
    ).toBeEnabled();
    // Nunca «permitir siempre».
    expect(screen.queryByText(/siempre/i)).not.toBeInTheDocument();

    // Con una pendiente, la sección va antes que «Ejecución».
    const execution = screen.getByRole("region", { name: "Ejecución" });
    expect(
      section.compareDocumentPosition(execution) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("sin alcance lo dice, y la descripción del botón también", async () => {
    mocked.getRun.mockResolvedValue(
      detail([approval({ scope: null, toolKind: null })]),
    );
    renderDetail();

    const item = within(await permissions()).getByRole("listitem");
    expect(item).toHaveTextContent("(el agente no indicó el alcance)");
    expect(item).not.toHaveTextContent("Tipo (según el agente)");
    expect(
      within(item).getByRole("button", { name: "Permitir una vez" }),
    ).toHaveAccessibleDescription(/\(el agente no indicó el alcance\)/);
  });

  it("«Permitir una vez» envía allow_once y recarga el detalle", async () => {
    const user = userEvent.setup();
    mocked.getRun.mockResolvedValueOnce(detail([approval()])).mockResolvedValue(
      detail([
        approval({
          state: "allowed",
          decidedAt: "2026-11-12T10:02:00Z",
          delivered: true,
        }),
      ]),
    );
    mocked.decideApproval.mockResolvedValue(approval({ state: "allowed" }));
    const { onChanged } = renderDetail();

    const item = within(await permissions()).getByRole("listitem");
    await user.click(
      within(item).getByRole("button", { name: "Permitir una vez" }),
    );

    expect(mocked.decideApproval).toHaveBeenCalledWith({
      requestId: expect.any(String) as string,
      projectId: "p1",
      approvalId: "ap1",
      decision: "allow_once",
    });
    await waitFor(() => {
      expect(item).toHaveTextContent("Permitido una vez");
    });
    expect(mocked.getRun).toHaveBeenCalledTimes(2);
    expect(onChanged).toHaveBeenCalled();
    expect(within(item).queryByRole("button")).not.toBeInTheDocument();
    // El foco no se pierde: va al encabezado de la petición.
    expect(
      within(item).getByRole("heading", { name: "Permiso #1: Ejecutar tests" }),
    ).toHaveFocus();
  });

  it("«Rechazar» envía reject; reintentar tras un fallo repite el requestId", async () => {
    const user = userEvent.setup();
    mocked.getRun.mockResolvedValue(detail([approval()]));
    const internal: ErrorPayload = {
      code: "internal",
      message: "Error interno.",
      recoverable: true,
    };
    mocked.decideApproval
      .mockRejectedValueOnce(internal)
      .mockResolvedValue(approval({ state: "rejected" }));
    renderDetail();

    const item = within(await permissions()).getByRole("listitem");
    const reject = within(item).getByRole("button", { name: "Rechazar" });
    await user.click(reject);
    expect(
      await within(item).findByText(
        "No se pudo registrar la decisión: Error interno.",
      ),
    ).toBeInTheDocument();
    await user.click(reject);

    expect(mocked.decideApproval).toHaveBeenCalledTimes(2);
    const [first, second] = mocked.decideApproval.mock.calls;
    expect(first?.[0].decision).toBe("reject");
    expect(second?.[0]).toEqual(first?.[0]);
  });

  it("approval_closed muestra el mensaje del núcleo y recarga", async () => {
    const user = userEvent.setup();
    mocked.getRun.mockResolvedValueOnce(detail([approval()])).mockResolvedValue(
      detail([
        approval({
          state: "withdrawn",
          decidedAt: "2026-11-12T10:03:00Z",
          endReason: "El agente canceló la petición.",
        }),
      ]),
    );
    const closed: ErrorPayload = {
      code: "approval_closed",
      message: "La petición ya no admite decisión.",
      recoverable: false,
    };
    mocked.decideApproval.mockRejectedValue(closed);
    renderDetail();

    const item = within(await permissions()).getByRole("listitem");
    await user.click(
      within(item).getByRole("button", { name: "Permitir una vez" }),
    );

    expect(await within(item).findByRole("alert")).toHaveTextContent(
      "La petición ya no admite decisión.",
    );
    await waitFor(() => {
      expect(item).toHaveTextContent(
        "Retirado por el agente El agente canceló la petición.",
      );
    });
    expect(within(item).queryByRole("button")).not.toBeInTheDocument();
    expect(mocked.getRun).toHaveBeenCalledTimes(2);
  });

  it("si el agente no ofreció permitir, solo se puede rechazar", async () => {
    mocked.getRun.mockResolvedValue(detail([approval({ canAllow: false })]));
    renderDetail();

    const item = within(await permissions()).getByRole("listitem");
    expect(
      within(item).queryByRole("button", { name: "Permitir una vez" }),
    ).not.toBeInTheDocument();
    expect(item).toHaveTextContent(
      "El agente no ofreció permitir esta acción: solo puedes rechazarla.",
    );
    expect(
      within(item).getByRole("button", { name: "Rechazar" }),
    ).toBeInTheDocument();
  });

  it("una pendiente con el plazo vencido no admite decidir", async () => {
    mocked.getRun.mockResolvedValue(detail([approval({ expiresAt: PAST })]));
    renderDetail();

    const section = await permissions();
    expect(section).toHaveAccessibleName("Permisos del agente");
    const item = within(section).getByRole("listitem");
    expect(item).toHaveTextContent("Caducada: la app responderá «rechazar»");
    expect(item).toHaveTextContent(`caducó a las ${formatTime(PAST)}`);
    expect(within(item).queryByRole("button")).not.toBeInTheDocument();
  });

  it("los estados cerrados se muestran con su texto y sin botones", async () => {
    const closed: [RunApproval["state"], string][] = [
      ["allowed", "Permitido una vez"],
      ["rejected", "Rechazado"],
      ["expired", "Caducó (se respondió rechazar)"],
      ["cancelled", "Cancelado"],
      ["withdrawn", "Retirado por el agente"],
    ];
    mocked.getRun.mockResolvedValue(
      detail(
        closed.map(([state], i) =>
          approval({
            id: `ap${String(i + 1)}`,
            sequence: i + 1,
            state,
            decidedAt: "2026-11-12T10:05:00Z",
            endReason: state === "cancelled" ? "Se canceló el intento." : null,
          }),
        ),
        {
          run: summary({
            state: "cancelled",
            endedAt: "2026-11-12T10:05:00Z",
          }),
        },
      ),
    );
    renderDetail();

    const section = await permissions();
    const items = within(section).getAllByRole("listitem");
    expect(items).toHaveLength(closed.length);
    closed.forEach(([, label], i) => {
      expect(items[i]).toHaveTextContent(label);
    });
    expect(items[3]).toHaveTextContent("Cancelado Se canceló el intento.");
    expect(within(section).queryByRole("button")).not.toBeInTheDocument();
    expect(section).toHaveTextContent("Cerrada");
  });

  it("sin peticiones no hay sección", async () => {
    mocked.getRun.mockResolvedValue(detail([]));
    renderDetail();
    await screen.findByRole("region", { name: "Ejecución" });
    expect(
      screen.queryByRole("region", { name: /^Permisos del agente/ }),
    ).not.toBeInTheDocument();
  });

  it("los eventos de permisos se describen en la lista de eventos", async () => {
    mocked.getRun.mockResolvedValue(
      detail([approval({ state: "rejected", sequence: 2 })], {
        events: [
          {
            sequence: 4,
            at: "2026-11-12T10:01:00Z",
            event: {
              kind: "approval_requested",
              approvalId: "ap1",
              sequence: 2,
            },
          },
          {
            sequence: 5,
            at: "2026-11-12T10:02:00Z",
            event: {
              kind: "approval_resolved",
              approvalId: "ap1",
              outcome: "rejected",
            },
          },
          {
            sequence: 6,
            at: "2026-11-12T10:03:00Z",
            event: {
              kind: "approval_resolved",
              approvalId: "otro",
              outcome: "expired",
            },
          },
        ],
      }),
    );
    renderDetail();

    const events = await screen.findByRole("list", {
      name: "Eventos en orden",
    });
    const items = within(events).getAllByRole("listitem");
    expect(items[0]).toHaveTextContent("Petición de permiso #2");
    expect(items[1]).toHaveTextContent("Permiso #2: Rechazado");
    expect(items[2]).toHaveTextContent(
      "Permiso: Caducó (se respondió rechazar)",
    );
  });
});

describe("Sistema → Ejecuciones: notificaciones de permisos", () => {
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

  it("la casilla está desactivada por defecto, explica el aviso y se envía al guardar", async () => {
    const user = userEvent.setup();
    mocked.getRunSettings.mockResolvedValue(settings());
    mocked.updateRunSettings
      .mockResolvedValueOnce(
        settings({ approvalNotifications: true, revision: 2 }),
      )
      .mockResolvedValue(
        settings({ approvalNotifications: false, revision: 3 }),
      );
    render(<RunSettingsPanel />);

    const box = await screen.findByRole("checkbox", {
      name: "Avisarme con una notificación del sistema cuando un agente pida permiso",
    });
    expect(box).not.toBeChecked();
    expect(box).toHaveAccessibleDescription(
      "El aviso solo dice el número de la historia, nunca el comando ni la ruta. Puede requerir permitir notificaciones de Kanban IA en el sistema.",
    );

    await user.click(box);
    // Marcarla es un cambio sin guardar.
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    expect(mocked.updateRunSettings).toHaveBeenCalledWith({
      expectedRevision: 1,
      maxConcurrency: 2,
      timeoutMinutes: 30,
      claudeAcpPath: null,
      codexAcpPath: null,
      nodePath: null,
      approvalNotifications: true,
      retentionDays: 30,
    });
    expect(
      await screen.findByText("✓ Configuración guardada."),
    ).toBeInTheDocument();
    expect(box).toBeChecked();
    await waitFor(() => {
      expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(false);
    });

    await user.click(box);
    await user.click(screen.getByRole("button", { name: "Guardar" }));
    expect(mocked.updateRunSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({
        expectedRevision: 2,
        approvalNotifications: false,
      }),
    );
  });
});
