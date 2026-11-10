import { AppInfoPanel } from "../features/app-info/AppInfoPanel";
import { Workspace } from "../features/projects/Workspace";
import { StoragePanel } from "../features/storage-status/StoragePanel";
import { getStorageStatus } from "../shared/ipc/commands";
import { useIpcResource } from "../shared/ipc/useIpcResource";

export function App() {
  const storage = useIpcResource(getStorageStatus);
  const ready =
    storage.state.status === "ready" && storage.state.data.status === "ready";

  return (
    <div className="shell">
      <header className="shell__header">
        <p className="shell__brand" aria-hidden="true">
          Kanban IA
          <span className="shell__cursor" />
        </p>
      </header>

      {/* Sin almacenamiento listo no se muestran datos: solo el estado y la opción de reintentar. */}
      {ready ? (
        <Workspace />
      ) : (
        <main className="view">
          <h1 className="view__title">Kanban IA</h1>
          <StoragePanel {...storage} />
        </main>
      )}

      {ready && (
        <details className="system">
          <summary>Sistema</summary>
          <div className="shell__panels">
            <StoragePanel {...storage} />
            <AppInfoPanel />
          </div>
        </details>
      )}
    </div>
  );
}
