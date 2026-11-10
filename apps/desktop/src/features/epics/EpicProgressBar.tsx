import type { EpicProgress } from "../../contracts/generated/EpicProgress";

/** Texto del avance según la regla del núcleo (`percent` nulo = sin historias). */
export function progressLabel(progress: EpicProgress): string {
  if (progress.percent === null) return "Sin historias";
  return `${String(progress.accepted)} de ${String(progress.total)} aceptadas (${String(progress.percent)} %)`;
}

/** Barra nativa (accesible por defecto); sin historias solo se muestra el texto. */
export function EpicProgressBar({
  progress,
  label,
}: {
  progress: EpicProgress;
  label: string;
}) {
  const text = progressLabel(progress);
  return (
    <div className="progress">
      {progress.percent !== null && (
        <progress
          className="progress__bar"
          aria-label={`Avance de ${label}`}
          value={progress.accepted}
          max={progress.total}
        />
      )}
      <span className="item-list__meta">{text}</span>
    </div>
  );
}
