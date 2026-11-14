import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { AgentCredential } from "../../contracts/generated/AgentCredential";
import {
  createAgentCredential,
  getLocalServiceStatus,
  listAgentCredentials,
  listAgents,
  revokeAgentCredential,
} from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { formatDateTime } from "../../shared/ui/format";

interface Props {
  projectId: string;
}

/**
 * Credenciales de agentes para el servicio local (HU-012). Cada una da acceso a este
 * proyecto y a nada más; el token se muestra una sola vez.
 */
export function AgentAccessSection({ projectId }: Props) {
  const id = useId();
  const loadCredentials = useCallback(
    () => listAgentCredentials(projectId),
    [projectId],
  );
  const credentials = useIpcResource(loadCredentials);
  const agents = useIpcResource(listAgents);
  const service = useIpcResource(getLocalServiceStatus);
  const creator = useMutation(createAgentCredential);
  const revoker = useMutation(revokeAgentCredential);

  const [agentId, setAgentId] = useState("");
  const [created, setCreated] = useState<{
    token: string;
    agentName: string;
  } | null>(null);
  const [copied, setCopied] = useState<"idle" | "ok" | "failed">("idle");
  const [confirm, setConfirm] = useState<AgentCredential | null>(null);
  const [message, setMessage] = useState("");
  const tokenRef = useRef<HTMLInputElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const trigger = useRef<HTMLElement | null>(null);
  // Destino estable del foco cuando desaparece lo que lo tenía (token guardado, revocada).
  const titleRef = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    if (created) tokenRef.current?.focus();
  }, [created]);
  useEffect(() => {
    if (confirm) cancelRef.current?.focus();
  }, [confirm]);

  const selectedAgent =
    agents.state.status === "ready"
      ? agents.state.data.find((a) => a.id === agentId)
      : undefined;

  function reload() {
    credentials.run(loadCredentials, { keepData: true });
  }

  async function handleCreate() {
    if (!selectedAgent) return;
    setMessage("");
    setCopied("idle");
    const result = await creator.mutate({ projectId, agentId });
    if (result.ok) {
      setCreated({ token: result.value.token, agentName: selectedAgent.name });
      reload();
    }
  }

  async function handleCopy() {
    if (!created) return;
    try {
      await navigator.clipboard.writeText(created.token);
      setCopied("ok");
    } catch {
      tokenRef.current?.select();
      setCopied("failed");
    }
  }

  function openConfirm(credential: AgentCredential) {
    trigger.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    revoker.reset();
    setMessage("");
    setConfirm(credential);
  }

  function closeConfirm() {
    setConfirm(null);
    revoker.reset();
    trigger.current?.focus();
  }

  async function handleRevoke() {
    if (!confirm) return;
    const result = await revoker.mutate({ projectId, id: confirm.id });
    if (result.ok) {
      setConfirm(null);
      // La fila pasa a «Revocada» y su botón desaparece: el foco va a un sitio estable.
      titleRef.current?.focus();
      setMessage(
        `Credencial de ${result.value.agentName} revocada: ya no tiene acceso.`,
      );
      reload();
    }
  }

  const noAgents =
    agents.state.status === "ready" && agents.state.data.length === 0;

  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <h2
        id={`${id}-title`}
        className="panel__title"
        ref={titleRef}
        tabIndex={-1}
      >
        Acceso de agentes
      </h2>
      <p className="field__hint">
        Una credencial permite a un agente usar este proyecto, y nada más, a
        través del servidor MCP <code>kanbania-mcp</code> y del servicio local
        de este equipo. Cómo configurarlo: guía local del repositorio (
        <code>docs/GUIA-LOCAL.md</code>).
      </p>
      <p className="field__hint">
        {service.state.status === "ready" &&
          (service.state.data.running && service.state.data.port !== undefined
            ? `Servicio local: activo en este equipo (127.0.0.1:${String(service.state.data.port)}).`
            : "Servicio local: no disponible. Los agentes no pueden conectarse ahora; cierra Kanban IA y vuelve a abrirlo.")}
        {service.state.status === "error" &&
          "Servicio local: no se pudo consultar su estado. Cierra Kanban IA y vuelve a abrirlo."}
      </p>

      <div className="form-row">
        <div className="field">
          <label htmlFor={`${id}-agent`}>Agente</label>
          <select
            id={`${id}-agent`}
            value={agentId}
            aria-describedby={noAgents ? `${id}-no-agents` : undefined}
            disabled={noAgents || creator.pending}
            onChange={(event) => {
              setAgentId(event.target.value);
            }}
          >
            <option value="">Elige un agente</option>
            {agents.state.status === "ready" &&
              agents.state.data.map((a) => (
                <option key={a.id} value={a.id}>
                  A-{a.number} {a.name}
                </option>
              ))}
          </select>
          {noAgents && (
            <span id={`${id}-no-agents`} className="field__hint">
              Primero crea un agente en «Equipo».
            </span>
          )}
        </div>
        <button
          type="button"
          className="button"
          aria-busy={creator.pending}
          disabled={!selectedAgent || creator.pending}
          onClick={() => {
            void handleCreate();
          }}
        >
          Crear credencial
        </button>
      </div>
      {creator.error && (
        <p className="field__error" role="alert">
          {creator.error.message}
        </p>
      )}

      {created && (
        <div
          className="access__secret"
          role="group"
          aria-labelledby={`${id}-secret`}
        >
          <p id={`${id}-secret`}>
            <strong>Copia ahora la credencial de {created.agentName}:</strong>{" "}
            no se volverá a mostrar, tampoco si sales de esta vista. Guárdala
            solo en la configuración de su integración; no la pegues en
            historias ni en Git.
          </p>
          <div className="field__inline">
            <input
              ref={tokenRef}
              className="input--mono access__token"
              readOnly
              value={created.token}
              aria-label={`Credencial de ${created.agentName}`}
              onFocus={(event) => {
                event.currentTarget.select();
              }}
            />
            <button
              type="button"
              className="button button--secondary"
              onClick={() => {
                void handleCopy();
              }}
            >
              Copiar
            </button>
            <button
              type="button"
              className="button button--secondary"
              onClick={() => {
                setCreated(null);
                setCopied("idle");
                titleRef.current?.focus();
              }}
            >
              Ya la guardé
            </button>
          </div>
          <p role="status" className="relations__saved">
            {copied === "ok" && "✓ Copiada al portapapeles."}
            {copied === "failed" &&
              "No se pudo copiar: está seleccionada, cópiala con ⌘C o Ctrl+C."}
          </p>
        </div>
      )}

      <p role="status" className="relations__saved">
        {message && `✓ ${message}`}
      </p>

      {confirm && (
        <div
          className="repo__confirm"
          role="alertdialog"
          aria-labelledby={`${id}-confirm`}
          onKeyDown={(event) => {
            if (event.key === "Escape" && !revoker.pending) closeConfirm();
          }}
        >
          <p id={`${id}-confirm`}>
            ¿Revocar la credencial de {confirm.agentName} (…
            {confirm.tokenHint})? Su integración dejará de tener acceso en la
            siguiente petición. No se puede deshacer: tendrías que crear otra.
          </p>
          {revoker.error && (
            <p className="field__error" role="alert">
              {revoker.error.message}
            </p>
          )}
          <div className="repo__actions">
            <button
              type="button"
              className="button button--danger"
              aria-busy={revoker.pending}
              disabled={revoker.pending}
              onClick={() => {
                void handleRevoke();
              }}
            >
              Revocar
            </button>
            <button
              type="button"
              className="button button--secondary"
              ref={cancelRef}
              disabled={revoker.pending}
              onClick={closeConfirm}
            >
              Cancelar
            </button>
          </div>
        </div>
      )}

      {credentials.state.status === "loading" && (
        <p className="panel__muted">Cargando credenciales…</p>
      )}
      {credentials.state.status === "error" && (
        <p className="panel__error" role="alert">
          {credentials.state.message}
        </p>
      )}
      {credentials.state.status === "ready" &&
        credentials.state.data.length === 0 && (
          <p className="panel__muted">Ningún agente tiene acceso todavía.</p>
        )}
      {credentials.state.status === "ready" &&
        credentials.state.data.length > 0 && (
          <ul className="item-list">
            {credentials.state.data.map((c) => (
              <li key={c.id} className="backup-item">
                <span className="item-list__name">
                  A-{c.agentNumber} {c.agentName}{" "}
                  <span className="input--mono">…{c.tokenHint}</span>{" "}
                  <span
                    className={
                      c.revokedAt === null ? "badge badge--ok" : "badge"
                    }
                  >
                    {c.revokedAt === null ? "Activa" : "Revocada"}
                  </span>
                </span>
                <span className="item-list__meta">
                  Creada el {formatDateTime(c.createdAt)} · Último uso:{" "}
                  {c.lastUsedAt === null
                    ? "nunca"
                    : formatDateTime(c.lastUsedAt)}
                  {c.revokedAt !== null &&
                    ` · Revocada el ${formatDateTime(c.revokedAt)}`}
                </span>
                {c.revokedAt === null && (
                  <span className="repo__actions">
                    <button
                      type="button"
                      className="button button--secondary"
                      disabled={revoker.pending}
                      onClick={() => {
                        openConfirm(c);
                      }}
                    >
                      Revocar…
                    </button>
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
    </section>
  );
}
