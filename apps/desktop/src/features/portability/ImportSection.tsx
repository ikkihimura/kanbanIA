import { useId, useState } from "react";
import type { ImportAction } from "../../contracts/generated/ImportAction";
import type { ImportItem } from "../../contracts/generated/ImportItem";
import type { ImportPreview } from "../../contracts/generated/ImportPreview";
import type { ResolutionChoice } from "../../contracts/generated/ResolutionChoice";
import { applyImport, previewImport } from "../../shared/ipc/commands";
import { useMutation } from "../../shared/ipc/useMutation";
import { chooseFolder } from "./chooseFolder";

const ACTION_LABEL: Record<ImportAction, string> = {
  create: "Se crea",
  update: "Se actualiza con el paquete",
  unchanged: "Sin cambios",
  keep_local: "Se conserva lo local (más reciente)",
  conflict: "Conflicto: elige",
};

interface Props {
  /** Importación aplicada: abrir el proyecto con un aviso del resultado. */
  onImported: (projectId: string, notice: string) => void;
}

/**
 * Importar un paquete Markdown en dos pasos: vista previa con validación y, después,
 * aplicar en una transacción. Los conflictos se resuelven uno a uno antes de aplicar.
 */
export function ImportSection({ onImported }: Props) {
  const id = useId();
  const [path, setPath] = useState<string | null>(null);
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [choices, setChoices] = useState<Record<string, ResolutionChoice>>({});
  const previewer = useMutation(previewImport);
  const applier = useMutation(applyImport);

  async function load(folder: string) {
    setPreview(null);
    setChoices({});
    applier.reset();
    const result = await previewer.mutate(folder);
    if (result.ok) setPreview(result.value);
  }

  async function choose() {
    const folder = await chooseFolder(
      "Elige la carpeta del paquete a importar",
    );
    if (folder === null) return;
    setPath(folder);
    await load(folder);
  }

  function cancel() {
    setPath(null);
    setPreview(null);
    setChoices({});
    previewer.reset();
    applier.reset();
  }

  const conflicts = preview?.items.filter((i) => i.action === "conflict") ?? [];
  const resolved = conflicts.every((c) => choices[c.id] !== undefined);
  const counts = (preview?.items ?? []).reduce<
    Partial<Record<ImportAction, number>>
  >(
    (acc, item) => ({ ...acc, [item.action]: (acc[item.action] ?? 0) + 1 }),
    {},
  );

  async function handleApply() {
    if (!preview || path === null) return;
    const result = await applier.mutate({
      path,
      fingerprint: preview.fingerprint,
      resolutions: conflicts.flatMap((c) => {
        const choice = choices[c.id];
        return choice ? [{ id: c.id, choice }] : [];
      }),
    });
    if (result.ok) {
      const r = result.value;
      onImported(
        r.projectId,
        `Importación aplicada: ${String(r.created)} creadas, ${String(r.updated)} actualizadas, ${String(r.unchanged)} sin cambios, ${String(r.keptLocal)} conservadas.`,
      );
    }
  }

  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="panel__title">
        Importar paquete
      </h2>
      <p className="field__hint">
        Elige una carpeta exportada. Primero verás qué cambiará; nada se guarda
        hasta que pulses «Aplicar importación». Nunca se borran datos locales ni
        se importan aceptaciones.
      </p>
      <div className="repo__actions">
        <button
          type="button"
          className="button button--secondary"
          aria-busy={previewer.pending}
          disabled={previewer.pending || applier.pending}
          onClick={() => {
            void choose();
          }}
        >
          Elegir paquete…
        </button>
        {path !== null && (
          <button
            type="button"
            className="button button--secondary"
            disabled={applier.pending}
            onClick={cancel}
          >
            Cancelar
          </button>
        )}
      </div>
      {previewer.pending && (
        <p className="panel__muted" role="status">
          Leyendo y validando el paquete…
        </p>
      )}
      {previewer.error && (
        <p className="field__error" role="alert">
          No se puede importar: {previewer.error.message}
        </p>
      )}

      {preview && (
        <div className="import-preview">
          <h3 className="relations__title">
            {preview.projectExists
              ? `Actualizar «${preview.projectName}»`
              : `Crear el proyecto «${preview.projectName}»`}
          </h3>
          <p role="status">
            Vista previa lista.{" "}
            {(
              [
                "create",
                "update",
                "conflict",
                "keep_local",
                "unchanged",
              ] as const
            )
              .filter((a) => counts[a])
              .map((a) => `${ACTION_LABEL[a]}: ${String(counts[a])}`)
              .join(" · ")}
          </p>
          {preview.warnings.length > 0 && (
            <ul className="import-preview__warnings">
              {preview.warnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          )}

          {conflicts.map((item) => (
            <ConflictChoice
              key={item.id}
              item={item}
              value={choices[item.id]}
              onChange={(choice) => {
                setChoices({ ...choices, [item.id]: choice });
                applier.reset();
              }}
            />
          ))}

          <details>
            <summary>Ver los {preview.items.length} elementos</summary>
            <ul className="item-list">
              {preview.items.map((item) => (
                <li key={item.id} className="import-preview__item">
                  <span>{item.label}</span>
                  <span className="item-list__meta">
                    {ACTION_LABEL[item.action]}
                  </span>
                </li>
              ))}
            </ul>
          </details>

          {applier.error && (
            <div role="alert">
              <p className="field__error">{applier.error.message}</p>
              {applier.error.code === "validation" && path !== null && (
                <button
                  type="button"
                  className="button button--secondary"
                  onClick={() => {
                    void load(path);
                  }}
                >
                  Volver a previsualizar
                </button>
              )}
            </div>
          )}
          <button
            type="button"
            className="button"
            aria-busy={applier.pending}
            aria-describedby={resolved ? undefined : `${id}-pending`}
            disabled={!resolved || applier.pending}
            onClick={() => {
              void handleApply();
            }}
          >
            Aplicar importación
          </button>
          {!resolved && (
            <p id={`${id}-pending`} className="field__hint">
              Elige una opción en cada conflicto para poder aplicar.
            </p>
          )}
        </div>
      )}
    </section>
  );
}

function ConflictChoice({
  item,
  value,
  onChange,
}: {
  item: ImportItem;
  value: ResolutionChoice | undefined;
  onChange: (choice: ResolutionChoice) => void;
}) {
  const name = useId();
  const kind = item.kind === "story" ? "Historia" : "Épica";
  return (
    <fieldset className="field import-preview__conflict">
      <legend>
        Conflicto en {kind.toLowerCase()} {item.label}: cambió aquí y en el
        paquete
      </legend>
      <label className="checkbox">
        <input
          type="radio"
          name={name}
          checked={value === "local"}
          onChange={() => {
            onChange("local");
          }}
        />
        Conservar la versión local
        {item.localLabel ? ` (${item.localLabel})` : ""}
      </label>
      <label className="checkbox">
        <input
          type="radio"
          name={name}
          checked={value === "package"}
          onChange={() => {
            onChange("package");
          }}
        />
        Usar la del paquete ({item.label})
      </label>
    </fieldset>
  );
}
