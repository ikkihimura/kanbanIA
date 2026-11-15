import { useState } from "react";
import { AppInfoPanel } from "../features/app-info/AppInfoPanel";
import { BackupPanel } from "../features/backup/BackupPanel";
import { RunSettingsPanel } from "../features/runs/RunSettingsPanel";
import { CloseConfirmDialog } from "./CloseConfirmDialog";
import { Workspace } from "../features/projects/Workspace";
import { StoragePanel } from "../features/storage-status/StoragePanel";
import { getStorageStatus } from "../shared/ipc/commands";
import { useIpcResource } from "../shared/ipc/useIpcResource";
import { useBlockExternalDrops } from "../shared/ui/useBlockExternalDrops";

export function App() {
  useBlockExternalDrops();
  const storage = useIpcResource(getStorageStatus);
  const ready =
    storage.state.status === "ready" && storage.state.data.status === "ready";
  // Tras restaurar una copia (o intentarlo), todo se vuelve a cargar desde la base vigente.
  const [generation, setGeneration] = useState(0);
  const [restoring, setRestoring] = useState(false);
  const [notice, setNotice] = useState("");
  const backupProps = {
    onRestoring: setRestoring,
    onFinished: (message: string | null) => {
      setRestoring(false);
      setGeneration((g) => g + 1);
      storage.run(getStorageStatus, { keepData: true });
      if (message) setNotice(message);
    },
  };

  return (
    <div className="shell">
      <CloseConfirmDialog />
      <header className="shell__header">
        <p className="shell__brand" aria-hidden="true">
          Kanban IA
          <span className="shell__cursor" />
        </p>
      </header>

      <p role="status" className={notice ? "notice" : "visually-hidden"}>
        {notice && `✓ ${notice}`}
      </p>

      {/* Sin almacenamiento listo no se muestran datos: solo el estado y la opción de reintentar. */}
      {ready ? (
        // Durante la restauración no se puede interactuar: ninguna escritura llega a mitad.
        <div inert={restoring} aria-busy={restoring}>
          <Workspace key={generation} />
        </div>
      ) : (
        <main className="view">
          <h1 className="view__title">Kanban IA</h1>
          <StoragePanel {...storage} />
          {storage.state.status === "ready" && (
            <BackupPanel canCreate={false} {...backupProps} />
          )}
        </main>
      )}

      {ready && (
        <details className="system">
          <summary>Sistema</summary>
          <div className="shell__panels">
            <StoragePanel {...storage} />
            <BackupPanel canCreate {...backupProps} />
            <RunSettingsPanel key={generation} />
            <AppInfoPanel />
          </div>
        </details>
      )}
    </div>
  );
}
