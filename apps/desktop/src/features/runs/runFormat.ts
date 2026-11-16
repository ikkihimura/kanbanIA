import type { AcpAgent } from "../../contracts/generated/AcpAgent";
import type { AcpSessionInfo } from "../../contracts/generated/AcpSessionInfo";
import type { ApprovalState } from "../../contracts/generated/ApprovalState";
import type { RunApproval } from "../../contracts/generated/RunApproval";
import type { RunEvent } from "../../contracts/generated/RunEvent";
import type { RunExecutor } from "../../contracts/generated/RunExecutor";
import type { RunOutputKind } from "../../contracts/generated/RunOutputKind";
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
 * Ejecutor de prueba de HU-018. Se muestra siempre junto a «Iniciar» y en el detalle: nunca se
 * da a entender que trabaja un agente.
 */
export const TEST_PROCESS_LABEL = "Proceso de prueba (sin IA)";

/** Ejecutor real de HU-019: Claude Code a través del adaptador ACP que configura la persona. */
export const CLAUDE_ACP_LABEL = "Claude Code (ACP)";

/** Segundo ejecutor real (HU-021): Codex a través de su adaptador ACP. */
export const CODEX_ACP_LABEL = "Codex (ACP)";

/** Nombre del agente real de cada ejecutor ACP, para frases («el adaptador de …»). */
export const ACP_AGENT_NAME: Record<AcpAgent, string> = {
  claudeCode: "Claude Code",
  codex: "Codex",
};

/** Ejecutor ACP → agente real; `null` para el proceso de prueba. */
export function acpAgentOf(executor: RunExecutor): AcpAgent | null {
  if (executor.kind === "claudeAcp") return "claudeCode";
  if (executor.kind === "codexAcp") return "codex";
  return null;
}

export function executorLabel(executor: RunExecutor): string {
  if (executor.kind === "claudeAcp") return CLAUDE_ACP_LABEL;
  if (executor.kind === "codexAcp") return CODEX_ACP_LABEL;
  return TEST_PROCESS_LABEL;
}

/** Ejecutor con su escenario, en una línea («Proceso de prueba (sin IA) · escenario …»). */
export function describeExecutor(executor: RunExecutor): string {
  return executor.kind === "testProcess"
    ? `${TEST_PROCESS_LABEL} · escenario «${SCENARIO_LABEL[executor.scenario]}»`
    : executorLabel(executor);
}

/** Modelo que declaró el agente (HU-021): reportado, la app no lo elige ni lo verifica. */
export function describeAgentModel(model: string | null): string {
  return model ?? "no informado";
}

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
  stdout: "stdout",
  stderr: "stderr",
  system: "app",
  agent: "agente (reportado)",
};

/** Tipo de una línea `agent` (lo que reporta el agente por ACP). */
export const OUTPUT_KIND_LABEL: Record<RunOutputKind, string> = {
  message: "mensaje",
  thought: "pensamiento",
  tool: "herramienta",
  plan: "plan",
  usage: "uso",
  permission: "permiso",
};

/**
 * Capacidades ACP declaradas por el agente, en español. Una clave desconocida se muestra tal
 * cual (texto plano): el núcleo ya las limita a una lista conocida.
 */
export const ACP_CAPABILITY_LABEL: Record<string, string> = {
  loadSession: "retomar sesión",
  image: "imágenes",
  audio: "audio",
  embeddedContext: "contexto incrustado",
  mcpHttp: "MCP por HTTP",
  mcpSse: "MCP por SSE",
  modes: "modos",
};

export function describeCapabilities(capabilities: readonly string[]): string {
  if (capabilities.length === 0) return "ninguna declarada";
  return capabilities.map((c) => ACP_CAPABILITY_LABEL[c] ?? c).join(", ");
}

/** Modo de sesión que fijó la app. `default` del adaptador de Claude Code pide permiso. */
export function describeAgentMode(mode: string | null): string {
  if (mode === null) return "ninguno fijado";
  if (mode === "default") return "Manual: pide permiso";
  return mode;
}

/** «<título o nombre> <versión>» de lo que declaró el agente; partes ausentes se omiten. */
export function describeAgentIdentity(
  info: Pick<AcpSessionInfo, "agentName" | "agentTitle" | "agentVersion">,
): string {
  const name = info.agentTitle ?? info.agentName ?? "Agente sin nombre";
  return info.agentVersion === null ? name : `${name} ${info.agentVersion}`;
}

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

/**
 * Estado de una petición de permiso (HU-020) en texto. `pending` es la vigente; una pendiente
 * cuyo plazo ya pasó se muestra con `EXPIRED_PENDING_LABEL`.
 */
export const APPROVAL_STATE_LABEL: Record<ApprovalState, string> = {
  pending: "Espera tu decisión",
  allowed: "Permitido una vez",
  rejected: "Rechazado",
  expired: "Caducó (se respondió rechazar)",
  cancelled: "Cancelado",
  withdrawn: "Retirado por el agente",
};

/** Pendiente con el plazo ya vencido: el supervisor responderá «rechazar». */
export const EXPIRED_PENDING_LABEL = "Caducada: la app responderá «rechazar»";

/** Pendiente y dentro de plazo según el reloj local: admite decidir. */
export function isApprovalOpen(
  approval: Pick<RunApproval, "state" | "expiresAt">,
  now: number,
): boolean {
  return approval.state === "pending" && Date.parse(approval.expiresAt) > now;
}

/** Estado visible de una petición, distinguiendo la pendiente ya caducada. */
export function approvalStatusLabel(
  approval: Pick<RunApproval, "state" | "expiresAt">,
  now: number,
): string {
  if (approval.state === "pending" && !isApprovalOpen(approval, now))
    return EXPIRED_PENDING_LABEL;
  return APPROVAL_STATE_LABEL[approval.state];
}

/**
 * Evento en texto. Los motivos los genera el núcleo; aun así se muestran como texto plano.
 * Con `approvals`, la resolución de un permiso lleva su número («Permiso #n: …»).
 */
export function describeRunEvent(
  event: RunEvent,
  approvals: readonly Pick<RunApproval, "id" | "sequence">[] = [],
): string {
  if (event.kind === "state_changed") {
    const to = `Estado → ${RUN_STATE_LABEL[event.to]}`;
    return event.reason === null ? to : `${to}: ${event.reason}`;
  }
  if (event.kind === "acp_session")
    return `Sesión del agente iniciada (reportado): ${describeAgentIdentity(event)} · ACP v${String(event.protocolVersion)}`;
  if (event.kind === "approval_requested")
    return `Petición de permiso #${String(event.sequence)}`;
  if (event.kind === "approval_resolved") {
    const approval = approvals.find((a) => a.id === event.approvalId);
    const label = approval
      ? `Permiso #${String(approval.sequence)}`
      : "Permiso";
    return `${label}: ${APPROVAL_STATE_LABEL[event.outcome]}`;
  }
  return `Uso reportado (${describeUsage(event)})`;
}
