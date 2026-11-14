import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentChange } from "../../contracts/generated/AgentChange";
import { AGENT_CHANGE_EVENT } from "./commands";
import { AGENT_CHANGE_BATCH_MS, useAgentChanges } from "./useAgentChanges";

// Se simula la API de eventos de Tauri; el envoltorio `onAgentChange` y el hook son los reales.
const listeners = new Map<string, (event: { payload: unknown }) => void>();
const unlisten = vi.fn();

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
  isTauri: () => true,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    (name: string, handler: (event: { payload: unknown }) => void) => {
      listeners.set(name, handler);
      return Promise.resolve(() => {
        unlisten();
        if (listeners.get(name) === handler) listeners.delete(name);
      });
    },
  ),
}));

function emit(payload: unknown) {
  act(() => {
    listeners.get(AGENT_CHANGE_EVENT)?.({ payload });
  });
}

function advance(ms: number) {
  act(() => {
    vi.advanceTimersByTime(ms);
  });
}

/** Deja resolver las promesas pendientes (suscribirse y dejar de escuchar son asíncronos). */
async function settle() {
  await act(async () => {
    await Promise.resolve();
  });
}

function Probe({
  projectId,
  onChange,
}: {
  projectId: string;
  onChange: (change: AgentChange) => void;
}) {
  useAgentChanges(projectId, onChange);
  return null;
}

beforeEach(() => {
  vi.useFakeTimers();
  listeners.clear();
  unlisten.mockClear();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useAgentChanges", () => {
  it("solo entrega los avisos del proyecto y deja de escuchar al desmontar", async () => {
    const onChange = vi.fn();
    const { unmount } = render(<Probe projectId="p1" onChange={onChange} />);
    await settle();
    expect(listeners.has(AGENT_CHANGE_EVENT)).toBe(true);

    emit({ projectId: "p1", storyIds: ["s1"] });
    emit({ projectId: "p2", storyIds: ["s9"] });
    advance(AGENT_CHANGE_BATCH_MS);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith({
      projectId: "p1",
      storyIds: ["s1"],
    });

    unmount();
    await settle();
    expect(unlisten).toHaveBeenCalledTimes(1);
    emit({ projectId: "p1", storyIds: ["s1"] });
    advance(AGENT_CHANGE_BATCH_MS);
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("agrupa una ráfaga en un solo aviso con la unión de historias", async () => {
    const onChange = vi.fn();
    render(<Probe projectId="p1" onChange={onChange} />);
    await settle();

    for (let i = 0; i < 50; i++) {
      emit({ projectId: "p1", storyIds: [`s${String(i % 3)}`] });
      emit({ projectId: "p2", storyIds: ["otra"] });
    }
    emit({ projectId: "p1", storyIds: ["s1", "s7"] });
    advance(AGENT_CHANGE_BATCH_MS - 1);
    expect(onChange).not.toHaveBeenCalled();
    advance(1);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith({
      projectId: "p1",
      storyIds: ["s0", "s1", "s2", "s7"],
    });

    // Un aviso posterior abre otra ventana.
    emit({ projectId: "p1", storyIds: ["s4"] });
    advance(AGENT_CHANGE_BATCH_MS);
    expect(onChange).toHaveBeenCalledTimes(2);
    expect(onChange).toHaveBeenLastCalledWith({
      projectId: "p1",
      storyIds: ["s4"],
    });
  });

  it("un flujo continuo da como mucho un aviso por ventana, sin retrasarlo", async () => {
    const onChange = vi.fn();
    render(<Probe projectId="p1" onChange={onChange} />);
    await settle();

    // Un aviso cada 50 ms durante 1 s.
    for (let t = 0; t < 1000; t += 50) {
      emit({ projectId: "p1", storyIds: ["s1"] });
      advance(50);
    }
    advance(AGENT_CHANGE_BATCH_MS);
    const calls = onChange.mock.calls.length;
    expect(calls).toBeGreaterThanOrEqual(3);
    expect(calls).toBeLessThanOrEqual(
      Math.ceil(1000 / AGENT_CHANGE_BATCH_MS) + 1,
    );
  });

  it("al desmontar durante una ventana no entrega nada", async () => {
    const onChange = vi.fn();
    const { unmount } = render(<Probe projectId="p1" onChange={onChange} />);
    await settle();

    emit({ projectId: "p1", storyIds: ["s1"] });
    unmount();
    advance(AGENT_CHANGE_BATCH_MS);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("ignora avisos mal formados", async () => {
    const onChange = vi.fn();
    render(<Probe projectId="p1" onChange={onChange} />);
    await settle();

    emit(null);
    emit("p1");
    emit({ projectId: "p1" });
    emit({ projectId: "p1", storyIds: [1] });
    advance(AGENT_CHANGE_BATCH_MS);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("al cambiar de proyecto se vuelve a suscribir con el nuevo filtro", async () => {
    const onChange = vi.fn();
    const { rerender } = render(<Probe projectId="p1" onChange={onChange} />);
    await settle();
    rerender(<Probe projectId="p2" onChange={onChange} />);
    await settle();
    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(listeners.has(AGENT_CHANGE_EVENT)).toBe(true);

    emit({ projectId: "p1", storyIds: ["s1"] });
    emit({ projectId: "p2", storyIds: ["s2"] });
    advance(AGENT_CHANGE_BATCH_MS);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith({
      projectId: "p2",
      storyIds: ["s2"],
    });
  });
});
