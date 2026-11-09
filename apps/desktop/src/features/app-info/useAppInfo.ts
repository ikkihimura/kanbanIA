import { useCallback, useEffect, useState } from "react";
import type { AppInfo } from "../../contracts/generated/AppInfo";
import { describeError, getAppInfo } from "../../shared/ipc/commands";

export type AppInfoState =
  | { status: "loading" }
  | { status: "ready"; info: AppInfo }
  | { status: "error"; message: string };

export function useAppInfo() {
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<AppInfoState>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;
    getAppInfo().then(
      (info) => {
        if (!cancelled) setState({ status: "ready", info });
      },
      (error: unknown) => {
        if (!cancelled)
          setState({ status: "error", message: describeError(error) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [attempt]);

  const retry = useCallback(() => {
    setState({ status: "loading" });
    setAttempt((n) => n + 1);
  }, []);

  return { state, retry };
}
