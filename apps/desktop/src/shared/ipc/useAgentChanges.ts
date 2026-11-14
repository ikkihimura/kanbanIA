import { useEffect, useRef } from "react";
import type { AgentChange } from "../../contracts/generated/AgentChange";
import { onAgentChange } from "./commands";

/** Ventana para agrupar avisos: una ráfaga de un agente da una sola recarga por componente. */
export const AGENT_CHANGE_BATCH_MS = 300;

/**
 * Llama a `handler` con los avisos `agent-change` del proyecto `projectId`; los de otros
 * proyectos se ignoran. Los avisos se agrupan: el primero abre una ventana de
 * `AGENT_CHANGE_BATCH_MS` y al cerrarla se entrega uno solo con la unión de `storyIds`. Es una
 * ventana fija (no se alarga con cada aviso), así que un flujo continuo da como mucho una
 * recarga por ventana y nunca la retrasa indefinidamente. `handler` puede cambiar en cada
 * render: se usa siempre el último sin volver a suscribirse.
 */
export function useAgentChanges(
  projectId: string,
  handler: (change: AgentChange) => void,
): void {
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  }, [handler]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    let pending: Set<string> | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const flush = () => {
      timer = undefined;
      const storyIds = pending;
      pending = null;
      if (active && storyIds)
        latest.current({ projectId, storyIds: [...storyIds] });
    };

    void onAgentChange((change) => {
      if (!active || change.projectId !== projectId) return;
      pending ??= new Set();
      for (const id of change.storyIds) pending.add(id);
      timer ??= setTimeout(flush, AGENT_CHANGE_BATCH_MS);
    }).then(
      (stop) => {
        if (active) unlisten = stop;
        else stop();
      },
      () => {
        // Sin eventos la app sigue funcionando: los datos se ven al recargar a mano.
      },
    );
    return () => {
      active = false;
      if (timer !== undefined) clearTimeout(timer);
      unlisten?.();
    };
  }, [projectId]);
}

/**
 * Ejecuta `refresh` cada vez que cambia `signal` (no en el primer render). Los paneles lo usan
 * para recargar sus datos de solo lectura cuando el editor recibe un aviso de un agente.
 */
export function useRefreshSignal(
  signal: number | undefined,
  refresh: () => void,
): void {
  const latest = useRef(refresh);
  useEffect(() => {
    latest.current = refresh;
  }, [refresh]);
  const seen = useRef(signal);
  useEffect(() => {
    if (seen.current === signal) return;
    seen.current = signal;
    latest.current();
  }, [signal]);
}
