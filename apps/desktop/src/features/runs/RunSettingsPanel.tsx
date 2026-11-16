import { useId, useState, type ReactNode } from "react";
import type { AcpAdapterCheck } from "../../contracts/generated/AcpAdapterCheck";
import type { AcpAgent } from "../../contracts/generated/AcpAgent";
import type { RunSettings } from "../../contracts/generated/RunSettings";
import {
  checkAcpAdapter,
  getCodexHomeStatus,
  getRunSettings,
  updateRunSettings,
  type UiError,
} from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { FieldError } from "../../shared/ui/FieldError";
import { useUnsavedChangesGuard } from "../../shared/ui/useUnsavedChangesGuard";
import {
  ACP_AGENT_NAME,
  describeAgentIdentity,
  describeCapabilities,
} from "./runFormat";

/** Iguales que los límites del núcleo (`run_settings`); el núcleo vuelve a validarlos. */
export const CONCURRENCY_LIMITS = { min: 1, max: 8 } as const;
export const TIMEOUT_LIMITS = { min: 1, max: 240 } as const;

type Field =
  "maxConcurrency" | "timeoutMinutes" | "claudeAcpPath" | "codexAcpPath";

/** Comprobación por agente; función de módulo para que `useMutation` la reciba estable. */
function checkAdapter({
  agent,
  path,
}: {
  agent: AcpAgent;
  path: string;
}): Promise<AcpAdapterCheck> {
  return checkAcpAdapter(agent, path);
}

/**
 * Comando para iniciar sesión en la carpeta de Codex de la app desde una shell POSIX (zsh,
 * bash). Comillas simples: nada se interpreta dentro; una `'` se escribe `'\''`.
 */
export function codexLoginCommand(path: string): string {
  return `CODEX_HOME='${path.replaceAll("'", "'\\''")}' codex login`;
}

/**
 * El mismo comando para PowerShell (Windows). En comillas simples nada se interpreta y una
 * `'` se escribe `''`.
 */
export function codexLoginCommandPowerShell(path: string): string {
  return `$env:CODEX_HOME='${path.replaceAll("'", "''")}'; codex login`;
}

/** Resultado de «Comprobar adaptador» en una línea (lo que declaró el adaptador). */
export function describeAdapterCheck(check: AcpAdapterCheck): string {
  return `Responde: ${describeAgentIdentity(check)} · ACP v${String(check.protocolVersion)} · capacidades: ${describeCapabilities(check.capabilities)}`;
}

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
    (typeof v.claudeAcpPath === "string" || v.claudeAcpPath === null) &&
    (typeof v.codexAcpPath === "string" || v.codexAcpPath === null) &&
    typeof v.approvalNotifications === "boolean" &&
    typeof v.revision === "number"
  );
}

/**
 * Sección «Ejecuciones» de «Sistema» (HU-018/019/021): concurrencia, tiempo máximo,
 * adaptadores ACP de Claude Code y Codex, y la carpeta de Codex propia de la app.
 */
export function RunSettingsPanel() {
  const id = useId();
  const { state, run } = useIpcResource(getRunSettings);

  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="panel__title">
        Ejecuciones
      </h2>
      <p className="field__hint">
        Límites de las ejecuciones y adaptadores de Claude Code y Codex para
        todo el perfil de datos.
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
  const [acpPath, setAcpPath] = useState(initial.claudeAcpPath ?? "");
  const [codexPath, setCodexPath] = useState(initial.codexAcpPath ?? "");
  const [notify, setNotify] = useState(initial.approvalNotifications);
  const [localErrors, setLocalErrors] = useState<
    Partial<Record<Field, UiError>>
  >({});
  const [message, setMessage] = useState("");
  const [conflict, setConflict] = useState<RunSettings | null>(null);
  const save = useMutation(updateRunSettings);
  // Valores escritos sin guardar: cerrar la ventana pide confirmación.
  const dirty =
    concurrency !== String(base.maxConcurrency) ||
    timeout !== String(base.timeoutMinutes) ||
    acpPath !== (base.claudeAcpPath ?? "") ||
    codexPath !== (base.codexAcpPath ?? "") ||
    notify !== base.approvalNotifications;
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

    const trimmedPath = acpPath.trim();
    const trimmedCodex = codexPath.trim();
    const result = await save.mutate({
      expectedRevision: base.revision,
      maxConcurrency,
      timeoutMinutes,
      // Siempre se envía: si faltara, el núcleo lo borraría. Vacío = no configurado.
      claudeAcpPath: trimmedPath === "" ? null : trimmedPath,
      codexAcpPath: trimmedCodex === "" ? null : trimmedCodex,
      // También obligatorio: sin él, el núcleo rechaza la petición.
      approvalNotifications: notify,
    });
    if (result.ok) {
      setBase(result.value);
      setConflict(null);
      setConcurrency(String(result.value.maxConcurrency));
      setTimeoutText(String(result.value.timeoutMinutes));
      setAcpPath(result.value.claudeAcpPath ?? "");
      setCodexPath(result.value.codexAcpPath ?? "");
      setNotify(result.value.approvalNotifications);
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
    serverError.field !== "timeoutMinutes" &&
    serverError.field !== "claudeAcpPath" &&
    serverError.field !== "codexAcpPath"
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

      <AdapterField
        agent="claudeCode"
        executable="claude-agent-acp"
        value={acpPath}
        error={fieldError("claudeAcpPath")}
        onChange={(value) => {
          setAcpPath(value);
          touch();
        }}
      >
        <p>
          Ruta absoluta del ejecutable <code>claude-agent-acp</code>, el
          adaptador que conecta Claude Code con la app mediante ACP (Agent
          Client Protocol). Vacío = no configurado: no se podrá elegir Claude
          Code al iniciar. La app no lo incluye ni lo instala.
        </p>
        <p>
          Si lo instalaste con npm necesita <code>node</code> en el PATH, y si
          abres la app desde Finder puede no encontrarlo: usa el binario
          autónomo de la release del adaptador o abre la app desde la terminal.
        </p>
        <p>
          Usa la sesión ya iniciada de Claude Code: puede consumir tu
          suscripción o tu saldo. La app fija el modo «Always ask» (pedir
          permiso): cada petición de permiso del agente aparece en el detalle
          del intento para que la permitas una vez o la rechaces; sin respuesta
          en 10 minutos, se rechaza. Para que nada se permita sin preguntar, la
          app pide al adaptador que no cargue tu configuración de Claude Code ni
          la del repositorio (reglas «allow», hooks, servidores MCP); tu sesión
          iniciada sí se usa.
        </p>
      </AdapterField>

      <AdapterField
        agent="codex"
        executable="codex-acp"
        value={codexPath}
        error={fieldError("codexAcpPath")}
        onChange={(value) => {
          setCodexPath(value);
          touch();
        }}
      >
        <p>
          Ruta absoluta del ejecutable <code>codex-acp</code>, el adaptador que
          conecta Codex (OpenAI) con la app mediante ACP. Vacío = no
          configurado: no se podrá elegir Codex al iniciar. La app no lo incluye
          ni lo instala.
        </p>
        <p>
          Puede consumir tu suscripción o tu saldo de OpenAI. Cada petición de
          permiso para editar o ejecutar comandos con efectos aparece en el
          detalle del intento para que la permitas una vez o la rechaces.
          Comprobado con Codex real: lee archivos sin preguntar, también fuera
          del worktree, y rechazar una escritura detiene su turno.
        </p>
      </AdapterField>

      <CodexHomeSection />

      <div className="field">
        <label className="checkbox run-settings__checkbox">
          <input
            type="checkbox"
            checked={notify}
            aria-describedby={`${id}-notify-hint`}
            onChange={(event) => {
              setNotify(event.target.checked);
              touch();
            }}
          />
          Avisarme con una notificación del sistema cuando un agente pida
          permiso
        </label>
        <span id={`${id}-notify-hint`} className="field__hint">
          El aviso solo dice el número de la historia, nunca el comando ni la
          ruta. Puede requerir permitir notificaciones de Kanban IA en el
          sistema.
        </span>
      </div>

      {conflict && (
        <div className="run-form__alert" role="alert">
          <p>
            La configuración cambió mientras la editabas. Ahora es:{" "}
            {conflict.maxConcurrency} a la vez, {conflict.timeoutMinutes} min,{" "}
            {conflict.claudeAcpPath === null
              ? "sin adaptador de Claude Code"
              : `adaptador de Claude Code ${conflict.claudeAcpPath}`}
            ,{" "}
            {conflict.codexAcpPath === null
              ? "sin adaptador de Codex"
              : `adaptador de Codex ${conflict.codexAcpPath}`}{" "}
            y{" "}
            {conflict.approvalNotifications
              ? "con notificaciones de permisos"
              : "sin notificaciones de permisos"}
            . Revisa tus valores y guarda de nuevo para aplicarlos.
          </p>
        </div>
      )}
      {generalError && (
        <p className="panel__error" role="alert">
          No se pudo guardar: {generalError.message}
        </p>
      )}

      {/* El aviso va junto al botón: el formulario es largo y arriba no se veía. */}
      <div className="run-start__actions">
        <button
          type="submit"
          className="button"
          aria-busy={save.pending}
          disabled={save.pending}
        >
          Guardar
        </button>
        <p role="status" className="relations__saved">
          {message && `✓ ${message}`}
        </p>
      </div>
    </form>
  );
}

interface AdapterFieldProps {
  agent: AcpAgent;
  /** Nombre del ejecutable, para el ejemplo de ruta. */
  executable: string;
  value: string;
  error: UiError | null;
  onChange: (value: string) => void;
  /** Ayuda del campo (forma parte de su descripción accesible). */
  children: ReactNode;
}

/**
 * Ruta del adaptador ACP de un agente con su «Comprobar adaptador». Cada agente tiene su
 * propia comprobación, resultado y error: comprobar uno no toca el otro.
 */
function AdapterField({
  agent,
  executable,
  value,
  error,
  onChange,
  children,
}: AdapterFieldProps) {
  const id = useId();
  const name = ACP_AGENT_NAME[agent];
  const check = useMutation(checkAdapter);
  const [checked, setChecked] = useState<AcpAdapterCheck | null>(null);
  // Se comprueba siempre la ruta escrita (sin guardarla): con el campo vacío no hay nada que
  // comprobar y el botón se deshabilita con la explicación al lado.
  const pathToCheck = value.trim();

  async function handleCheck() {
    if (pathToCheck === "") return;
    setChecked(null);
    const result = await check.mutate({ agent, path: pathToCheck });
    if (result.ok) setChecked(result.value);
  }

  const describedBy = [`${id}-hint`, error ? `${id}-error` : null]
    .filter(Boolean)
    .join(" ");

  return (
    <div className="field">
      <label htmlFor={`${id}-path`}>Adaptador de {name} (ACP)</label>
      <input
        id={`${id}-path`}
        type="text"
        className="input--mono"
        spellCheck={false}
        autoComplete="off"
        autoCapitalize="off"
        placeholder={`/ruta/absoluta/a/${executable}`}
        value={value}
        aria-invalid={error ? true : undefined}
        aria-describedby={describedBy}
        onChange={(event) => {
          onChange(event.target.value);
          check.reset();
          setChecked(null);
        }}
      />
      <div id={`${id}-hint`} className="field__hint run-settings__help">
        {children}
      </div>
      <FieldError id={`${id}-error`} error={error} />
      <div className="run-start__actions">
        <button
          type="button"
          className="button button--secondary"
          aria-busy={check.pending}
          disabled={check.pending || pathToCheck === ""}
          aria-describedby={`${id}-check-result`}
          onClick={() => {
            void handleCheck();
          }}
        >
          Comprobar adaptador <span className="visually-hidden">de {name}</span>
        </button>
      </div>
      <div id={`${id}-check-result`} role="status">
        {pathToCheck === "" && (
          <p className="field__hint">Escribe la ruta para comprobarla.</p>
        )}
        {check.pending && (
          <p className="panel__muted">
            Comprobando el adaptador (puede tardar unos segundos)…
          </p>
        )}
        {checked && (
          <p className="relations__saved">✓ {describeAdapterCheck(checked)}</p>
        )}
        {check.error && (
          <p className="field__error">
            No se pudo comprobar el adaptador: {check.error.message}
          </p>
        )}
      </div>
    </div>
  );
}

/**
 * Carpeta de Codex propia de la app (HU-021): la app lanza Codex con `CODEX_HOME` apuntando
 * aquí, no a `~/.codex`. Se muestra la ruta, si hay sesión iniciada (solo que exista el
 * archivo) y el comando para iniciarla, como texto seleccionable (sin portapapeles).
 */
function CodexHomeSection() {
  const id = useId();
  const { state, run } = useIpcResource(getCodexHomeStatus);
  const recheck = () => {
    run(getCodexHomeStatus, { keepData: true });
  };
  return (
    <section className="field" aria-labelledby={`${id}-title`}>
      <h3 id={`${id}-title`} className="run-start__label">
        Carpeta de Codex de la app
      </h3>
      <p className="field__hint">
        La app lanza Codex con su propia carpeta de Codex (
        <code>CODEX_HOME</code>), no con tu <code>~/.codex</code>: no carga tu
        configuración, tus servidores MCP, tus reglas ni tu historial de Codex.
        Por eso hay que iniciar sesión una vez en esa carpeta.
      </p>
      {state.status === "loading" && (
        <p className="panel__muted" role="status">
          Comprobando la carpeta de Codex…
        </p>
      )}
      {state.status === "error" && (
        <div>
          <p className="panel__error" role="alert">
            No se pudo comprobar la carpeta de Codex: {state.message}
          </p>
          <button
            type="button"
            className="button button--secondary"
            onClick={recheck}
          >
            Volver a comprobar
          </button>
        </div>
      )}
      {state.status === "ready" && (
        <>
          <dl className="facts facts--compact">
            <dt>Carpeta</dt>
            <dd className="facts__path">{state.data.path}</dd>
            <dt>Sesión</dt>
            <dd>
              {state.data.signedIn
                ? "Hay credenciales guardadas en la carpeta de Codex (no se comprueba si siguen siendo válidas)"
                : "No hay credenciales guardadas en la carpeta de Codex"}
            </dd>
          </dl>
          <p className="field__hint">
            Si las credenciales caducaron, Codex fallará al empezar y tendrás
            que volver a iniciar sesión. Necesitas la CLI de Codex (
            <code>codex</code>) instalada. Para iniciar sesión, ejecuta en una
            terminal:
          </p>
          <p id={`${id}-posix`} className="run-start__label">
            macOS / Linux (zsh, bash)
          </p>
          <code
            className="codex-home__command"
            aria-describedby={`${id}-posix`}
          >
            {codexLoginCommand(state.data.path)}
          </code>
          <p id={`${id}-pwsh`} className="run-start__label">
            Windows (PowerShell)
          </p>
          <code className="codex-home__command" aria-describedby={`${id}-pwsh`}>
            {codexLoginCommandPowerShell(state.data.path)}
          </code>
          <div className="run-start__actions">
            <button
              type="button"
              className="button button--secondary"
              onClick={recheck}
            >
              Volver a comprobar
            </button>
          </div>
        </>
      )}
    </section>
  );
}
