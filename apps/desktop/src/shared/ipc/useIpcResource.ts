import { useCallback, useEffect, useState } from "react";
import { describeError } from "./commands";

export type ResourceState<T> =
  | { status: "loading" }
  | { status: "ready"; data: T }
  | { status: "error"; message: string };

/**
 * Carga un recurso por IPC con estados de carga/error. `load` debe ser estable
 * (función de módulo o `useCallback`; si depende de props, montar con `key`).
 * `run(next)` vuelve a cargar, opcionalmente con otro comando. Con `keepData`, los datos
 * actuales siguen visibles durante la recarga (no se desmonta la lista ni se pierde el foco).
 */
export function useIpcResource<T>(load: () => Promise<T>) {
  const [state, setState] = useState<ResourceState<T>>({ status: "loading" });
  const [request, setRequest] = useState({ load });

  useEffect(() => {
    let cancelled = false;
    request.load().then(
      (data) => {
        if (!cancelled) setState({ status: "ready", data });
      },
      (error: unknown) => {
        if (!cancelled)
          setState({ status: "error", message: describeError(error) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [request]);

  const run = useCallback(
    (next?: () => Promise<T>, options?: { keepData?: boolean }) => {
      if (!options?.keepData) setState({ status: "loading" });
      setRequest({ load: next ?? load });
    },
    [load],
  );

  return { state, run };
}
