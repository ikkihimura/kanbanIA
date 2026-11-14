import { useEffect, useId, useRef, useState } from "react";
import type { Agent } from "../../contracts/generated/Agent";
import type { AgentRef } from "../../contracts/generated/AgentRef";
import type { CreateRunInput } from "../../contracts/generated/CreateRunInput";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import {
  createRun,
  newRequestId,
  type UiError,
} from "../../shared/ipc/commands";
import { useMutation } from "../../shared/ipc/useMutation";
import { FieldError } from "../../shared/ui/FieldError";

/** Igual que `MAX_NOTE_CHARS` del núcleo (cuenta caracteres, no unidades UTF-16). */
export const MAX_NOTE_CHARS = 4000;

const count = new Intl.NumberFormat("es");

/** Puntos de código, como `chars().count()` en el núcleo (no unidades UTF-16 ni grafemas). */
function countChars(text: string): number {
  let n = 0;
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    // Un par sustituto (alto + bajo) es un solo punto de código.
    if (code >= 0xdc00 && code <= 0xdfff && i > 0) {
      const prev = text.charCodeAt(i - 1);
      if (prev >= 0xd800 && prev <= 0xdbff) continue;
    }
    n += 1;
  }
  return n;
}

interface Props {
  projectId: string;
  storyId: string;
  /**
   * Agente asignado a la historia (opción por defecto); `null` si no hay y `undefined` si aún no
   * se sabe (cargando o error).
   */
  assigned: AgentRef | null | undefined;
  /** Agentes del equipo; `null` mientras cargan o si fallaron. */
  agents: Agent[] | null;
  onCreated: (run: RunSummary) => void;
  /** Intento activo de la historia según la última lista cargada. */
  activeRun: RunSummary | null;
  /** Abrir el detalle de un intento (acción de la alerta `run_active`). */
  onShowRun: (runId: string) => void;
  /** El núcleo dijo que ya hay un intento activo: hay que recargar la lista. */
  onRunActive: () => void;
  /** Hay texto o agente elegidos sin enviar (para avisar antes de salir). */
  onDirtyChange: (dirty: boolean) => void;
}

/** Preparar un intento (HU-017): agente y nota opcional; el núcleo genera las instrucciones. */
export function PrepareRunForm({
  projectId,
  storyId,
  assigned,
  agents,
  onCreated,
  activeRun,
  onShowRun,
  onRunActive,
  onDirtyChange,
}: Props) {
  const id = useId();
  const [agentId, setAgentId] = useState("");
  const [note, setNote] = useState("");
  // Se conserva al reintentar el mismo envío; cambia al editar o tras crear.
  const requestId = useRef(newRequestId());
  const create = useMutation(createRun);
  const touch = () => {
    requestId.current = newRequestId();
    create.reset();
  };

  const dirty = agentId !== "" || note.trim() !== "";
  useEffect(() => {
    onDirtyChange(dirty);
  }, [dirty, onDirtyChange]);
  useEffect(
    () => () => {
      onDirtyChange(false);
    },
    [onDirtyChange],
  );

  const noteChars = countChars(note);
  const tooLong = noteChars > MAX_NOTE_CHARS;
  const error = create.error;
  const noteError = error?.field === "note" ? error : null;
  const agentError = error?.field === "agentId" ? error : null;
  const generalError: UiError | null =
    error && !noteError && !agentError ? error : null;

  // El agente asignado es la opción por defecto («»: el núcleo usa el asignado).
  // Lo elegido sigue en la lista aunque pase a ser el asignado: lo enviado es lo que se ve.
  const others = (agents ?? []).filter(
    (a) => a.id !== assigned?.id || a.id === agentId,
  );

  async function handleSubmit() {
    if (tooLong) return;
    const input: CreateRunInput = {
      requestId: requestId.current,
      projectId,
      storyId,
    };
    if (agentId !== "") input.agentId = agentId;
    if (note.trim() !== "") input.note = note;
    const result = await create.mutate(input);
    if (result.ok) {
      requestId.current = newRequestId();
      setAgentId("");
      setNote("");
      onCreated(result.value);
    } else if (result.error.code === "run_active") {
      onRunActive();
    }
  }

  const noteDescribedBy = [
    `${id}-note-hint`,
    `${id}-note-count`,
    noteError ? `${id}-note-error` : null,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <form
      className="team-form"
      aria-label="Preparar intento"
      onSubmit={(event) => {
        event.preventDefault();
        void handleSubmit();
      }}
    >
      <div className="field">
        <label htmlFor={`${id}-agent`}>Agente del intento</label>
        <select
          id={`${id}-agent`}
          value={agentId}
          aria-invalid={agentError ? true : undefined}
          aria-describedby={agentError ? `${id}-agent-error` : undefined}
          onChange={(event) => {
            setAgentId(event.target.value);
            touch();
          }}
        >
          <option value="">
            {assigned === undefined
              ? "Asignado a la historia"
              : assigned === null
                ? "Asignado a la historia (no hay ninguno)"
                : `Asignado a la historia: A-${String(assigned.number)} ${assigned.name}`}
          </option>
          {others.map((a) => (
            <option key={a.id} value={a.id}>
              A-{a.number} {a.name}
            </option>
          ))}
        </select>
        <FieldError id={`${id}-agent-error`} error={agentError} />
      </div>

      <div className="field">
        <label htmlFor={`${id}-note`}>Nota para el agente (opcional)</label>
        <textarea
          id={`${id}-note`}
          rows={3}
          value={note}
          aria-invalid={noteError || tooLong ? true : undefined}
          aria-describedby={noteDescribedBy}
          onChange={(event) => {
            setNote(event.target.value);
            touch();
          }}
        />
        <span id={`${id}-note-hint`} className="field__hint">
          Markdown. Se añade a las instrucciones que genera la app. No incluyas
          credenciales.
        </span>
        <span
          id={`${id}-note-count`}
          className={tooLong ? "field__error" : "field__hint"}
        >
          {count.format(noteChars)} / {count.format(MAX_NOTE_CHARS)} caracteres
          {tooLong &&
            `: sobran ${count.format(noteChars - MAX_NOTE_CHARS)}. Acórtala para preparar el intento.`}
        </span>
        <FieldError id={`${id}-note-error`} error={noteError} />
      </div>

      {generalError && (
        <div className="run-form__alert" role="alert">
          <p>No se pudo preparar el intento: {generalError.message}</p>
          {generalError.code === "run_active" && activeRun && (
            <button
              type="button"
              className="button button--secondary"
              onClick={() => {
                onShowRun(activeRun.id);
              }}
            >
              Ver el intento #{activeRun.number} activo
            </button>
          )}
        </div>
      )}

      <div>
        <button
          type="submit"
          className="button"
          aria-busy={create.pending}
          disabled={create.pending || tooLong}
        >
          Preparar intento
        </button>
      </div>
    </form>
  );
}
