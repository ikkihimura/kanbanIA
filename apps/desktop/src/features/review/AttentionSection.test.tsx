import { act, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AttentionItem } from "../../contracts/generated/AttentionItem";
import type { RunChange } from "../../contracts/generated/RunChange";
import type { WaitingApproval } from "../../contracts/generated/WaitingApproval";
import * as commands from "../../shared/ipc/commands";
import { emitAgentChange, fakeAgentChanges } from "../../test/agentChanges";
import { RUN_CHANGE_BATCH_MS } from "../runs/useRunChanges";
import { AttentionSection } from "./AttentionSection";

vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  listAttention: vi.fn(),
  listWaitingApprovals: vi.fn(),
  onAgentChange: vi.fn(),
  onRunChange: vi.fn(),
}));

const mocked = vi.mocked(commands);

function item(extra: Partial<AttentionItem> = {}): AttentionItem {
  return {
    storyId: "s1",
    number: 3,
    title: "Informe",
    columnName: "En curso",
    blockerReason: null,
    pendingPrerequisites: 0,
    pendingDeliveries: 0,
    since: "2026-11-12T10:00:00Z",
    ...extra,
  };
}

function waitingFor(extra: Partial<WaitingApproval> = {}): WaitingApproval {
  return {
    storyId: "s1",
    storyNumber: 3,
    storyTitle: "Informe",
    runId: "r1",
    runNumber: 1,
    agentId: "a1",
    pending: 1,
    since: "2026-11-12T11:00:00Z",
    ...extra,
  };
}

type RunHandler = (change: RunChange) => void;
const runHandlers = new Set<RunHandler>();

async function emitRunChange(change: RunChange): Promise<void> {
  await waitFor(() => {
    expect(runHandlers.size).toBeGreaterThan(0);
  });
  act(() => {
    for (const handler of runHandlers) handler(change);
  });
  await act(async () => {
    await new Promise((resolve) =>
      setTimeout(resolve, RUN_CHANGE_BATCH_MS + 20),
    );
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  fakeAgentChanges(mocked.onAgentChange);
  runHandlers.clear();
  mocked.onRunChange.mockImplementation((handler) => {
    runHandlers.add(handler);
    return Promise.resolve(() => {
      runHandlers.delete(handler);
    });
  });
  mocked.listWaitingApprovals.mockResolvedValue([]);
});

describe("AttentionSection", () => {
  it("un aviso de un agente del proyecto recarga la bandeja sin pasar por «Cargando»", async () => {
    mocked.listAttention.mockResolvedValue([]);
    render(<AttentionSection projectId="p1" onOpenStory={vi.fn()} />);
    expect(await screen.findByText(/Nada pendiente/)).toBeInTheDocument();

    mocked.listAttention.mockResolvedValue([
      item({ blockerReason: "Sin red" }),
      item({
        storyId: "s2",
        number: 4,
        title: "Exportar",
        columnName: "Revisión",
        pendingDeliveries: 1,
      }),
    ]);
    await emitAgentChange({ projectId: "p1", storyIds: ["s1", "s2"] });

    expect(
      await screen.findByRole("heading", { name: "Atención (2)" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("En curso · Bloqueada: Sin red"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Revisión · 1 entrega pendiente de revisión"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Cargando…")).not.toBeInTheDocument();
    expect(mocked.listAttention).toHaveBeenCalledTimes(2);
    expect(mocked.listAttention).toHaveBeenLastCalledWith("p1");
  });

  it("ignora los avisos de agentes de otro proyecto", async () => {
    mocked.listAttention.mockResolvedValue([]);
    render(<AttentionSection projectId="p1" onOpenStory={vi.fn()} />);
    expect(await screen.findByText(/Nada pendiente/)).toBeInTheDocument();

    await emitAgentChange({ projectId: "p2", storyIds: ["s1"] });

    expect(mocked.listAttention).toHaveBeenCalledTimes(1);
  });
});

describe("AttentionSection: permisos de agentes (HU-020)", () => {
  it("une los permisos a la fila de la historia, sin duplicados, y las ordena primero", async () => {
    mocked.listAttention.mockResolvedValue([
      item({
        storyId: "s9",
        number: 9,
        title: "Otra",
        columnName: "Listo",
        pendingDeliveries: 1,
      }),
      item({ blockerReason: "Sin red" }),
    ]);
    mocked.listWaitingApprovals.mockResolvedValue([
      waitingFor({ pending: 2 }),
      waitingFor({
        storyId: "s5",
        storyNumber: 5,
        storyTitle: "Solo permiso",
        runId: "r5",
        since: "2026-11-12T12:00:00Z",
      }),
    ]);
    render(<AttentionSection projectId="p1" onOpenStory={vi.fn()} />);

    // Tres historias únicas, aunque #3 tenga bloqueo y permisos.
    expect(
      await screen.findByRole("heading", { name: "Atención (3)" }),
    ).toBeInTheDocument();
    const rows = within(screen.getByRole("list")).getAllByRole("button");
    expect(rows.map((row) => row.textContent)).toEqual([
      "#3 InformeEn curso · Espera tu permiso (2) · Bloqueada: Sin red",
      "#5 Solo permisoEspera tu permiso (1)",
      "#9 OtraListo · 1 entrega pendiente de revisión",
    ]);
    expect(mocked.listWaitingApprovals).toHaveBeenCalledWith("p1");
  });

  it("el texto vacío menciona los permisos y un aviso run-change recarga la bandeja", async () => {
    mocked.listAttention.mockResolvedValue([]);
    render(<AttentionSection projectId="p1" onOpenStory={vi.fn()} />);
    expect(
      await screen.findByText(
        "Nada pendiente: sin bloqueos, entregas por revisar ni permisos que decidir.",
      ),
    ).toBeInTheDocument();

    mocked.listWaitingApprovals.mockResolvedValue([waitingFor()]);
    await emitRunChange({ runIds: ["r1"], storyIds: ["s1"] });

    expect(
      await screen.findByRole("heading", { name: "Atención (1)" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /#3 Informe/ }),
    ).toHaveTextContent("Espera tu permiso (1)");
    expect(mocked.listWaitingApprovals).toHaveBeenCalledTimes(2);
  });

  it("si los permisos no cargan, la bandeja sigue y lo dice", async () => {
    mocked.listAttention.mockResolvedValue([
      item({ blockerReason: "Sin red" }),
    ]);
    mocked.listWaitingApprovals.mockRejectedValue(new Error("caído"));
    render(<AttentionSection projectId="p1" onOpenStory={vi.fn()} />);

    expect(
      await screen.findByText(/No se pudieron cargar los permisos pendientes/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Atención (1)" }),
    ).toBeInTheDocument();
  });
});
