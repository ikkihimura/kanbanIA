import { useId } from "react";
import type { ColumnCategory } from "../../contracts/generated/ColumnCategory";
import type { UiError } from "../../shared/ipc/commands";
import { CATEGORIES, CATEGORY_LABEL } from "./categories";

const FIELDS = ["name", "category", "wipLimit"];

export interface ColumnDraft {
  name: string;
  category: ColumnCategory;
  wip: string;
}

interface Props {
  draft: ColumnDraft;
  error: UiError | null;
  onChange: (draft: ColumnDraft) => void;
  autoFocus?: boolean;
}

/** Nombre, categoría y límite WIP. El error se muestra junto al campo que lo causó. */
export function ColumnFields({
  draft,
  error,
  onChange,
  autoFocus = false,
}: Props) {
  const id = useId();
  const errorFor = (field: string) => (error?.field === field ? error : null);
  // Errores sin campo propio (conflicto, WIP, aceptación…) se muestran aparte.
  const general = error && !FIELDS.includes(error.field ?? "") ? error : null;
  const describe = (field: string) =>
    errorFor(field) ? `${id}-${field}-error` : undefined;

  return (
    <>
      {general && (
        <p className="field__error form-row__full" role="alert">
          {general.message}
        </p>
      )}
      <div className="field">
        <label htmlFor={`${id}-name`}>Nombre</label>
        <input
          id={`${id}-name`}
          value={draft.name}
          maxLength={60}
          autoComplete="off"
          autoFocus={autoFocus}
          aria-invalid={errorFor("name") ? true : undefined}
          aria-describedby={describe("name")}
          onChange={(event) => {
            onChange({ ...draft, name: event.target.value });
          }}
        />
        <FieldMessage id={describe("name")} error={errorFor("name")} />
      </div>

      <div className="field">
        <label htmlFor={`${id}-category`}>Tipo de etapa</label>
        <select
          id={`${id}-category`}
          value={draft.category}
          aria-invalid={errorFor("category") ? true : undefined}
          aria-describedby={describe("category")}
          onChange={(event) => {
            onChange({
              ...draft,
              category: event.target.value as ColumnCategory,
            });
          }}
        >
          {CATEGORIES.map((category) => (
            <option key={category} value={category}>
              {CATEGORY_LABEL[category]}
            </option>
          ))}
        </select>
        <FieldMessage id={describe("category")} error={errorFor("category")} />
      </div>

      <div className="field field--narrow">
        <label htmlFor={`${id}-wip`}>Límite WIP</label>
        <input
          id={`${id}-wip`}
          inputMode="numeric"
          placeholder="sin límite"
          value={draft.wip}
          aria-invalid={errorFor("wipLimit") ? true : undefined}
          aria-describedby={describe("wipLimit") ?? `${id}-wip-hint`}
          onChange={(event) => {
            onChange({ ...draft, wip: event.target.value });
          }}
        />
        {errorFor("wipLimit") ? (
          <FieldMessage
            id={describe("wipLimit")}
            error={errorFor("wipLimit")}
          />
        ) : (
          <span id={`${id}-wip-hint`} className="field__hint">
            1–999, vacío = sin límite
          </span>
        )}
      </div>
    </>
  );
}

function FieldMessage({
  id,
  error,
}: {
  id: string | undefined;
  error: UiError | null;
}) {
  if (!error || !id) return null;
  return (
    <p id={id} className="field__error" role="alert">
      {error.message}
    </p>
  );
}
