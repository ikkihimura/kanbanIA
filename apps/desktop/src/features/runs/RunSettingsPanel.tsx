import { useId, useState } from "react";
import type { RunSettings } from "../../contracts/generated/RunSettings";
import {
  getRunSettings,
  updateRunSettings,
  type UiError,
} from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { FieldError } from "../../shared/ui/FieldError";
import { useUnsavedChangesGuard } from "../../shared/ui/useUnsavedChangesGuard";

/** Iguales que los límites del núcleo (`run_settings`); el núcleo vuelve a validarlos. */
export const CONCURRENCY_LIMITS = { min: 1, max: 8 } as const;
export const TIMEOUT_LIMITS = { min: 1, max: 240 } as const;

type Field = "maxConcurrency" | "timeoutMinutes";

function parseInRange(
  text: string,
  limits: { min: number; max: number },
): number | null {
  if (!/^\d+$/.test(text.trim())) return null;
  const value = Number(text.trim());
  return value >= limits.min && value <= limits.max ? value : null;
}

function isRunSettings(value: unknown): value is RunSettings {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.maxConcurrency === "number" &&
    typeof v.timeoutMinutes === "number" &&
    typeof v.revision === "number"
  );
}

/** Sección «Ejecuciones» de «Sistema» (HU-018): concurrencia y tiempo máximo del perfil. */
export function RunSettingsPanel() {
  const id = useId();
  const { state, run } = useIpcResource(getRunSettings);

  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="panel__title">
        Ejecuciones
      </h2>
      <p className="field__hint">
        Límites del proceso de prueba (sin IA) para todo el perfil de datos.
      </p>
      {state.status === "loading" && (
        <p className="panel__muted" role="status">
          Cargando configuración…
        </p>
      )}
      {state.status === "error" && (
        <div>
          <p className="panel__error" role="alert">
            No se pudo cargar la configuración: {state.message}
          </p>
          <button
            type="button"
            className="button"
            onClick={() => {
              run(getRunSettings);
            }}
          >
            Reintentar
          </button>
        </div>
      )}
      {state.status === "ready" && <SettingsForm initial={state.data} />}
    </section>
  );
}

function SettingsForm({ initial }: { initial: RunSettings }) {
  const id = useId();
  // Configuración que el formulario conoce (revisión para `expectedRevision`).
  const [base, setBase] = useState(initial);
  const [concurrency, setConcurrency] = useState(
    String(initial.maxConcurrency),
  );
  const [timeout, setTimeoutText] = useState(String(initial.timeoutMinutes));
  const [localErrors, setLocalErrors] = useState<
    Partial<Record<Field, UiError>>
  >({});
  const [message, setMessage] = useState("");
  const [conflict, setConflict] = useState<RunSettings | null>(null);
  const save = useMutation(updateRunSettings);
  // Valores escritos sin guardar: cerrar la ventana pide confirmación.
  const dirty =
    concurrency !== String(base.maxConcurrency) ||
    timeout !== String(base.timeoutMinutes);
  useUnsavedChangesGuard(dirty);

  const touch = () => {
    save.reset();
    setMessage("");
    setLocalErrors({});
  };

  async function handleSubmit() {
    const maxConcurrency = parseInRange(concurrency, CONCURRENCY_LIMITS);
    const timeoutMinutes = parseInRange(timeout, TIMEOUT_LIMITS);
    const errors: Partial<Record<Field, UiError>> = {};
    if (maxConcurrency === null)
      errors.maxConcurrency = {
        code: "validation",
        field: "maxConcurrency",
        message: `Escribe un número entero entre ${String(CONCURRENCY_LIMITS.min)} y ${String(CONCURRENCY_LIMITS.max)}.`,
      };
    if (timeoutMinutes === null)
      errors.timeoutMinutes = {
        code: "validation",
        field: "timeoutMinutes",
        message: `Escribe un número entero de minutos entre ${String(TIMEOUT_LIMITS.min)} y ${String(TIMEOUT_LIMITS.max)}.`,
      };
    setLocalErrors(errors);
    setMessage("");
    if (maxConcurrency === null || timeoutMinutes === null) return;

    const result = await save.mutate({
      expectedRevision: base.revision,
      maxConcurrency,
      timeoutMinutes,
    });
    if (result.ok) {
      setBase(result.value);
      setConflict(null);
      setConcurrency(String(result.value.maxConcurrency));
      setTimeoutText(String(result.value.timeoutMinutes));
      setMessage("Configuración guardada.");
    } else if (
      result.error.code === "conflict" &&
      isRunSettings(result.error.current)
    ) {
      // Se adopta la revisión vigente y se conservan los valores escritos: guardar de nuevo
      // los aplicará sobre la configuración actual (que se muestra para decidir).
      setBase(result.error.current);
      setConflict(result.error.current);
    }
  }

  const serverError = save.error;
  const fieldError = (field: Field): UiError | null =>
    localErrors[field] ?? (serverError?.field === field ? serverError : null);
  const concurrencyError = fieldError("maxConcurrency");
  const timeoutError = fieldError("timeoutMinutes");
  const generalError =
    serverError &&
    serverError.code !== "conflict" &&
    serverError.field !== "maxConcurrency" &&
    serverError.field !== "timeoutMinutes"
      ? serverError
      : null;

  const describe = (hint: string, error: UiError | null, errorId: string) =>
    [hint, error ? errorId : null].filter(Boolean).join(" ");

  return (
    <form
      className="team-form"
      aria-label="Configuración de ejecuciones"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        void handleSubmit();
      }}
    >
      <p role="status" className="relations__saved">
        {message && `✓ ${message}`}
      </p>

      <div className="field">
        <label htmlFor={`${id}-concurrency`}>Ejecuciones a la vez</label>
        <input
          id={`${id}-concurrency`}
          type="number"
          inputMode="numeric"
          min={CONCURRENCY_LIMITS.min}
          max={CONCURRENCY_LIMITS.max}
          step={1}
          value={concurrency}
          aria-invalid={concurrencyError ? true : undefined}
          aria-describedby={describe(
            `${id}-concurrency-hint`,
            concurrencyError,
            `${id}-concurrency-error`,
          )}
          onChange={(event) => {
            setConcurrency(event.target.value);
            touch();
          }}
        />
        <span id={`${id}-concurrency-hint`} className="field__hint">
          Entre {CONCURRENCY_LIMITS.min} y {CONCURRENCY_LIMITS.max}. Los demás
          esperan en cola. Bajar el límite no detiene los que ya corren.
        </span>
        <FieldError id={`${id}-concurrency-error`} error={concurrencyError} />
      </div>

      <div className="field">
        <label htmlFor={`${id}-timeout`}>
          Tiempo máximo por ejecución (minutos)
        </label>
        <input
          id={`${id}-timeout`}
          type="number"
          inputMode="numeric"
          min={TIMEOUT_LIMITS.min}
          max={TIMEOUT_LIMITS.max}
          step={1}
          value={timeout}
          aria-invalid={timeoutError ? true : undefined}
          aria-describedby={describe(
            `${id}-timeout-hint`,
            timeoutError,
            `${id}-timeout-error`,
          )}
          onChange={(event) => {
            setTimeoutText(event.target.value);
            touch();
          }}
        />
        <span id={`${id}-timeout-hint`} className="field__hint">
          Entre {TIMEOUT_LIMITS.min} y {TIMEOUT_LIMITS.max}. Al superarlo se
          detienen el proceso y sus hijos, y el intento queda «Fallido». Cada
          ejecución usa el valor vigente al salir de la cola.
        </span>
        <FieldError id={`${id}-timeout-error`} error={timeoutError} />
      </div>

      {conflict && (
        <div className="run-form__alert" role="alert">
          <p>
            La configuración cambió mientras la editabas. Ahora es:{" "}
            {conflict.maxConcurrency} a la vez y {conflict.timeoutMinutes} min.
            Revisa tus valores y guarda de nuevo para aplicarlos.
          </p>
        </div>
      )}
      {generalError && (
        <p className="panel__error" role="alert">
          No se pudo guardar: {generalError.message}
        </p>
      )}

      <div>
        <button
          type="submit"
          className="button"
          aria-busy={save.pending}
          disabled={save.pending}
        >
          Guardar
        </button>
      </div>
    </form>
  );
}
