import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { StoryClaim } from "../../contracts/generated/StoryClaim";
import { listActiveClaims, releaseClaim } from "../../shared/ipc/commands";
import { useRefreshSignal } from "../../shared/ipc/useAgentChanges";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { formatDateTime, formatTime } from "../../shared/ui/format";
import { useUnexpiredClaims } from "./useUnexpiredClaims";

const NO_CLAIMS: StoryClaim[] = [];

interface Props {
  projectId: string;
  storyId: string;
  /** Se liberó el reclamo: el núcleo también quitó la asignación del agente. */
  onReleased?: () => void;
  /** Cambia cuando un agente reclama o renueva esta historia: vuelve a consultar. */
  refreshSignal?: number;
}

/**
 * Reclamo vigente de la historia (HU-014). Los agentes reclaman y renuevan por MCP; aquí la
 * persona solo lo ve y, si hace falta, lo libera. Liberar también quita la asignación.
 */
export function ClaimPanel({
  projectId,
  storyId,
  onReleased,
  refreshSignal,
}: Props) {
  const id = useId();
  const load = useCallback(() => listActiveClaims(projectId), [projectId]);
  const claims = useIpcResource(load);
  const releaser = useMutation(releaseClaim);
  const [confirm, setConfirm] = useState<StoryClaim | null>(null);
  const [message, setMessage] = useState("");
  const titleRef = useRef<HTMLHeadingElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const openRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (confirm) cancelRef.current?.focus();
  }, [confirm]);

  // Un reclamo vencido ya no protege la historia: deja de mostrarse al vencer.
  const active = useUnexpiredClaims(
    claims.state.status === "ready" ? claims.state.data : NO_CLAIMS,
  );
  const claim = active.find((c) => c.storyId === storyId);

  const { run } = claims;
  const reload = useCallback(() => {
    run(load, { keepData: true });
  }, [run, load]);
  useRefreshSignal(refreshSignal, reload);

  function openConfirm(current: StoryClaim) {
    releaser.reset();
    setMessage("");
    setConfirm(current);
  }

  function closeConfirm() {
    setConfirm(null);
    releaser.reset();
    // Si el reclamo venció mientras tanto, el botón ya no existe.
    (openRef.current ?? titleRef.current)?.focus();
  }

  async function handleRelease() {
    if (!confirm) return;
    const result = await releaser.mutate({
      projectId,
      claimId: confirm.claimId,
    });
    if (result.ok) {
      const name = confirm.owner.agentName;
      setConfirm(null);
      // El botón «Liberar» desaparece: el foco va al título de la sección.
      titleRef.current?.focus();
      setMessage(
        `Reclamo de ${name} liberado y asignación quitada: la historia queda libre para cualquier agente.`,
      );
      reload();
      onReleased?.();
    }
  }

  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <h2
        id={`${id}-title`}
        className="panel__title"
        ref={titleRef}
        tabIndex={-1}
      >
        Reclamo
      </h2>

      <p role="status" className="relations__saved">
        {message && `✓ ${message}`}
      </p>

      {claims.state.status === "loading" && (
        <p className="panel__muted">Cargando reclamo…</p>
      )}
      {claims.state.status === "error" && (
        <div>
          <p className="panel__error" role="alert">
            No se pudo consultar el reclamo: {claims.state.message}
          </p>
          <button type="button" className="button" onClick={reload}>
            Reintentar
          </button>
        </div>
      )}
      {claims.state.status === "ready" && !claim && (
        <p className="panel__muted">Sin reclamo.</p>
      )}
      {claim && (
        <>
          <p>
            Reclamada por{" "}
            <strong>
              A-{claim.owner.agentNumber} {claim.owner.agentName}
            </strong>{" "}
            desde el {formatDateTime(claim.claimedAt)}. Vence a las{" "}
            {formatTime(claim.expiresAt)}.
          </p>
          <p className="field__hint">
            {claim.renewedAt === null
              ? "Sin renovar."
              : `Renovado el ${formatDateTime(claim.renewedAt)}.`}{" "}
            Mientras esté vigente, ningún otro agente puede tomarla.
          </p>
          {/* Sigue montado durante la confirmación: «Cancelar» le devuelve el foco. */}
          <button
            type="button"
            className="button button--secondary"
            ref={openRef}
            disabled={releaser.pending}
            onClick={() => {
              openConfirm(claim);
            }}
          >
            Liberar reclamo…
          </button>
        </>
      )}

      {confirm && (
        <div
          className="repo__confirm"
          role="alertdialog"
          aria-labelledby={`${id}-confirm`}
          onKeyDown={(event) => {
            if (event.key === "Escape" && !releaser.pending) closeConfirm();
          }}
        >
          <p id={`${id}-confirm`}>
            ¿Liberar el reclamo de A-{confirm.owner.agentNumber}{" "}
            {confirm.owner.agentName}? El agente dejará de poder renovarlo y
            también se quitará la asignación: la historia quedará libre para
            cualquier agente, incluido este. Para impedir que la vuelva a tomar,
            bloquéala o muévela a una columna que no sea de categoría
            «Pendiente» ni «Listo para empezar».
          </p>
          {releaser.error && (
            <p className="field__error" role="alert">
              {releaser.error.message}
            </p>
          )}
          <div className="repo__actions">
            <button
              type="button"
              className="button button--danger"
              aria-busy={releaser.pending}
              disabled={releaser.pending}
              onClick={() => {
                void handleRelease();
              }}
            >
              Liberar
            </button>
            <button
              type="button"
              className="button button--secondary"
              ref={cancelRef}
              disabled={releaser.pending}
              onClick={closeConfirm}
            >
              Cancelar
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
