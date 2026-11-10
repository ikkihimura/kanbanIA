import { useId, useState } from "react";
import type { ExportResult } from "../../contracts/generated/ExportResult";
import { exportProject } from "../../shared/ipc/commands";
import { useMutation } from "../../shared/ipc/useMutation";
import { chooseFolder } from "./chooseFolder";

interface Props {
  projectId: string;
}

/** Exportar la planificación del proyecto a una carpeta Markdown (HU-010). */
export function ExportSection({ projectId }: Props) {
  const id = useId();
  const exporter = useMutation(exportProject);
  const [done, setDone] = useState<{
    result: ExportResult;
    destination: string;
  } | null>(null);

  async function handleExport() {
    setDone(null);
    const destination = await chooseFolder(
      "Elige dónde crear la carpeta del paquete",
    );
    if (destination === null) return;
    const result = await exporter.mutate({ projectId, destination });
    if (result.ok) setDone({ result: result.value, destination });
  }

  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="panel__title">
        Exportar
      </h2>
      <p className="field__hint">
        Crea una carpeta con un Markdown por historia y por épica y un
        manifiesto. Incluye criterios, relaciones, columnas y, como dato
        informativo, aceptación, motivo de bloqueo y agente asignado (id y
        nombre). No incluye rutas de repositorios ni entregas. Es una copia
        puntual: no se sincroniza sola.
      </p>
      <button
        type="button"
        className="button button--secondary"
        aria-busy={exporter.pending}
        disabled={exporter.pending}
        onClick={() => {
          void handleExport();
        }}
      >
        Exportar a una carpeta…
      </button>
      <p role="status" className="relations__saved">
        {done &&
          `✓ Exportado en «${done.result.folderName}» (${String(done.result.stories)} historias, ${String(done.result.epics)} épicas) dentro de ${done.destination}.`}
      </p>
      {exporter.error && (
        <p className="field__error" role="alert">
          {exporter.error.message}
        </p>
      )}
    </section>
  );
}
