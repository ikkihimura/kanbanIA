import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { Agent } from "../../contracts/generated/Agent";
import type { AgentFields } from "../../contracts/generated/AgentFields";
import type { AvatarSpec } from "../../contracts/generated/AvatarSpec";
import type { Capability } from "../../contracts/generated/Capability";
import type { UserProfile } from "../../contracts/generated/UserProfile";
import {
  createAgent,
  getUserProfile,
  listAgents,
  newRequestId,
  updateAgent,
  updateUserProfile,
  type UiError,
} from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { AvatarEditor } from "./AvatarEditor";
import { PixelAvatar } from "./PixelAvatar";

export const CAPABILITY_LABEL: Record<Capability, string> = {
  plan: "Planificar",
  implement: "Implementar",
  review: "Revisar",
  test: "Probar",
  document: "Documentar",
};
const CAPABILITIES = Object.keys(CAPABILITY_LABEL) as Capability[];

const NEW_AGENT_AVATAR: AvatarSpec = {
  head: 2,
  skin: 6,
  eyes: 4,
  mouth: 3,
  hair: 6,
  hairColor: 7,
  accessory: 5,
  background: 3,
};

function emptyAgent(): AgentFields {
  return {
    name: "",
    role: "",
    provider: "",
    model: "",
    capabilities: [],
    avatar: NEW_AGENT_AVATAR,
  };
}

export function TeamView() {
  const titleRef = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    titleRef.current?.focus();
  }, []);

  return (
    <main className="view">
      <h1 className="view__title" ref={titleRef} tabIndex={-1}>
        Equipo
      </h1>
      <UserSection />
      <AgentsSection />
    </main>
  );
}

function UserSection() {
  const { state } = useIpcResource(getUserProfile);
  if (state.status === "loading") {
    return <p className="panel__muted">Cargando tu perfil…</p>;
  }
  if (state.status === "error") {
    return <p className="panel__error">{state.message}</p>;
  }
  // Sin `key` por revisión: guardar no vuelve a montar el formulario (se conservan foco y aviso).
  return <UserForm profile={state.data} />;
}

/** Revisión vigente que trae un error de conflicto, si la hay. */
function conflictRevision(error: UiError): number | undefined {
  const current = error.current;
  if (current && typeof current === "object" && "revision" in current) {
    const revision = (current as { revision: unknown }).revision;
    if (typeof revision === "number") return revision;
  }
  return undefined;
}

const CONFLICT_HINT =
  "Se guardó otra versión mientras editabas. Tus cambios siguen aquí: pulsa guardar de nuevo para conservarlos.";

function UserForm({ profile }: { profile: UserProfile }) {
  const [name, setName] = useState(profile.name);
  const [avatar, setAvatar] = useState(profile.avatar);
  const [revision, setRevision] = useState(profile.revision);
  const [message, setMessage] = useState("");
  const save = useMutation(updateUserProfile);

  async function handleSave() {
    const result = await save.mutate({
      expectedRevision: revision,
      name,
      avatar,
    });
    if (result.ok) {
      setRevision(result.value.revision);
      setName(result.value.name);
      setMessage("Perfil guardado.");
    } else if (result.error.code === "conflict") {
      // Se adopta la revisión vigente sin tocar lo que el usuario estaba editando.
      const current = conflictRevision(result.error);
      if (current !== undefined) setRevision(current);
      setMessage("");
    }
  }

  return (
    <section className="panel" aria-labelledby="user-title">
      <h2 id="user-title" className="panel__title">
        Tú
      </h2>
      <p className="panel__hint">
        Tu perfil es de persona: los agentes son entidades aparte.
      </p>
      <form
        className="team-form"
        aria-label="Tu perfil"
        onSubmit={(event) => {
          event.preventDefault();
          void handleSave();
        }}
      >
        <div className="field">
          <label htmlFor="user-name">Nombre</label>
          <input
            id="user-name"
            value={name}
            maxLength={60}
            autoComplete="off"
            onChange={(event) => {
              setName(event.target.value);
              setMessage("");
              save.reset();
            }}
          />
        </div>
        <AvatarEditor
          value={avatar}
          name={name}
          onChange={(next) => {
            setAvatar(next);
            setMessage("");
          }}
        />
        <ErrorLine error={save.error} />
        {save.error?.code === "conflict" && (
          <p className="field__error">{CONFLICT_HINT}</p>
        )}
        <p role="status" className="relations__saved">
          {message && `✓ ${message}`}
        </p>
        <div className="repo__actions">
          <button
            type="submit"
            className="button"
            aria-busy={save.pending}
            disabled={save.pending}
          >
            Guardar perfil
          </button>
        </div>
      </form>
    </section>
  );
}

function AgentsSection() {
  const { state, run } = useIpcResource(listAgents);
  const [creating, setCreating] = useState(false);
  const [message, setMessage] = useState("");
  const reload = useCallback(() => {
    run(undefined, { keepData: true });
  }, [run]);

  return (
    <section className="panel" aria-labelledby="agents-title">
      <h2 id="agents-title" className="panel__title">
        Agentes
      </h2>
      <p className="panel__hint">
        Proveedor y modelo son informativos: no conectan con ningún servicio.
        Asignar un agente a una historia no inicia ninguna ejecución.
      </p>
      <p role="status" className="relations__saved">
        {message && `✓ ${message}`}
      </p>

      {state.status === "loading" && (
        <p className="panel__muted">Cargando agentes…</p>
      )}
      {state.status === "error" && (
        <p className="panel__error">{state.message}</p>
      )}
      {state.status === "ready" && state.data.length === 0 && !creating && (
        <p className="panel__muted">Aún no hay agentes.</p>
      )}

      {state.status === "ready" && state.data.length > 0 && (
        <ul className="item-list">
          {state.data.map((agent) => (
            <AgentItem
              key={agent.id}
              agent={agent}
              onSaved={(text) => {
                setMessage(text);
                reload();
              }}
            />
          ))}
        </ul>
      )}

      {creating ? (
        <AgentForm
          title="Nuevo agente"
          initial={emptyAgent()}
          submitLabel="Crear agente"
          submit={async (fields, requestId) =>
            createAgent({ requestId, ...fields })
          }
          onDone={(agent) => {
            setCreating(false);
            if (agent) {
              setMessage(
                `Agente A-${String(agent.number)} ${agent.name} creado.`,
              );
              reload();
            }
          }}
        />
      ) : (
        <button
          type="button"
          className="button form-row--spaced"
          onClick={() => {
            setMessage("");
            setCreating(true);
          }}
        >
          Nuevo agente
        </button>
      )}
    </section>
  );
}

function AgentItem({
  agent,
  onSaved,
}: {
  agent: Agent;
  onSaved: (message: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  // Revisión con la que se guarda; ante un conflicto se adopta la vigente.
  const [revision, setRevision] = useState(agent.revision);
  const label = `A-${String(agent.number)} ${agent.name}`;

  if (editing) {
    return (
      <li className="agent-card">
        <AgentForm
          title={`Editar ${label}`}
          initial={{
            name: agent.name,
            role: agent.role,
            provider: agent.provider,
            model: agent.model,
            capabilities: agent.capabilities,
            avatar: agent.avatar,
          }}
          submitLabel="Guardar agente"
          submit={async (fields) =>
            updateAgent({
              id: agent.id,
              expectedRevision: revision,
              ...fields,
            })
          }
          onConflict={setRevision}
          onDone={(saved) => {
            setEditing(false);
            if (saved) onSaved(`Agente ${label} guardado.`);
          }}
        />
      </li>
    );
  }

  return (
    <li className="agent-card">
      <PixelAvatar spec={agent.avatar} name={agent.name} size={64} decorative />
      <div className="agent-card__body">
        <span className="item-list__name">
          <span className="story-number">A-{agent.number}</span> {agent.name}
        </span>
        <span className="item-list__meta">{agent.role || "Sin rol"}</span>
        <dl className="facts facts--compact">
          <dt>Proveedor</dt>
          <dd>{agent.provider || "—"}</dd>
          <dt>Modelo</dt>
          <dd>{agent.model || "—"}</dd>
          <dt>Capacidades</dt>
          <dd>
            {agent.capabilities.length === 0
              ? "—"
              : agent.capabilities.map((c) => CAPABILITY_LABEL[c]).join(", ")}
          </dd>
          <dt>Asignado a</dt>
          <dd>
            {agent.assignedStories === 1
              ? "1 historia"
              : `${String(agent.assignedStories)} historias`}{" "}
            (sin actividad registrada)
          </dd>
        </dl>
      </div>
      <button
        type="button"
        className="button button--secondary"
        aria-label={`Editar ${label}`}
        onClick={() => {
          setRevision(agent.revision);
          setEditing(true);
        }}
      >
        Editar
      </button>
    </li>
  );
}

function AgentForm({
  title,
  initial,
  submitLabel,
  submit,
  onDone,
  onConflict,
}: {
  title: string;
  initial: AgentFields;
  submitLabel: string;
  submit: (fields: AgentFields, requestId: string) => Promise<Agent>;
  onDone: (agent: Agent | null) => void;
  /** Conflicto de revisión: recibe la revisión vigente; el borrador se conserva. */
  onConflict?: (revision: number) => void;
}) {
  const formId = useId();
  const [fields, setFields] = useState(initial);
  const requestId = useRef(newRequestId());
  const save = useMutation(
    ({ value, rid }: { value: AgentFields; rid: string }) => submit(value, rid),
  );
  const update = (next: Partial<AgentFields>) => {
    setFields((current) => ({ ...current, ...next }));
    requestId.current = newRequestId();
    save.reset();
  };
  const fieldError = (field: string) =>
    save.error?.field === field ? save.error : null;

  async function handleSubmit() {
    const result = await save.mutate({ value: fields, rid: requestId.current });
    if (result.ok) {
      onDone(result.value);
    } else if (result.error.code === "conflict") {
      const current = conflictRevision(result.error);
      if (current !== undefined) onConflict?.(current);
    }
  }

  const text = (
    field: "name" | "role" | "provider" | "model",
    label: string,
    max: number,
    placeholder?: string,
  ) => (
    <div className="field">
      <label htmlFor={`${formId}-${field}`}>{label}</label>
      <input
        id={`${formId}-${field}`}
        value={fields[field]}
        maxLength={max}
        placeholder={placeholder}
        autoComplete="off"
        aria-invalid={fieldError(field) ? true : undefined}
        onChange={(event) => {
          update({ [field]: event.target.value });
        }}
      />
      <ErrorLine error={fieldError(field)} />
    </div>
  );

  return (
    <form
      className="team-form"
      aria-label={title}
      onSubmit={(event) => {
        event.preventDefault();
        void handleSubmit();
      }}
    >
      <h3 className="relations__title">{title}</h3>
      <div className="form-row">
        {text("name", "Nombre (identidad estable)", 40)}
        {text("role", "Rol", 60, "p. ej. Implementación backend")}
        {text("provider", "Proveedor (informativo)", 60)}
        {text("model", "Modelo (informativo)", 80)}
      </div>
      <fieldset className="field">
        <legend>Capacidades</legend>
        <div className="checkbox-row">
          {CAPABILITIES.map((capability) => (
            <label key={capability} className="checkbox">
              <input
                type="checkbox"
                checked={fields.capabilities.includes(capability)}
                onChange={(event) => {
                  update({
                    capabilities: event.target.checked
                      ? [...fields.capabilities, capability]
                      : fields.capabilities.filter((c) => c !== capability),
                  });
                }}
              />
              {CAPABILITY_LABEL[capability]}
            </label>
          ))}
        </div>
      </fieldset>
      <AvatarEditor
        value={fields.avatar}
        name={fields.name}
        onChange={(avatar) => {
          update({ avatar });
        }}
      />
      {save.error?.code === "conflict" && (
        <p className="field__error">{CONFLICT_HINT}</p>
      )}
      <ErrorLine
        error={
          save.error &&
          !["name", "role", "provider", "model"].includes(
            save.error.field ?? "",
          )
            ? save.error
            : null
        }
      />
      <div className="repo__actions">
        <button
          type="submit"
          className="button"
          aria-busy={save.pending}
          disabled={save.pending}
        >
          {submitLabel}
        </button>
        <button
          type="button"
          className="button button--secondary"
          onClick={() => {
            onDone(null);
          }}
        >
          Cancelar
        </button>
      </div>
    </form>
  );
}

function ErrorLine({ error }: { error: UiError | null }) {
  if (!error) return null;
  return (
    <p className="field__error" role="alert">
      {error.message}
    </p>
  );
}
