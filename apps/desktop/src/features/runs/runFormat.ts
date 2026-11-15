import type { RunEvent } from "../../contracts/generated/RunEvent";
import type { RunOutputStream } from "../../contracts/generated/RunOutputStream";
import type { RunState } from "../../contracts/generated/RunState";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import type { RunUsage } from "../../contracts/generated/RunUsage";
import type { TestScenario } from "../../contracts/generated/TestScenario";
import type { WorktreeRemoval } from "../../contracts/generated/WorktreeRemoval";

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

/**
 * Estado visible de un intento: distingue lo que el estado solo no dice (una petición de
 * inicio esperando turno o una cancelación pedida que el supervisor aún no ha atendido).
 */
export function runStatusLabel(
  run: Pick<RunSummary, "state" | "startRequestedAt" | "cancelRequestedAt">,
): string {
  if (isActiveRun(run.state) && run.cancelRequestedAt !== null)
    return "Cancelación solicitada";
  if (run.state === "queued" && run.startRequestedAt !== null)
    return "En cola · esperando turno";
  return RUN_STATE_LABEL[run.state];
}

/** Hay una cancelación pedida que el supervisor aún no ha atendido. */
export function isCancelPending(
  run: Pick<RunSummary, "state" | "cancelRequestedAt">,
): boolean {
  return isActiveRun(run.state) && run.cancelRequestedAt !== null;
}

/** En cola sin petición de inicio vigente: se puede iniciar. */
export function canStartRun(
  run: Pick<RunSummary, "state" | "startRequestedAt" | "cancelRequestedAt">,
): boolean {
  return (
    run.state === "queued" &&
    run.startRequestedAt === null &&
    run.cancelRequestedAt === null
  );
}

/**
 * Único ejecutor de HU-018. Se muestra siempre junto a «Iniciar» y en el detalle: nunca se
 * da a entender que trabaja un agente.
 */
export const TEST_PROCESS_LABEL = "Proceso de prueba (sin IA)";

export const SCENARIO_LABEL: Record<TestScenario, string> = {
  success: "Termina bien",
  fail: "Falla (termina con error)",
  slow: "Tarda y crea procesos hijos (unos 10 min)",
  write: "Modifica archivos del worktree",
  flood: "Produce mucha salida",
  hang: "Se queda colgado (hasta el tiempo máximo)",
};

export const SCENARIOS: readonly TestScenario[] = [
  "success",
  "fail",
  "slow",
  "write",
  "flood",
  "hang",
];

/** Etiqueta de cada línea de salida: el origen siempre en texto. */
export const STREAM_LABEL: Record<RunOutputStream, string> = {
  stdout: "salida",
  stderr: "error",
  system: "sistema",
};

export const REMOVAL_LABEL: Record<WorktreeRemoval, string> = {
  clean: "Estaba limpio.",
  discarded: "Se descartaron sus cambios sin confirmar.",
  missing: "La carpeta ya no existía.",
};

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
