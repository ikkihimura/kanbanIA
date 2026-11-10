import { useCallback, useState } from "react";
import { toUiError, type UiError } from "./commands";

export type MutationResult<R> = { ok: true; value: R } | { ok: false };

/**
 * Ejecuta una mutación IPC con estado `pending` y error normalizado.
 * `mutate` devuelve `{ ok, value }`; si falla, el error queda en `error`.
 */
export function useMutation<A, R>(fn: (args: A) => Promise<R>) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<UiError | null>(null);

  const mutate = useCallback(
    async (args: A): Promise<MutationResult<R>> => {
      setPending(true);
      setError(null);
      try {
        return { ok: true, value: await fn(args) };
      } catch (caught) {
        setError(toUiError(caught));
        return { ok: false };
      } finally {
        setPending(false);
      }
    },
    [fn],
  );

  const reset = useCallback(() => {
    setError(null);
  }, []);

  return { mutate, pending, error, reset };
}
