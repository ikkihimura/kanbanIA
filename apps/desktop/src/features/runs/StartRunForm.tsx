import { useId, useRef, useState, type Ref } from "react";
import type { RunExecutor } from "../../contracts/generated/RunExecutor";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import type { SnapshotRepository } from "../../contracts/generated/SnapshotRepository";
import type { StartRunInput } from "../../contracts/generated/StartRunInput";
import type { TestScenario } from "../../contracts/generated/TestScenario";
import {
  getRunSettings,
  newRequestId,
  startRun,
  type UiError,
} from "../../shared/ipc/commands";
import {
  useIpcResource,
  type ResourceState,
} from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { FieldError } from "../../shared/ui/FieldError";
import {
  CLAUDE_ACP_LABEL,
  SCENARIOS,
  SCENARIO_LABEL,
  TEST_PROCESS_LABEL,
  executorLabel,
} from "./runFormat";

type ExecutorKind = RunExecutor["kind"];

const NO_REPOSITORY_HINT =
  "Ningún repositorio de la instantánea tiene commit: vincula al proyecto un repositorio con commits y prepara otro intento.";

interface Props {
  projectId: string;
  runId: string;
  runNumber: number;
  /** Repositorios congelados en la instantánea (solo los que tienen commit sirven). */
  repositories: SnapshotRepository[];
  /** Referencia a la opción de ejecutor elegida, para llevar el foco desde la lista. */
  focusRef?: Ref<HTMLInputElement>;
  onStarted: (run: RunSummary, executor: RunExecutor) => void;
  /** El núcleo rechazó el inicio porque el intento ya cambió: hay que volver a leer. */
  onStale: () => void;
}

/**
 * Pedir el inicio de un intento en cola (HU-018/019) con el ejecutor elegido: el proceso de
 * prueba incluido (sin IA) o Claude Code vía ACP. El núcleo es la autoridad: valida el
 * ejecutor (ACP exige un adaptador configurado), el repositorio y que la historia siga
 * ejecutable. La configuración solo sirve para explicar por qué ACP no está disponible.
 */
export function StartRunForm({
  projectId,
  runId,
  runNumber,
  repositories,
  focusRef,
  onStarted,
  onStale,
}: Props) {
  const id = useId();
  const settings = useIpcResource(getRunSettings);
  const acpConfigured =
    settings.state.status === "ready" &&
    settings.state.data.claudeAcpPath !== null;
  // Por defecto, el proceso de prueba: no lanza IA ni consume saldo sin elegirlo.
  const [kind, setKind] = useState<ExecutorKind>("testProcess");
  const [scenario, setScenario] = useState<TestScenario>("success");
  const [repositoryId, setRepositoryId] = useState("");
  const [localError, setLocalError] = useState<UiError | null>(null);
  // Se conserva al reintentar el mismo envío; cambia al editar.
  const requestId = useRef(newRequestId());
  const start = useMutation(startRun);
  const touch = () => {
    requestId.current = newRequestId();
    start.reset();
    setLocalError(null);
  };

  const withCommit = repositories.filter((r) => r.head !== null);
  const single = withCommit.length === 1 ? withCommit[0] : undefined;
  const choose = withCommit.length > 1;

  const error = localError ?? start.error;
  const executorError =
    error?.field === "executor" || error?.field === "scenario" ? error : null;
  const repositoryError = error?.field === "repositoryId" ? error : null;
  const generalError =
    error && !executorError && !repositoryError ? error : null;
  const executor: RunExecutor =
    kind === "claudeAcp" ? { kind: "claudeAcp" } : { kind, scenario };

  async function handleSubmit() {
    if (choose && repositoryId === "") {
      setLocalError({
        code: "validation",
        field: "repositoryId",
        message: "Elige el repositorio donde crear el worktree.",
      });
      return;
    }
    const input: StartRunInput = {
      requestId: requestId.current,
      projectId,
      runId,
      executor,
    };
    if (choose) input.repositoryId = repositoryId;
    const result = await start.mutate(input);
    if (result.ok) {
      // Se conserva el requestId: repetir el envío (doble pulsación) es la misma petición.
      onStarted(result.value, executor);
    } else if (
      result.error.code === "invalid_run_transition" ||
      result.error.code === "not_found" ||
      result.error.field === "runId"
    ) {
      onStale();
    }
  }

  const repoErrorId = `${id}-repo-error`;
  const submitDescribedBy = [
    `${id}-executor`,
    // Sin selector, el error del repositorio describe el botón.
    !choose && repositoryError ? repoErrorId : null,
    // Rechazos generales (historia bloqueada, ejecución no disponible…) también lo describen.
    generalError ? `${id}-general-error` : null,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <form
      className="team-form run-start"
      aria-label={`Iniciar el intento #${String(runNumber)}`}
      onSubmit={(event) => {
        event.preventDefault();
        void handleSubmit();
      }}
    >
      <p className="field__hint">
        Crea un worktree y una rama nuevos desde el commit congelado en la
        instantánea y lanza en él el ejecutor elegido. Tu repositorio original
        no se modifica.
      </p>

      {/* Opciones siempre visibles (no un desplegable): con un desplegable que ya muestra
          una opción, era fácil no ver que había que elegir. */}
      <fieldset
        className="field run-start__scenarios"
        aria-describedby={executorError ? `${id}-executor-error` : undefined}
      >
        <legend className="run-start__label">Ejecutor</legend>
        <label className="checkbox">
          <input
            type="radio"
            name={`${id}-executor-kind`}
            value="testProcess"
            ref={kind === "testProcess" ? focusRef : undefined}
            checked={kind === "testProcess"}
            onChange={() => {
              setKind("testProcess");
              touch();
            }}
          />
          {TEST_PROCESS_LABEL}
        </label>
        <label className="checkbox">
          <input
            type="radio"
            name={`${id}-executor-kind`}
            value="claudeAcp"
            ref={kind === "claudeAcp" ? focusRef : undefined}
            checked={kind === "claudeAcp"}
            disabled={!acpConfigured}
            aria-describedby={
              acpConfigured ? undefined : `${id}-acp-unavailable`
            }
            onChange={() => {
              setKind("claudeAcp");
              touch();
            }}
          />
          {CLAUDE_ACP_LABEL}
        </label>
        {!acpConfigured && (
          <AcpUnavailable
            id={`${id}-acp-unavailable`}
            state={settings.state}
            onRetry={() => {
              settings.run();
            }}
          />
        )}
        <FieldError id={`${id}-executor-error`} error={executorError} />
      </fieldset>

      {kind === "testProcess" ? (
        <>
          <p className="field__hint">
            Lanza un proceso de prueba incluido en la app. No trabaja ningún
            agente ni IA: sirve para probar la supervisión.
          </p>
          <fieldset className="field run-start__scenarios">
            <legend className="run-start__label">
              Escenario del proceso de prueba
            </legend>
            {SCENARIOS.map((s) => (
              <label key={s} className="checkbox">
                <input
                  type="radio"
                  name={`${id}-scenario`}
                  value={s}
                  checked={s === scenario}
                  onChange={() => {
                    setScenario(s);
                    touch();
                  }}
                />
                {SCENARIO_LABEL[s]}
              </label>
            ))}
          </fieldset>
        </>
      ) : (
        <p className="run-detail__info">
          Lanza Claude Code en el worktree con las instrucciones de la
          instantánea. El agente trabaja con tu sesión de Claude Code. Hasta
          HU-020 la app rechaza todas sus peticiones de permiso (editar,
          ejecutar…), así que normalmente solo podrá leer y responder.
        </p>
      )}

      {choose ? (
        <div className="field">
          <label htmlFor={`${id}-repo`}>Repositorio del worktree</label>
          <select
            id={`${id}-repo`}
            value={repositoryId}
            aria-invalid={repositoryError ? true : undefined}
            aria-describedby={repositoryError ? repoErrorId : undefined}
            onChange={(event) => {
              setRepositoryId(event.target.value);
              touch();
            }}
          >
            <option value="">Elige un repositorio…</option>
            {withCommit.map((r) => (
              <option key={r.id} value={r.id}>
                {r.path} ({(r.head ?? "").slice(0, 8)})
              </option>
            ))}
          </select>
          <FieldError id={repoErrorId} error={repositoryError} />
        </div>
      ) : (
        <div className="field">
          <span className="run-start__label">Repositorio del worktree</span>
          {single ? (
            <span className="repo__path">
              {single.path} (commit {(single.head ?? "").slice(0, 8)})
            </span>
          ) : (
            <span className="field__hint">{NO_REPOSITORY_HINT}</span>
          )}
          <FieldError id={repoErrorId} error={repositoryError} />
        </div>
      )}

      {generalError && (
        <div className="run-form__alert" role="alert">
          <p id={`${id}-general-error`}>
            No se pudo iniciar el intento: {generalError.message}
          </p>
        </div>
      )}

      <div className="run-start__actions">
        <button
          type="submit"
          className="button"
          aria-busy={start.pending}
          disabled={start.pending}
          aria-describedby={submitDescribedBy}
        >
          Iniciar
        </button>
        <span id={`${id}-executor`} className="badge run-start__executor">
          {executorLabel(executor)}
        </span>
      </div>
    </form>
  );
}

interface UnavailableProps {
  id: string;
  state: ResourceState<unknown>;
  onRetry: () => void;
}

/** Por qué «Claude Code (ACP)» no se puede elegir, siempre en texto. */
function AcpUnavailable({ id, state, onRetry }: UnavailableProps) {
  if (state.status === "loading")
    return (
      <p id={id} className="field__hint">
        Comprobando si hay un adaptador de Claude Code configurado…
      </p>
    );
  if (state.status === "error")
    return (
      <div className="field__hint">
        <p id={id}>
          No disponible: no se pudo leer la configuración ({state.message}).
        </p>
        <button
          type="button"
          className="button button--secondary"
          onClick={onRetry}
        >
          Volver a leer la configuración
        </button>
      </div>
    );
  return (
    <p id={id} className="field__hint">
      No disponible: no hay ningún adaptador de Claude Code configurado. Indica
      la ruta de <code>claude-agent-acp</code> en «Sistema → Ejecuciones» (al
      final de la ventana).
    </p>
  );
}
