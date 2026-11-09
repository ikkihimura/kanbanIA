import { useRef } from "react";
import { getStorageStatus, retryOpenStorage } from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";

export function StoragePanel() {
  const { state, run } = useIpcResource(getStorageStatus);
  const titleRef = useRef<HTMLHeadingElement>(null);

  const storage = state.status === "ready" ? state.data : null;
  const unavailable = storage?.status === "unavailable" ? storage.error : null;
  const canRetry =
    state.status === "error" || (unavailable?.recoverable ?? false);

  function handleRetry() {
    titleRef.current?.focus();
    // Si el IPC falló, se repite la consulta; si el almacenamiento no abrió, se reintenta abrirlo.
    run(state.status === "error" ? getStorageStatus : retryOpenStorage);
  }

  return (
    <section
      className="panel"
      aria-labelledby="storage-title"
      aria-busy={state.status === "loading"}
    >
      <h2
        id="storage-title"
        className="panel__title"
        ref={titleRef}
        tabIndex={-1}
      >
        Almacenamiento local
      </h2>

      <div role="status" aria-live="polite">
        {state.status === "loading" && (
          <p className="panel__muted">Abriendo almacenamiento…</p>
        )}

        {state.status === "error" && (
          <div className="panel__error">
            <p>
              <strong>Error:</strong> no se pudo consultar el almacenamiento.
            </p>
            <p className="panel__muted">{state.message}</p>
          </div>
        )}

        {unavailable && (
          <div className="panel__error">
            <p>
              <strong>No disponible:</strong> {unavailable.message}
            </p>
            <p className="panel__muted">Código: {unavailable.code}</p>
          </div>
        )}

        {storage?.status === "ready" && (
          <p className="panel__ok">Listo. Los cambios se guardan localmente.</p>
        )}
      </div>

      {storage?.status === "ready" && (
        <dl className="facts">
          <dt>Esquema</dt>
          <dd>v{storage.schemaVersion}</dd>
          <dt>Ubicación</dt>
          <dd className="facts__path">{storage.databasePath}</dd>
        </dl>
      )}

      {canRetry && (
        <button type="button" className="button" onClick={handleRetry}>
          Reintentar
        </button>
      )}
    </section>
  );
}
