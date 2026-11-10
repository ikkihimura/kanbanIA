import { useEffect, useState } from "react";
import type { RepositoryLink } from "../../contracts/generated/RepositoryLink";
import type { RepositoryStatus } from "../../contracts/generated/RepositoryStatus";
import {
  relocateRepository,
  unlinkRepository,
} from "../../shared/ipc/commands";
import { useMutation } from "../../shared/ipc/useMutation";
import { FolderField } from "./FolderField";

const STATUS_LABEL: Record<RepositoryStatus, string> = {
  available: "Disponible",
  missing: "No encontrado: ¿se movió o se borró?",
  not_a_repository: "La carpeta ya no es un repositorio Git",
  inaccessible: "Sin permiso de acceso",
};

interface Props {
  projectId: string;
  repository: RepositoryLink;
  onChanged: (announcement: string) => void;
  /** Los datos mostrados están desactualizados (conflicto o ya no existe): recargar. */
  onStale: () => void;
}

type Mode = "idle" | "relocating" | "confirm-unlink";

export function RepositoryItem({
  projectId,
  repository,
  onChanged,
  onStale,
}: Props) {
  const [mode, setMode] = useState<Mode>("idle");
  const [path, setPath] = useState(repository.path);
  const relocate = useMutation(relocateRepository);
  const unlink = useMutation(unlinkRepository);
  const healthy = repository.status === "available";

  // Tras un conflicto, recargar para que el siguiente intento use la revisión vigente.
  const staleCode = relocate.error?.code ?? unlink.error?.code;
  useEffect(() => {
    if (staleCode === "conflict" || staleCode === "not_found") onStale();
  }, [staleCode, onStale]);

  async function handleRelocate() {
    const result = await relocate.mutate({
      projectId,
      id: repository.id,
      expectedRevision: repository.revision,
      path,
    });
    if (result.ok) onChanged(`Repositorio «${result.value.name}» reubicado.`);
  }

  async function handleUnlink() {
    const result = await unlink.mutate({
      projectId,
      id: repository.id,
      expectedRevision: repository.revision,
    });
    if (result.ok) onChanged(`Repositorio «${repository.name}» desvinculado.`);
  }

  return (
    <li className="repo">
      <div className="repo__summary">
        <span className="item-list__name">{repository.name}</span>
        <span className={`badge ${healthy ? "badge--ok" : "badge--warn"}`}>
          {STATUS_LABEL[repository.status]}
        </span>
      </div>
      <code className="repo__path">{repository.path}</code>

      {mode === "idle" && (
        <div className="repo__actions">
          <button
            type="button"
            className={`button ${healthy ? "button--secondary" : ""}`}
            onClick={() => {
              setPath(repository.path);
              setMode("relocating");
            }}
          >
            Reubicar…
          </button>
          <button
            type="button"
            className="button button--secondary"
            onClick={() => {
              setMode("confirm-unlink");
            }}
          >
            Desvincular
          </button>
        </div>
      )}

      {mode === "relocating" && (
        <form
          className="form-row"
          aria-label={`Reubicar ${repository.name}`}
          onSubmit={(event) => {
            event.preventDefault();
            void handleRelocate();
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") setMode("idle");
          }}
        >
          <FolderField
            label="Nueva ubicación"
            value={path}
            error={relocate.error}
            autoFocus
            onChange={(value) => {
              setPath(value);
              relocate.reset();
            }}
          />
          <button type="submit" className="button" disabled={relocate.pending}>
            Guardar
          </button>
          <button
            type="button"
            className="button button--secondary"
            onClick={() => {
              setMode("idle");
            }}
          >
            Cancelar
          </button>
        </form>
      )}

      {mode === "confirm-unlink" && (
        <div
          className="repo__confirm"
          role="group"
          aria-label="Confirmar desvinculación"
        >
          <p>
            ¿Desvincular «{repository.name}» de este proyecto? Los archivos del
            repositorio no se modifican.
          </p>
          {unlink.error && (
            <p className="field__error" role="alert">
              {unlink.error.message}
            </p>
          )}
          <div className="repo__actions">
            <button
              type="button"
              className="button button--danger"
              autoFocus
              disabled={unlink.pending}
              onClick={() => {
                void handleUnlink();
              }}
            >
              Desvincular
            </button>
            <button
              type="button"
              className="button button--secondary"
              onClick={() => {
                setMode("idle");
              }}
            >
              Cancelar
            </button>
          </div>
        </div>
      )}
    </li>
  );
}
