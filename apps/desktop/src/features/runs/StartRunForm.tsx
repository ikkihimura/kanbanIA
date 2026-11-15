import { useId, useRef, useState, type Ref } from "react";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import type { SnapshotRepository } from "../../contracts/generated/SnapshotRepository";
import type { StartRunInput } from "../../contracts/generated/StartRunInput";
import type { TestScenario } from "../../contracts/generated/TestScenario";
import {
  newRequestId,
  startRun,
  type UiError,
} from "../../shared/ipc/commands";
import { useMutation } from "../../shared/ipc/useMutation";
import { FieldError } from "../../shared/ui/FieldError";
import { SCENARIOS, SCENARIO_LABEL, TEST_PROCESS_LABEL } from "./runFormat";

const NO_REPOSITORY_HINT =
  "Ningún repositorio de la instantánea tiene commit: vincula al proyecto un repositorio con commits y prepara otro intento.";

interface Props {
  projectId: string;
  runId: string;
  runNumber: number;
  /** Repositorios congelados en la instantánea (solo los que tienen commit sirven). */
  repositories: SnapshotRepository[];
  /** Referencia a la opción de escenario elegida, para llevar el foco desde la lista. */
  scenarioRef?: Ref<HTMLInputElement>;
  onStarted: (run: RunSummary) => void;
  /** El núcleo rechazó el inicio porque el intento ya cambió: hay que volver a leer. */
  onStale: () => void;
}

/**
 * Pedir el inicio de un intento en cola (HU-018) con el proceso de prueba incluido. El núcleo
 * es la autoridad: valida el escenario, el repositorio y que la historia siga ejecutable.
 */
export function StartRunForm({
  projectId,
  runId,
  runNumber,
  repositories,
  scenarioRef,
  onStarted,
  onStale,
}: Props) {
  const id = useId();
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
  const scenarioError = error?.field === "scenario" ? error : null;
  const repositoryError = error?.field === "repositoryId" ? error : null;
  const generalError =
    error && !scenarioError && !repositoryError ? error : null;

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
      scenario,
    };
    if (choose) input.repositoryId = repositoryId;
    const result = await start.mutate(input);
    if (result.ok) {
      // Se conserva el requestId: repetir el envío (doble pulsación) es la misma petición.
      onStarted(result.value);
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
        instantánea y lanza en él un proceso de prueba incluido en la app. No
        trabaja ningún agente ni IA: sirve para probar la supervisión. Tu
        repositorio original no se modifica.
      </p>

      {/* Opciones siempre visibles (no un desplegable): con un desplegable que ya muestra
          «Termina bien», era fácil no ver que había que elegir. */}
      <fieldset
        className="field run-start__scenarios"
        aria-describedby={scenarioError ? `${id}-scenario-error` : undefined}
      >
        <legend className="run-start__label">
          Escenario del proceso de prueba
        </legend>
        {SCENARIOS.map((s) => (
          <label key={s} className="checkbox">
            <input
              type="radio"
              name={`${id}-scenario`}
              value={s}
              ref={s === scenario ? scenarioRef : undefined}
              checked={s === scenario}
              onChange={() => {
                setScenario(s);
                touch();
              }}
            />
            {SCENARIO_LABEL[s]}
          </label>
        ))}
        <FieldError id={`${id}-scenario-error`} error={scenarioError} />
      </fieldset>

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
          {TEST_PROCESS_LABEL}
        </span>
      </div>
    </form>
  );
}
