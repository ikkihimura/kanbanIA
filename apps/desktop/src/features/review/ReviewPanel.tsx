import { useCallback, useId, useRef, useState } from "react";
import type { Delivery } from "../../contracts/generated/Delivery";
import type { ReferenceInput } from "../../contracts/generated/ReferenceInput";
import type { ReferenceStatus } from "../../contracts/generated/ReferenceStatus";
import type { StoryDetail } from "../../contracts/generated/StoryDetail";
import type { StorySummary } from "../../contracts/generated/StorySummary";
import {
  acceptStory,
  getStoryReview,
  listColumns,
  newRequestId,
  reopenStory,
  returnDelivery,
  setStoryBlocker,
  submitDelivery,
  type UiError,
} from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { formatDateTime } from "../../shared/ui/format";

const REFERENCE_STATUS: Record<ReferenceStatus, string> = {
  accessible: "Accesible",
  missing: "No encontrada",
  inaccessible: "Sin acceso",
  unverified: "URL no verificada",
};

const DELIVERY_STATUS: Record<Delivery["status"], string> = {
  pending: "Pendiente de revisión",
  accepted: "Aceptada",
  returned: "Devuelta",
};

interface Props {
  story: StoryDetail;
  /**
   * Hay cambios de contenido sin guardar: las acciones de revisión cambian la revisión de la
   * historia, así que se bloquean para no provocar un conflicto con el borrador.
   */
  locked: boolean;
  /** La historia cambió (columna, aceptación…): el editor debe recargarla. */
  onStoryChanged: () => void;
}

/** Bloqueo, entregas, aceptación humana, reapertura e historial. */
export function ReviewPanel({ story, locked, onStoryChanged }: Props) {
  const id = useId();
  const load = useCallback(
    () => getStoryReview(story.projectId, story.id),
    [story.projectId, story.id],
  );
  const { state, run } = useIpcResource(load);
  const [message, setMessage] = useState("");
  const archived = story.archivedAt !== null;
  const accepted = story.acceptedAt !== null;

  const changed = useCallback(
    (text: string, storyChanged: boolean) => {
      setMessage(text);
      run(load, { keepData: true });
      if (storyChanged) onStoryChanged();
    },
    [run, load, onStoryChanged],
  );

  if (state.status === "loading") {
    return <p className="panel__muted">Cargando revisión…</p>;
  }
  if (state.status === "error") {
    return <p className="panel__error">{state.message}</p>;
  }
  const review = state.data;
  const pending = review.deliveries.filter((d) => d.status === "pending");

  return (
    <section className="panel review" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="panel__title">
        Revisión
      </h2>
      <p className="review__state">
        {accepted && story.acceptedAt ? (
          <span className="badge badge--ok">
            Aceptada {formatDateTime(story.acceptedAt)}
          </span>
        ) : pending.length > 0 ? (
          <span className="badge badge--warn">
            {pending.length === 1
              ? "1 entrega pendiente de revisión"
              : `${String(pending.length)} entregas pendientes de revisión`}
          </span>
        ) : (
          <span className="badge">Sin aceptar</span>
        )}
      </p>
      {locked && (
        <p className="field__hint">
          Guarda o descarta los cambios del contenido para entregar, aceptar o
          reabrir.
        </p>
      )}
      <p role="status" className="relations__saved">
        {message && `✓ ${message}`}
      </p>

      {!archived && !accepted && (
        <BlockerControl
          story={story}
          blocker={review.blocker}
          onChanged={(text) => {
            changed(text, false);
          }}
        />
      )}

      {!archived && !accepted && (
        <AcceptForm
          // Una revisión nueva (p. ej. criterios editados) exige volver a comprobarlos.
          key={`accept-${String(story.revision)}`}
          story={story}
          locked={locked}
          onAccepted={() => {
            changed(`Historia #${String(story.number)} aceptada.`, true);
          }}
        />
      )}

      {!archived && accepted && (
        <ReopenForm
          story={story}
          locked={locked}
          onReopened={() => {
            changed(`Historia #${String(story.number)} reabierta.`, true);
          }}
        />
      )}

      <h3 className="relations__title">Entregas</h3>
      {!archived && !accepted && (
        <DeliveryForm
          story={story}
          locked={locked}
          onSubmitted={(delivery, movedToReview) => {
            changed(
              `Entrega E${String(delivery.number)} registrada${movedToReview ? "; revisión solicitada" : ""}.`,
              true,
            );
          }}
        />
      )}
      {review.deliveries.length === 0 ? (
        <p className="panel__muted">Sin entregas.</p>
      ) : (
        <ul className="item-list">
          {review.deliveries.map((delivery) => (
            <DeliveryItem
              key={delivery.id}
              projectId={story.projectId}
              delivery={delivery}
              onReturned={() => {
                changed(`Entrega E${String(delivery.number)} devuelta.`, false);
              }}
            />
          ))}
        </ul>
      )}

      <h3 className="relations__title">Historial de aceptación</h3>
      {review.history.length === 0 ? (
        <p className="panel__muted">Nunca se ha aceptado.</p>
      ) : (
        <ol className="relations__list">
          {review.history.map((event) => (
            <li key={`${event.kind}-${event.createdAt}`}>
              <strong>
                {event.kind === "accepted" ? "Aceptada" : "Reabierta"}
              </strong>{" "}
              por una persona · {formatDateTime(event.createdAt)}
              {event.reason && <> · Motivo: {event.reason}</>}
              {event.criteria.length > 0 && (
                <ul className="review__criteria">
                  {event.criteria.map((c) => (
                    <li key={c.number}>
                      ✓ CA-{c.number}: {c.text}
                    </li>
                  ))}
                </ul>
              )}
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function BlockerControl({
  story,
  blocker,
  onChanged,
}: {
  story: StoryDetail;
  blocker: { reason: string; source: string; agentName: string | null } | null;
  onChanged: (message: string) => void;
}) {
  const inputId = useId();
  const [reason, setReason] = useState("");
  const save = useMutation(setStoryBlocker);

  async function apply(next: string | null) {
    const result = await save.mutate({
      projectId: story.projectId,
      storyId: story.id,
      reason: next,
    });
    if (result.ok) {
      setReason("");
      onChanged(next ? "Historia marcada como bloqueada." : "Bloqueo quitado.");
    }
  }

  if (blocker) {
    return (
      <div className="review__blocker">
        <p className="badge badge--warn">
          Bloqueada: {blocker.reason}
          {blocker.source === "reported" && blocker.agentName
            ? ` (reportado por ${blocker.agentName})`
            : ""}
        </p>
        <button
          type="button"
          className="button button--secondary"
          aria-busy={save.pending}
          disabled={save.pending}
          onClick={() => {
            void apply(null);
          }}
        >
          Quitar bloqueo
        </button>
        <ErrorLine error={save.error} />
      </div>
    );
  }
  return (
    <form
      className="form-row"
      aria-label="Marcar como bloqueada"
      onSubmit={(event) => {
        event.preventDefault();
        void apply(reason);
      }}
    >
      <div className="field field--grow">
        <label htmlFor={inputId}>Motivo del bloqueo</label>
        <input
          id={inputId}
          value={reason}
          maxLength={500}
          placeholder="p. ej. Esperando acceso a la API"
          aria-invalid={save.error ? true : undefined}
          aria-describedby={save.error ? `${inputId}-error` : undefined}
          onChange={(event) => {
            setReason(event.target.value);
            save.reset();
          }}
        />
        <ErrorLine id={`${inputId}-error`} error={save.error} />
      </div>
      <button
        type="submit"
        className="button button--secondary"
        aria-busy={save.pending}
        disabled={save.pending || reason.trim() === ""}
      >
        Marcar bloqueada
      </button>
    </form>
  );
}

/** Lista de criterios a comprobar y botón «Aceptar historia» (también la usa el tablero). */
export function AcceptForm({
  story,
  locked,
  target,
  onAccepted,
}: {
  story: StoryDetail;
  locked: boolean;
  /** Columna «hecho» y posición destino (tablero); sin ella, al final de la primera. */
  target?: { columnId: string; toIndex: number };
  onAccepted: (accepted: StorySummary) => void;
}) {
  const hintId = useId();
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const accept = useMutation(acceptStory);
  const allChecked =
    story.criteria.length > 0 && story.criteria.every((c) => checked.has(c.id));

  async function handleAccept() {
    const result = await accept.mutate({
      projectId: story.projectId,
      storyId: story.id,
      expectedRevision: story.revision,
      // Solo los criterios vigentes: nunca ids de criterios ya borrados.
      satisfiedCriteria: story.criteria
        .filter((c) => checked.has(c.id))
        .map((c) => c.id),
      ...(target ?? {}),
    });
    if (result.ok) onAccepted(result.value);
  }

  return (
    <form
      className="review__accept"
      aria-label="Aceptar historia"
      onSubmit={(event) => {
        event.preventDefault();
        void handleAccept();
      }}
    >
      <fieldset className="field">
        <legend>Aceptación: marca cada criterio que comprobaste</legend>
        {story.criteria.length === 0 ? (
          <p className="panel__muted">
            Añade al menos un criterio de aceptación verificable antes de
            aceptar.
          </p>
        ) : (
          story.criteria.map((c) => (
            <label key={c.id} className="checkbox">
              <input
                type="checkbox"
                checked={checked.has(c.id)}
                onChange={(event) => {
                  const next = new Set(checked);
                  if (event.target.checked) next.add(c.id);
                  else next.delete(c.id);
                  setChecked(next);
                  accept.reset();
                }}
              />
              Cumple CA-{c.number}: {c.text}
            </label>
          ))
        )}
      </fieldset>
      <ErrorLine error={accept.error} />
      <button
        type="submit"
        className="button"
        aria-busy={accept.pending}
        aria-describedby={allChecked ? undefined : `${hintId}-pending`}
        disabled={locked || !allChecked || accept.pending}
      >
        Aceptar historia
      </button>
      {!allChecked && story.criteria.length > 0 && (
        <p id={`${hintId}-pending`} className="field__hint">
          Marca todos los criterios para poder aceptar.
        </p>
      )}
      <p className="field__hint">
        Aceptar la mueve a «Hecho». Solo una persona puede aceptar; los agentes
        no.
      </p>
    </form>
  );
}

function ReopenForm({
  story,
  locked,
  onReopened,
}: {
  story: StoryDetail;
  locked: boolean;
  onReopened: () => void;
}) {
  const id = useId();
  const loadColumns = useCallback(
    () => listColumns(story.projectId),
    [story.projectId],
  );
  const columns = useIpcResource(loadColumns);
  const [reason, setReason] = useState("");
  const [columnId, setColumnId] = useState("");
  const reopen = useMutation(reopenStory);
  const options =
    columns.state.status === "ready"
      ? columns.state.data.filter((c) => c.category !== "done")
      : [];

  async function handleReopen() {
    const result = await reopen.mutate({
      projectId: story.projectId,
      storyId: story.id,
      expectedRevision: story.revision,
      reason,
      ...(columnId ? { columnId } : {}),
    });
    if (result.ok) onReopened();
  }

  return (
    <form
      className="form-row"
      aria-label="Reabrir historia"
      onSubmit={(event) => {
        event.preventDefault();
        void handleReopen();
      }}
    >
      <div className="field field--grow">
        <label htmlFor={`${id}-reason`}>Motivo de la reapertura</label>
        <input
          id={`${id}-reason`}
          value={reason}
          maxLength={500}
          aria-invalid={reopen.error ? true : undefined}
          aria-describedby={reopen.error ? `${id}-error` : undefined}
          onChange={(event) => {
            setReason(event.target.value);
            reopen.reset();
          }}
        />
      </div>
      <div className="field">
        <label htmlFor={`${id}-column`}>Volver a</label>
        <select
          id={`${id}-column`}
          value={columnId}
          onChange={(event) => {
            setColumnId(event.target.value);
          }}
        >
          <option value="">
            Por defecto (revisión o la primera disponible)
          </option>
          {options.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </select>
      </div>
      <button
        type="submit"
        className="button button--secondary"
        aria-busy={reopen.pending}
        disabled={locked || reopen.pending || reason.trim() === ""}
      >
        Reabrir
      </button>
      <ErrorLine id={`${id}-error`} error={reopen.error} />
    </form>
  );
}

function DeliveryForm({
  story,
  locked,
  onSubmitted,
}: {
  story: StoryDetail;
  locked: boolean;
  onSubmitted: (delivery: Delivery, movedToReview: boolean) => void;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [summary, setSummary] = useState("");
  const [references, setReferences] = useState<ReferenceInput[]>([]);
  const [toReview, setToReview] = useState(true);
  const requestId = useRef(newRequestId());
  const submit = useMutation(submitDelivery);
  const touch = () => {
    requestId.current = newRequestId();
    submit.reset();
  };

  async function handleSubmit() {
    const result = await submit.mutate({
      requestId: requestId.current,
      projectId: story.projectId,
      storyId: story.id,
      summary,
      references,
      sendToReview: toReview,
    });
    if (result.ok) {
      requestId.current = newRequestId();
      setSummary("");
      setReferences([]);
      setOpen(false);
      onSubmitted(result.value, toReview);
    }
  }

  if (!open) {
    return (
      <button
        type="button"
        className="button button--secondary"
        disabled={locked}
        onClick={() => {
          setOpen(true);
        }}
      >
        Registrar entrega
      </button>
    );
  }

  return (
    <form
      className="team-form"
      aria-label="Registrar entrega"
      onSubmit={(event) => {
        event.preventDefault();
        void handleSubmit();
      }}
    >
      <div className="field">
        <label htmlFor={`${id}-summary`}>Resumen de lo entregado</label>
        <textarea
          id={`${id}-summary`}
          rows={3}
          maxLength={2000}
          value={summary}
          autoFocus
          aria-invalid={submit.error ? true : undefined}
          aria-describedby={submit.error ? `${id}-error` : undefined}
          onChange={(event) => {
            setSummary(event.target.value);
            touch();
          }}
        />
      </div>
      <fieldset className="field">
        <legend>Referencias (PR, commit, archivo…)</legend>
        {references.map((reference, index) => (
          <div key={index} className="field__inline">
            <select
              aria-label={`Tipo de la referencia ${String(index + 1)}`}
              value={reference.kind}
              onChange={(event) => {
                const kind = event.target.value as ReferenceInput["kind"];
                setReferences(
                  references.map((r, i) => (i === index ? { ...r, kind } : r)),
                );
                touch();
              }}
            >
              <option value="url">URL</option>
              <option value="path">Ruta local</option>
            </select>
            <input
              aria-label={`Referencia ${String(index + 1)}`}
              className="input--mono"
              value={reference.value}
              placeholder={
                reference.kind === "url"
                  ? "https://…"
                  : "/ruta/absoluta/al/archivo"
              }
              onChange={(event) => {
                const value = event.target.value;
                setReferences(
                  references.map((r, i) => (i === index ? { ...r, value } : r)),
                );
                touch();
              }}
            />
            <button
              type="button"
              className="button button--secondary"
              aria-label={`Quitar referencia ${String(index + 1)}`}
              onClick={() => {
                setReferences(references.filter((_, i) => i !== index));
                touch();
              }}
            >
              Quitar
            </button>
          </div>
        ))}
        {references.length < 10 && (
          <button
            type="button"
            className="button button--secondary"
            onClick={() => {
              setReferences([...references, { kind: "url", value: "" }]);
              touch();
            }}
          >
            Añadir referencia
          </button>
        )}
      </fieldset>
      <label className="checkbox">
        <input
          type="checkbox"
          checked={toReview}
          onChange={(event) => {
            setToReview(event.target.checked);
            touch();
          }}
        />
        Enviar a revisión (no la marca como hecha)
      </label>
      <ErrorLine id={`${id}-error`} error={submit.error} />
      <div className="repo__actions">
        <button
          type="submit"
          className="button"
          aria-busy={submit.pending}
          disabled={locked || submit.pending}
        >
          Registrar
        </button>
        <button
          type="button"
          className="button button--secondary"
          onClick={() => {
            setOpen(false);
          }}
        >
          Cancelar
        </button>
      </div>
    </form>
  );
}

function DeliveryItem({
  projectId,
  delivery,
  onReturned,
}: {
  projectId: string;
  delivery: Delivery;
  onReturned: () => void;
}) {
  const id = useId();
  const [returning, setReturning] = useState(false);
  const [reason, setReason] = useState("");
  const ret = useMutation(returnDelivery);

  async function handleReturn() {
    const result = await ret.mutate({
      projectId,
      deliveryId: delivery.id,
      reason,
    });
    if (result.ok) onReturned();
  }

  return (
    <li className="column-item">
      <div className="column-item__summary">
        <span className="item-list__name">E{delivery.number}</span>
        <span className="item-list__meta">
          {delivery.source === "manual"
            ? "Manual"
            : `Reportada por ${delivery.agentName ?? "un agente"}`}
          {" · "}
          {formatDateTime(delivery.createdAt)}
        </span>
        <span
          className={`badge ${delivery.status === "returned" ? "badge--warn" : delivery.status === "accepted" ? "badge--ok" : ""}`}
        >
          {DELIVERY_STATUS[delivery.status]}
        </span>
      </div>
      <p className="review__summary">{delivery.summary}</p>
      {delivery.returnReason && (
        <p className="panel__muted">Motivo: {delivery.returnReason}</p>
      )}
      {delivery.references.length > 0 && (
        <ul className="relations__list">
          {delivery.references.map((reference, index) => (
            <li key={index} className="relations__item">
              <code className="repo__path">{reference.value}</code>
              <span
                className={`badge ${reference.status === "accessible" ? "badge--ok" : "badge--warn"}`}
              >
                {REFERENCE_STATUS[reference.status]}
              </span>
            </li>
          ))}
        </ul>
      )}
      {delivery.status === "pending" &&
        (returning ? (
          <form
            className="form-row"
            aria-label={`Devolver entrega E${String(delivery.number)}`}
            onSubmit={(event) => {
              event.preventDefault();
              void handleReturn();
            }}
          >
            <div className="field field--grow">
              <label htmlFor={`${id}-reason`}>Motivo de la devolución</label>
              <input
                id={`${id}-reason`}
                value={reason}
                maxLength={500}
                autoFocus
                aria-invalid={ret.error ? true : undefined}
                aria-describedby={ret.error ? `${id}-error` : undefined}
                onChange={(event) => {
                  setReason(event.target.value);
                  ret.reset();
                }}
              />
            </div>
            <button
              type="submit"
              className="button button--secondary"
              aria-busy={ret.pending}
              disabled={ret.pending || reason.trim() === ""}
            >
              Devolver
            </button>
            <button
              type="button"
              className="button button--secondary"
              onClick={() => {
                setReturning(false);
              }}
            >
              Cancelar
            </button>
            <ErrorLine id={`${id}-error`} error={ret.error} />
          </form>
        ) : (
          <div className="repo__actions">
            <button
              type="button"
              className="button button--secondary"
              onClick={() => {
                setReturning(true);
              }}
            >
              Devolver…
            </button>
          </div>
        ))}
    </li>
  );
}

function ErrorLine({ error, id }: { error: UiError | null; id?: string }) {
  if (!error) return null;
  return (
    <p id={id} className="field__error" role="alert">
      {error.message}
    </p>
  );
}
