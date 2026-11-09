import { useRef } from "react";
import { useAppInfo } from "./useAppInfo";

export function AppInfoPanel() {
  const { state, retry } = useAppInfo();
  // El botón desaparece al reintentar; el foco pasa al título para no perderse en <body>.
  const titleRef = useRef<HTMLHeadingElement>(null);

  function handleRetry() {
    titleRef.current?.focus();
    retry();
  }

  return (
    <section
      className="panel"
      aria-labelledby="app-info-title"
      aria-busy={state.status === "loading"}
    >
      <h2
        id="app-info-title"
        className="panel__title"
        ref={titleRef}
        tabIndex={-1}
      >
        Información de versión
      </h2>

      {/* Solo los mensajes de estado se anuncian; la tabla se lee al navegar. */}
      <div role="status" aria-live="polite">
        {state.status === "loading" && (
          <p className="panel__muted">Consultando el núcleo…</p>
        )}

        {state.status === "error" && (
          <div className="panel__error">
            <p>
              <strong>Error:</strong> no se pudo obtener la información del
              núcleo.
            </p>
            <p className="panel__muted">{state.message}</p>
          </div>
        )}
      </div>

      {state.status === "ready" && (
        <dl className="facts">
          <dt>Aplicación</dt>
          <dd>{state.info.productName}</dd>
          <dt>Versión</dt>
          <dd>{state.info.appVersion}</dd>
          <dt>Núcleo</dt>
          <dd>{state.info.coreVersion}</dd>
          <dt>Contrato</dt>
          <dd>v{state.info.contractVersion}</dd>
          <dt>Plataforma</dt>
          <dd>
            {state.info.os} · {state.info.arch}
          </dd>
        </dl>
      )}

      {state.status === "error" && (
        <button type="button" className="button" onClick={handleRetry}>
          Reintentar
        </button>
      )}
    </section>
  );
}
