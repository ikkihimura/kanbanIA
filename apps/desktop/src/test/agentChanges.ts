// Simulación del evento `agent-change` para pruebas de componentes (solo pruebas).
import { act, waitFor } from "@testing-library/react";
import { expect, type Mock } from "vitest";
import type { AgentChange } from "../contracts/generated/AgentChange";
import { AGENT_CHANGE_BATCH_MS } from "../shared/ipc/useAgentChanges";

type Handler = (change: AgentChange) => void;

const handlers = new Set<Handler>();

/** Conecta el `onAgentChange` simulado a los suscriptores de prueba. Llamar en `beforeEach`. */
export function fakeAgentChanges(
  onAgentChange: Mock<(handler: Handler) => Promise<() => void>>,
): void {
  handlers.clear();
  onAgentChange.mockImplementation((handler) => {
    handlers.add(handler);
    return Promise.resolve(() => {
      handlers.delete(handler);
    });
  });
}

/**
 * Imita al host: entrega `change` (o una ráfaga) a todos los suscriptores y espera a que cierre
 * la ventana de agrupación de `useAgentChanges`, de modo que la recarga (si procede) ya se
 * pidió. Con `gapMs`, cada aviso de la ráfaga llega en su propio turno, separado por esa pausa
 * real (como desde el host), en lugar de todos en el mismo lote de React.
 */
export async function emitAgentChange(
  change: AgentChange | AgentChange[],
  options: { gapMs?: number } = {},
): Promise<void> {
  const changes = Array.isArray(change) ? change : [change];
  await waitFor(() => {
    expect(handlers.size).toBeGreaterThan(0);
  });
  for (const one of changes) {
    act(() => {
      for (const handler of handlers) handler(one);
    });
    if (options.gapMs !== undefined) await sleep(options.gapMs);
  }
  await sleep(AGENT_CHANGE_BATCH_MS + 20);
}

async function sleep(ms: number): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}
