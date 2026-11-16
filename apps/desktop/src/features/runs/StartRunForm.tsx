import { useId, useRef, useState, type Ref } from "react";
import type { AcpAgent } from "../../contracts/generated/AcpAgent";
import type { CodexHomeStatus } from "../../contracts/generated/CodexHomeStatus";
import type { RunSettings } from "../../contracts/generated/RunSettings";
import type { RunExecutor } from "../../contracts/generated/RunExecutor";
import type { RunSummary } from "../../contracts/generated/RunSummary";
import type { SnapshotRepository } from "../../contracts/generated/SnapshotRepository";
import type { StartRunInput } from "../../contracts/generated/StartRunInput";
import type { TestScenario } from "../../contracts/generated/TestScenario";
import {
  getCodexHomeStatus,
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
import { AgentComparison } from "./AgentComparison";
import {
  ACP_AGENT_NAME,
  acpAgentOf,
  CLAUDE_ACP_LABEL,
  CODEX_ACP_LABEL,
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
 * Pedir el inicio de un intento en cola (HU-018/019/021) con el ejecutor elegido: el proceso
 * de prueba incluido (sin IA), Claude Code o Codex vía ACP. El núcleo es la autoridad: valida
 * el ejecutor (ACP exige un adaptador configurado), el repositorio y que la historia siga
 * ejecutable. La configuración y la sesión de Codex solo sirven para deshabilitar lo que no
 * se puede usar y explicar por qué.
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
  const codexHome = useIpcResource(getCodexHomeStatus);
  const ready = settings.state.status === "ready" ? settings.state.data : null;
  const available: Record<AcpAgent, boolean> = {
    claudeCode: ready !== null && ready.claudeAcpPath !== null,
    codex:
      ready !== null &&
      ready.codexAcpPath !== null &&
      codexHome.state.status === "ready" &&
      codexHome.state.data.signedIn,
  };
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
    kind === "testProcess" ? { kind, scenario } : { kind };

  // La opción marcada puede dejar de estar disponible después de elegirla (al volver a
  // comprobar la sesión de Codex o al releer la configuración): nunca se envía.
  const selectedAgent = acpAgentOf(executor);
  const selectedUnavailable =
    selectedAgent !== null && !available[selectedAgent];

  async function handleSubmit() {
    if (selectedAgent !== null && selectedUnavailable) {
      setLocalError({
        code: "validation",
        field: "executor",
        message: `${ACP_AGENT_NAME[selectedAgent]} ya no está disponible (el motivo está junto a su opción): elige otro ejecutor.`,
      });
      return;
    }
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
        {settings.state.status === "error" && (
          <div className="field__hint">
            <p id={`${id}-settings-error`}>
              Claude Code y Codex no disponibles: no se pudo leer la
              configuración ({settings.state.message}).
            </p>
            <button
              type="button"
              className="button button--secondary"
              onClick={() => {
                settings.run();
              }}
            >
              Volver a leer la configuración
            </button>
          </div>
        )}
        {ACP_OPTIONS.map(({ agent, kind: optionKind, label }) => {
          const reasonId =
            settings.state.status === "error"
              ? `${id}-settings-error`
              : `${id}-${agent}-unavailable`;
          return (
            <div key={agent}>
              <label className="checkbox">
                <input
                  type="radio"
                  name={`${id}-executor-kind`}
                  value={optionKind}
                  ref={kind === optionKind ? focusRef : undefined}
                  checked={kind === optionKind}
                  disabled={!available[agent]}
                  aria-describedby={available[agent] ? undefined : reasonId}
                  onChange={() => {
                    setKind(optionKind);
                    touch();
                  }}
                />
                {label}
              </label>
              {!available[agent] && settings.state.status !== "error" && (
                <AcpUnavailable
                  id={reasonId}
                  agent={agent}
                  settings={settings.state}
                  codexHome={codexHome.state}
                />
              )}
              {/* Siempre que haya adaptador: tras iniciar (o cerrar) sesión en una terminal,
                  se vuelve a comprobar sin cerrar el formulario. */}
              {agent === "codex" &&
                ready?.codexAcpPath != null &&
                codexHome.state.status !== "loading" && (
                  <button
                    type="button"
                    className="button button--secondary"
                    onClick={() => {
                      codexHome.run(getCodexHomeStatus, { keepData: true });
                    }}
                  >
                    Volver a comprobar la sesión de Codex
                  </button>
                )}
            </div>
          );
        })}
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
      ) : kind === "claudeAcp" ? (
        <p className="run-detail__info">
          Lanza Claude Code en el worktree con las instrucciones de la
          instantánea. El agente trabaja con tu sesión de Claude Code. Cada
          petición de permiso (editar, ejecutar…) aparece en el detalle del
          intento para que la permitas una vez o la rechaces; sin respuesta en
          10 minutos, se rechaza.
        </p>
      ) : (
        <div className="run-detail__info">
          <p>
            Lanza Codex en el worktree con las instrucciones de la instantánea y
            la carpeta de Codex de la app. Cada petición de permiso para editar
            o ejecutar comandos con efectos aparece en el detalle del intento
            para que la permitas una vez o la rechaces; sin respuesta en 10
            minutos, se rechaza.
          </p>
          <p>
            Atención (comprobado con Codex real): lee archivos sin pedir
            permiso, también fuera del worktree (podría leer los datos de la
            app), y rechazar una escritura detiene su turno.
          </p>
        </div>
      )}

      <p className="field__hint">
        Con Claude Code o Codex, el intento se atribuye al agente del equipo
        asignado a la historia: su nombre y su avatar no cambian según el
        ejecutor que elijas.
      </p>
      <details>
        <summary>Comparar Claude Code y Codex</summary>
        <AgentComparison />
      </details>

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

const ACP_OPTIONS: readonly {
  agent: AcpAgent;
  kind: Exclude<ExecutorKind, "testProcess">;
  label: string;
}[] = [
  { agent: "claudeCode", kind: "claudeAcp", label: CLAUDE_ACP_LABEL },
  { agent: "codex", kind: "codexAcp", label: CODEX_ACP_LABEL },
];

const ADAPTER_EXECUTABLE: Record<AcpAgent, string> = {
  claudeCode: "claude-agent-acp",
  codex: "codex-acp",
};

interface UnavailableProps {
  id: string;
  agent: AcpAgent;
  settings: ResourceState<RunSettings>;
  codexHome: ResourceState<CodexHomeStatus>;
}

/**
 * Por qué una opción ACP no se puede elegir, siempre en texto. El error al leer la
 * configuración lo muestra el formulario una sola vez (sirve para los dos agentes).
 */
function AcpUnavailable({ id, agent, settings, codexHome }: UnavailableProps) {
  const name = ACP_AGENT_NAME[agent];
  if (settings.status === "loading")
    return (
      <p id={id} className="field__hint">
        Comprobando si hay un adaptador de {name} configurado…
      </p>
    );
  if (settings.status !== "ready") return null;
  const path =
    agent === "claudeCode"
      ? settings.data.claudeAcpPath
      : settings.data.codexAcpPath;
  if (path === null)
    return (
      <p id={id} className="field__hint">
        No disponible: configura el adaptador de {name} en Sistema → Ejecuciones
        (ruta de <code>{ADAPTER_EXECUTABLE[agent]}</code>, al final de la
        ventana).
      </p>
    );
  // Solo Codex llega aquí: necesita además sesión iniciada en la carpeta de la app.
  if (codexHome.status === "loading")
    return (
      <p id={id} className="field__hint">
        Comprobando si hay sesión iniciada en la carpeta de Codex de la app…
      </p>
    );
  if (codexHome.status === "error")
    return (
      <p id={id} className="field__hint">
        No disponible: no se pudo comprobar la sesión de Codex (
        {codexHome.message}).
      </p>
    );
  return (
    <p id={id} className="field__hint">
      No disponible: inicia sesión en la carpeta de Codex de la app (Sistema →
      Ejecuciones).
    </p>
  );
}
