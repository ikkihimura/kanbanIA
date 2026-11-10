import { useCallback, useRef, useState } from "react";
import {
  linkRepository,
  listRepositories,
  newRequestId,
} from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { FolderField } from "./FolderField";
import { RepositoryItem } from "./RepositoryItem";

interface Props {
  projectId: string;
}

export function RepositoriesSection({ projectId }: Props) {
  const load = useCallback(() => listRepositories(projectId), [projectId]);
  const { state, run } = useIpcResource(load);
  const [path, setPath] = useState("");
  const requestId = useRef(newRequestId());
  const link = useMutation(linkRepository);
  const [announcement, setAnnouncement] = useState("");
  const titleRef = useRef<HTMLHeadingElement>(null);

  // Recarga conservando la lista visible: sin parpadeo ni anuncio de "comprobando".
  const reload = useCallback(() => {
    run(load, { keepData: true });
  }, [run, load]);

  async function handleLink() {
    const result = await link.mutate({
      requestId: requestId.current,
      projectId,
      path,
    });
    if (result.ok) {
      requestId.current = newRequestId();
      setPath("");
      setAnnouncement(`Repositorio «${result.value.name}» vinculado.`);
      reload();
    }
  }

  return (
    <section className="panel" aria-labelledby="repositories-title">
      <h2
        id="repositories-title"
        className="panel__title"
        ref={titleRef}
        tabIndex={-1}
      >
        Repositorios
      </h2>

      <form
        className="form-row"
        aria-label="Vincular repositorio"
        onSubmit={(event) => {
          event.preventDefault();
          void handleLink();
        }}
      >
        <FolderField
          label="Carpeta del repositorio"
          value={path}
          error={link.error}
          onChange={(value) => {
            setPath(value);
            requestId.current = newRequestId();
            link.reset();
          }}
        />
        <button type="submit" className="button" disabled={link.pending}>
          {link.pending ? "Vinculando…" : "Vincular"}
        </button>
      </form>
      <p className="panel__hint">
        Kanban IA solo lee el repositorio para comprobarlo; nunca modifica sus
        archivos.
      </p>

      <div role="status" aria-live="polite" className="visually-hidden">
        {announcement}
      </div>
      <div role="status" aria-live="polite">
        {state.status === "loading" && (
          <p className="panel__muted">Comprobando repositorios…</p>
        )}
        {state.status === "error" && (
          <p className="panel__error">{state.message}</p>
        )}
        {state.status === "ready" && state.data.length === 0 && (
          <p className="panel__muted">
            Este proyecto aún no tiene repositorios vinculados.
          </p>
        )}
      </div>

      {state.status === "ready" && state.data.length > 0 && (
        <ul className="item-list">
          {state.data.map((repository) => (
            <RepositoryItem
              key={repository.id}
              projectId={projectId}
              repository={repository}
              onChanged={(message) => {
                setAnnouncement(message);
                reload();
                // El elemento puede desaparecer (desvincular): llevar el foco a un lugar estable.
                titleRef.current?.focus();
              }}
              onStale={reload}
            />
          ))}
        </ul>
      )}
    </section>
  );
}
