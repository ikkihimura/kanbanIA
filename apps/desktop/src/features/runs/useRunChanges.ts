import { useEffect, useRef } from "react";
import type { RunChange } from "../../contracts/generated/RunChange";
import { onRunChange } from "../../shared/ipc/commands";

/** Ventana para agrupar avisos `run-change`: una ráfaga del supervisor da una sola recarga. */
export const RUN_CHANGE_BATCH_MS = 300;

/**
 * Llama a `handler` con los avisos `run-change` del supervisor, agrupados como los de
 * `useAgentChanges`: el primero abre una ventana fija de `RUN_CHANGE_BATCH_MS` y al cerrarla
 * se entrega uno solo con la unión de `runIds` y `storyIds`. El aviso solo sirve para volver a
 * leer del núcleo. `handler` puede cambiar en cada render: se usa siempre el último.
 */
export function useRunChanges(handler: (change: RunChange) => void): void {
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  }, [handler]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    let pending: { runIds: Set<string>; storyIds: Set<string> } | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const flush = () => {
      timer = undefined;
      const batch = pending;
      pending = null;
      if (active && batch)
        latest.current({
          runIds: [...batch.runIds],
          storyIds: [...batch.storyIds],
        });
    };

    void onRunChange((change) => {
      if (!active) return;
      pending ??= { runIds: new Set(), storyIds: new Set() };
      for (const id of change.runIds) pending.runIds.add(id);
      for (const id of change.storyIds) pending.storyIds.add(id);
      timer ??= setTimeout(flush, RUN_CHANGE_BATCH_MS);
    }).then(
      (stop) => {
        if (active) unlisten = stop;
        else stop();
      },
      () => {
        // Sin eventos la app sigue funcionando: «Actualizar» vuelve a leer a mano.
      },
    );
    return () => {
      active = false;
      if (timer !== undefined) clearTimeout(timer);
      unlisten?.();
    };
  }, []);
}
