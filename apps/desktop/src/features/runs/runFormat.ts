import type { RunEvent } from "../../contracts/generated/RunEvent";
import type { RunState } from "../../contracts/generated/RunState";
import type { RunUsage } from "../../contracts/generated/RunUsage";

/** Estado en texto: el color de la insignia solo lo refuerza. */
export const RUN_STATE_LABEL: Record<RunState, string> = {
  queued: "En cola",
  preparing: "Preparando",
  running: "En ejecución",
  waiting_approval: "Esperando aprobación",
  completed: "Completado",
  failed: "Fallido",
  cancelled: "Cancelado",
  interrupted: "Interrumpido",
};

const TERMINAL: ReadonlySet<RunState> = new Set<RunState>([
  "completed",
  "failed",
  "cancelled",
  "interrupted",
]);

/** Un intento activo admite cancelarse; uno terminal ya no cambia. */
export function isActiveRun(state: RunState): boolean {
  return !TERMINAL.has(state);
}

export function runStateBadgeClass(state: RunState): string {
  if (state === "completed") return "badge badge--ok";
  if (state === "failed" || state === "interrupted") return "badge badge--warn";
  return "badge";
}

/** Texto para un dato que el proveedor no informó: nunca se muestra como cero. */
export const NOT_AVAILABLE = "no disponible";

const integer = new Intl.NumberFormat("es");

export function formatTokens(value: number | null): string {
  return value === null ? NOT_AVAILABLE : integer.format(value);
}

const usd = new Intl.NumberFormat("es", {
  style: "currency",
  currency: "USD",
  minimumFractionDigits: 2,
  maximumFractionDigits: 6,
});

/** Coste en dólares a partir de millonésimas; `null` = no informado. */
export function formatCost(micros: number | null): string {
  return micros === null ? NOT_AVAILABLE : usd.format(micros / 1_000_000);
}

export function describeUsage(usage: RunUsage): string {
  return (
    `tokens de entrada: ${formatTokens(usage.inputTokens)}; ` +
    `tokens de salida: ${formatTokens(usage.outputTokens)}; ` +
    `coste: ${formatCost(usage.costUsdMicros)}`
  );
}

/** Evento en texto. Los motivos los genera el núcleo; aun así se muestran como texto plano. */
export function describeRunEvent(event: RunEvent): string {
  if (event.kind === "state_changed") {
    const to = `Estado → ${RUN_STATE_LABEL[event.to]}`;
    return event.reason === null ? to : `${to}: ${event.reason}`;
  }
  return `Uso reportado (${describeUsage(event)})`;
}
