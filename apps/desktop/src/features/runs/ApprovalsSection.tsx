import { useEffect, useId, useRef, useState } from "react";
import type { ApprovalDecision } from "../../contracts/generated/ApprovalDecision";
import type { RunApproval } from "../../contracts/generated/RunApproval";
import { decideApproval, newRequestId } from "../../shared/ipc/commands";
import { useMutation } from "../../shared/ipc/useMutation";
import { formatDateTime, formatTime } from "../../shared/ui/format";
import { approvalStatusLabel, isApprovalOpen } from "./runFormat";

// Límite de `setTimeout` (≈ 24,8 días); por encima se dispararía al momento.
const MAX_DELAY = 2_147_483_647;

/**
 * Hora local que avanza al vencer la próxima petición pendiente: una petición caducada deja de
 * ofrecer botones sin esperar a que llegue el aviso del supervisor.
 */
function useApprovalClock(approvals: readonly RunApproval[]): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const next = approvals
      .filter((a) => a.state === "pending")
      .map((a) => Date.parse(a.expiresAt))
      .filter((t) => t > now)
      .reduce((a, b) => Math.min(a, b), Number.POSITIVE_INFINITY);
    if (next === Number.POSITIVE_INFINITY) return;
    const delay = Math.min(Math.max(next - Date.now(), 0) + 1, MAX_DELAY);
    const timer = setTimeout(() => {
      setNow(Date.now());
    }, delay);
    return () => {
      clearTimeout(timer);
    };
  }, [approvals, now]);
  return now;
}

interface Props {
  projectId: string;
  /** Más reciente primero, tal como las devuelve el núcleo. */
  approvals: RunApproval[];
  /** Tras decidir (o si el núcleo dice que ya estaba cerrada): recargar el detalle. */
  onChanged: () => void;
}

/**
 * «Permisos del agente» (HU-020). Acción, tipo y alcance los informa el agente: se muestran
 * como texto plano y la app no los comprueba. Solo «Permitir una vez» y «Rechazar».
 */
export function ApprovalsSection({ projectId, approvals, onChanged }: Props) {
  const id = useId();
  const now = useApprovalClock(approvals);
  const open = approvals.filter((a) => isApprovalOpen(a, now)).length;
  return (
    <section
      className={`approvals${open > 0 ? " approvals--open" : ""}`}
      aria-labelledby={`${id}-title`}
    >
      <h4 id={`${id}-title`} className="relations__title">
        Permisos del agente
        {open > 0 &&
          ` (${String(open)} ${open === 1 ? "espera" : "esperan"} tu decisión)`}
      </h4>
      <p className="field__hint">
        La acción y el alcance los informa el agente: la app no los comprueba.
        «Permitir una vez» vale solo para esa petición. Sin respuesta en 10
        minutos, la petición caduca y la app responde «rechazar». Mientras
        esperas, el tiempo máximo del intento sigue contando.
      </p>
      <ol className="approvals__list" aria-label="Peticiones de permiso">
        {approvals.map((approval) => (
          <ApprovalItem
            key={approval.id}
            projectId={projectId}
            approval={approval}
            now={now}
            onChanged={onChanged}
          />
        ))}
      </ol>
    </section>
  );
}

interface ItemProps {
  projectId: string;
  approval: RunApproval;
  now: number;
  onChanged: () => void;
}

function ApprovalItem({ projectId, approval, now, onChanged }: ItemProps) {
  const id = useId();
  const titleRef = useRef<HTMLHeadingElement>(null);
  const decide = useMutation(decideApproval);
  const [sending, setSending] = useState<ApprovalDecision | null>(null);
  // Mismo `requestId` mientras se reintenta la misma decisión; otra decisión, otro envío.
  const attempt = useRef<{
    decision: ApprovalDecision;
    requestId: string;
  } | null>(null);
  const [closedMessage, setClosedMessage] = useState<string | null>(null);
  // Decisión aceptada por el núcleo: sin botones mientras llega el detalle recargado.
  const [decided, setDecided] = useState(false);

  const isOpen = isApprovalOpen(approval, now);
  const canDecide = isOpen && !decided;
  const pending = approval.state === "pending";
  const status = approvalStatusLabel(approval, now);
  const titleId = `${id}-title`;
  const scopeId = `${id}-scope`;
  const errorId = `${id}-error`;

  async function send(decision: ApprovalDecision) {
    if (attempt.current?.decision !== decision)
      attempt.current = { decision, requestId: newRequestId() };
    setClosedMessage(null);
    setSending(decision);
    const result = await decide.mutate({
      requestId: attempt.current.requestId,
      projectId,
      approvalId: approval.id,
      decision,
    });
    setSending(null);
    if (result.ok) {
      attempt.current = null;
      setDecided(true);
      // Los botones desaparecen: el foco va al encabezado de la petición.
      titleRef.current?.focus();
      onChanged();
    } else if (
      result.error.code === "approval_closed" ||
      result.error.code === "not_found"
    ) {
      // Ya no admite decisión (decidida, caducada, retirada o intento terminado): se muestra
      // el motivo del núcleo y se vuelve a leer el estado vigente.
      attempt.current = null;
      decide.reset();
      setClosedMessage(result.error.message);
      titleRef.current?.focus();
      onChanged();
    }
  }

  const error = decide.error;
  const describedBy = (extra: string[]) =>
    [...extra, error ? errorId : null].filter(Boolean).join(" ");

  return (
    <li className={`approval${isOpen ? " approval--open" : ""}`}>
      <h5 id={titleId} className="approval__title" ref={titleRef} tabIndex={-1}>
        Permiso #{approval.sequence}: {approval.toolTitle}
      </h5>
      <p>
        <span className={isOpen ? "badge badge--warn" : "badge"}>{status}</span>
        {!pending && approval.endReason !== null && ` ${approval.endReason}`}
      </p>
      <dl className="facts facts--compact">
        {approval.toolKind !== null && (
          <>
            <dt>Tipo (según el agente)</dt>
            <dd>{approval.toolKind}</dd>
          </>
        )}
        <dt>Alcance (según el agente)</dt>
        <dd>
          <pre id={scopeId} className="approval__scope">
            {approval.scope ?? "(el agente no indicó el alcance)"}
          </pre>
        </dd>
        <dt>Pedido</dt>
        <dd>
          <time dateTime={approval.requestedAt}>
            {formatDateTime(approval.requestedAt)}
          </time>
        </dd>
        <dt>Caducidad</dt>
        <dd>
          <time dateTime={approval.expiresAt}>
            {isOpen
              ? `caduca a las ${formatTime(approval.expiresAt)}`
              : pending || approval.state === "expired"
                ? `caducó a las ${formatTime(approval.expiresAt)}`
                : `plazo hasta las ${formatTime(approval.expiresAt)}`}
          </time>
        </dd>
        {approval.decidedAt !== null && (
          <>
            <dt>Cerrada</dt>
            <dd>
              <time dateTime={approval.decidedAt}>
                {formatDateTime(approval.decidedAt)}
              </time>
            </dd>
          </>
        )}
      </dl>

      {canDecide && (
        <div className="run-start__actions">
          {approval.canAllow ? (
            <button
              type="button"
              className="button"
              disabled={decide.pending}
              aria-busy={sending === "allow_once"}
              aria-describedby={describedBy([titleId, scopeId])}
              onClick={() => {
                void send("allow_once");
              }}
            >
              Permitir una vez
            </button>
          ) : (
            <p className="field__hint">
              El agente no ofreció permitir esta acción: solo puedes rechazarla.
            </p>
          )}
          <button
            type="button"
            className="button button--secondary"
            disabled={decide.pending}
            aria-busy={sending === "reject"}
            aria-describedby={describedBy([titleId])}
            onClick={() => {
              void send("reject");
            }}
          >
            Rechazar
          </button>
        </div>
      )}
      {error && (
        <p id={errorId} className="field__error" role="alert">
          No se pudo registrar la decisión: {error.message}
        </p>
      )}
      {closedMessage !== null && (
        <p className="run-detail__warning" role="alert">
          {closedMessage}
        </p>
      )}
    </li>
  );
}
