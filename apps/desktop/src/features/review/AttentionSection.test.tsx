import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AttentionItem } from "../../contracts/generated/AttentionItem";
import * as commands from "../../shared/ipc/commands";
import { emitAgentChange, fakeAgentChanges } from "../../test/agentChanges";
import { AttentionSection } from "./AttentionSection";

vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  listAttention: vi.fn(),
  onAgentChange: vi.fn(),
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

beforeEach(() => {
  vi.clearAllMocks();
  fakeAgentChanges(mocked.onAgentChange);
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
