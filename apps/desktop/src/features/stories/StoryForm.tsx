import { useEffect, useId, useRef, useState } from "react";
import type { Priority } from "../../contracts/generated/Priority";
import type { StoryDetail } from "../../contracts/generated/StoryDetail";
import { newRequestId, updateStory } from "../../shared/ipc/commands";
import { useMutation } from "../../shared/ipc/useMutation";
import { MarkdownPreview } from "../../shared/ui/MarkdownPreview";
import { draftFrom, sameDraft, toUpdateInput, type StoryDraft } from "./draft";
import { PRIORITIES, PRIORITY_LABEL } from "./priorities";

const MAX_DESCRIPTION = 20_000;
const FIELDS = ["title", "description", "criteria"];

interface Props {
  story: StoryDetail;
  onSaved: (story: StoryDetail) => void;
  onDirtyChange: (dirty: boolean) => void;
}

/** Editor de una historia no archivada. El borrador nunca se pierde por un conflicto. */
export function StoryForm({ story, onSaved, onDirtyChange }: Props) {
  const id = useId();
  const [base, setBase] = useState(story);
  const [draft, setDraft] = useState<StoryDraft>(() => draftFrom(story));
  const [tab, setTab] = useState<"write" | "preview">("write");
  const [saved, setSaved] = useState(false);
  const [conflict, setConflict] = useState<StoryDetail | null>(null);
  const save = useMutation(updateStory);
  const lastCriterionRef = useRef<HTMLInputElement>(null);
  const focusNewCriterion = useRef(false);

  const dirty = !sameDraft(draft, draftFrom(base));
  useEffect(() => {
    onDirtyChange(dirty);
  }, [dirty, onDirtyChange]);

  useEffect(() => {
    if (focusNewCriterion.current) {
      focusNewCriterion.current = false;
      lastCriterionRef.current?.focus();
    }
  }, [draft.criteria.length]);

  function update(next: Partial<StoryDraft>) {
    setDraft((current) => ({ ...current, ...next }));
    setSaved(false);
    save.reset();
  }

  async function submit(against: StoryDetail) {
    const result = await save.mutate(toUpdateInput(against, draft));
    if (result.ok) {
      setBase(result.value);
      setDraft(draftFrom(result.value));
      setConflict(null);
      setSaved(true);
      onSaved(result.value);
    } else if (result.error.code === "conflict" && result.error.current) {
      // La versión vigente se muestra junto al borrador, que se conserva intacto.
      setConflict(result.error.current as unknown as StoryDetail);
    }
  }

  const fieldError = (field: string) =>
    save.error?.field === field ? save.error : null;
  const generalError =
    save.error &&
    save.error.code !== "conflict" &&
    !FIELDS.includes(save.error.field ?? "")
      ? save.error
      : null;
  const describedBy = (field: string) =>
    fieldError(field) ? `${id}-${field}-error` : undefined;

  return (
    <form
      className="story-form"
      aria-label="Editar historia"
      onSubmit={(event) => {
        event.preventDefault();
        void submit(base);
      }}
    >
      <div role="status" aria-live="polite" className="story-form__status">
        {save.pending && "Guardando…"}
        {!save.pending && saved && !dirty && "Guardado ✓"}
        {!save.pending && dirty && !conflict && "Cambios sin guardar"}
      </div>

      {generalError && (
        <p className="field__error" role="alert">
          {generalError.message}
        </p>
      )}

      {conflict && (
        <section
          className="conflict"
          role="alert"
          aria-labelledby={`${id}-conflict-title`}
        >
          <h3 id={`${id}-conflict-title`} className="conflict__title">
            Otra edición guardó cambios mientras editabas
          </h3>
          <p>
            Tu borrador sigue intacto abajo. Esta es la versión guardada
            (revisión {conflict.revision}):
          </p>
          <div className="conflict__current">
            <p>
              <strong>{conflict.title}</strong> · prioridad{" "}
              {PRIORITY_LABEL[conflict.priority].toLowerCase()}
            </p>
            <MarkdownPreview source={conflict.description} />
            {conflict.criteria.length > 0 && (
              <ul>
                {conflict.criteria.map((c) => (
                  <li key={c.id}>
                    CA-{c.number}: {c.text}
                  </li>
                ))}
              </ul>
            )}
          </div>
          <div className="repo__actions">
            <button
              type="button"
              className="button"
              disabled={save.pending}
              onClick={() => {
                void submit(conflict);
              }}
            >
              Conservar mi versión
            </button>
            <button
              type="button"
              className="button button--secondary"
              onClick={() => {
                setBase(conflict);
                setDraft(draftFrom(conflict));
                setConflict(null);
                save.reset();
                onSaved(conflict);
              }}
            >
              Descartar mi borrador y usar la guardada
            </button>
          </div>
        </section>
      )}

      <div className="field">
        <label htmlFor={`${id}-title`}>Título</label>
        <input
          id={`${id}-title`}
          value={draft.title}
          maxLength={200}
          required
          aria-invalid={fieldError("title") ? true : undefined}
          aria-describedby={describedBy("title")}
          onChange={(event) => {
            update({ title: event.target.value });
          }}
        />
        <FieldMessage
          id={describedBy("title")}
          message={fieldError("title")?.message}
        />
      </div>

      <div className="field field--narrow">
        <label htmlFor={`${id}-priority`}>Prioridad</label>
        <select
          id={`${id}-priority`}
          value={draft.priority}
          onChange={(event) => {
            update({ priority: event.target.value as Priority });
          }}
        >
          {PRIORITIES.map((p) => (
            <option key={p} value={p}>
              {PRIORITY_LABEL[p]}
            </option>
          ))}
        </select>
      </div>

      <fieldset className="field">
        <legend>Descripción (Markdown)</legend>
        <div className="tabs" role="group" aria-label="Modo de la descripción">
          <button
            type="button"
            className="tabs__tab"
            aria-pressed={tab === "write"}
            onClick={() => {
              setTab("write");
            }}
          >
            Escribir
          </button>
          <button
            type="button"
            className="tabs__tab"
            aria-pressed={tab === "preview"}
            onClick={() => {
              setTab("preview");
            }}
          >
            Vista previa
          </button>
        </div>
        {tab === "write" ? (
          <textarea
            id={`${id}-description`}
            aria-label="Descripción en Markdown"
            className="input--mono"
            rows={12}
            maxLength={MAX_DESCRIPTION}
            value={draft.description}
            aria-invalid={fieldError("description") ? true : undefined}
            aria-describedby={describedBy("description") ?? `${id}-count`}
            onChange={(event) => {
              update({ description: event.target.value });
            }}
          />
        ) : (
          <div className="preview" aria-label="Vista previa de la descripción">
            <MarkdownPreview source={draft.description} />
          </div>
        )}
        <span id={`${id}-count`} className="field__hint">
          {draft.description.length.toLocaleString("es")} /{" "}
          {MAX_DESCRIPTION.toLocaleString("es")} caracteres
        </span>
        <FieldMessage
          id={describedBy("description")}
          message={fieldError("description")?.message}
        />
      </fieldset>

      <fieldset className="field">
        <legend>Criterios de aceptación</legend>
        {draft.criteria.length === 0 && (
          <p className="panel__muted">
            Sin criterios. Añade al menos uno verificable.
          </p>
        )}
        <ol className="criteria">
          {draft.criteria.map((criterion, index) => {
            const label = criterion.number
              ? `CA-${String(criterion.number)}`
              : "Nuevo";
            return (
              <li key={criterion.key} className="criteria__item">
                <label
                  htmlFor={`${id}-criterion-${criterion.key}`}
                  className="criteria__label"
                >
                  {label}
                </label>
                <input
                  id={`${id}-criterion-${criterion.key}`}
                  ref={
                    index === draft.criteria.length - 1
                      ? lastCriterionRef
                      : undefined
                  }
                  value={criterion.text}
                  maxLength={500}
                  onChange={(event) => {
                    update({
                      criteria: draft.criteria.map((c) =>
                        c.key === criterion.key
                          ? { ...c, text: event.target.value }
                          : c,
                      ),
                    });
                  }}
                />
                <button
                  type="button"
                  className="button button--secondary"
                  aria-label={`Quitar ${label}: ${criterion.text}`}
                  onClick={() => {
                    update({
                      criteria: draft.criteria.filter(
                        (c) => c.key !== criterion.key,
                      ),
                    });
                  }}
                >
                  Quitar
                </button>
              </li>
            );
          })}
        </ol>
        <FieldMessage
          id={describedBy("criteria")}
          message={fieldError("criteria")?.message}
        />
        <button
          type="button"
          className="button button--secondary"
          onClick={() => {
            focusNewCriterion.current = true;
            update({
              criteria: [...draft.criteria, { key: newRequestId(), text: "" }],
            });
          }}
        >
          Añadir criterio
        </button>
      </fieldset>

      <div className="repo__actions">
        <button
          type="submit"
          className="button"
          aria-busy={save.pending}
          disabled={save.pending || !dirty || conflict !== null}
        >
          {save.pending
            ? "Guardando…"
            : !dirty && saved
              ? "Guardado ✓"
              : "Guardar"}
        </button>
      </div>
    </form>
  );
}

function FieldMessage({
  id,
  message,
}: {
  id: string | undefined;
  message: string | undefined;
}) {
  if (!id || !message) return null;
  return (
    <p id={id} className="field__error" role="alert">
      {message}
    </p>
  );
}
