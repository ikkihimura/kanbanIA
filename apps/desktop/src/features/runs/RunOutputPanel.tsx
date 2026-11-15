import { useCallback, useEffect, useId, useLayoutEffect, useRef } from "react";
import type { RunOutputView } from "../../contracts/generated/RunOutputView";
import { getRunOutput } from "../../shared/ipc/commands";
import { useRefreshSignal } from "../../shared/ipc/useAgentChanges";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import type { RunOutputLine } from "../../contracts/generated/RunOutputLine";
import { OUTPUT_KIND_LABEL, STREAM_LABEL } from "./runFormat";

/** Mientras el proceso corre, la salida se vuelve a leer con esta pausa. */
export const OUTPUT_POLL_MS = 3000;

const count = new Intl.NumberFormat("es");

interface Props {
  projectId: string;
  runId: string;
  /** El proceso está en marcha: se refresca solo cada `OUTPUT_POLL_MS`. */
  live: boolean;
  /** Cambia con cada aviso `run-change` del intento o recarga del detalle. */
  refreshSignal: number;
}

/**
 * Últimas líneas de salida del proceso (HU-018) y de lo que reporta el agente por ACP
 * (HU-019). El texto lo escribe el proceso o el agente: es dato no confiable y se muestra
 * siempre como texto plano (nunca Markdown ni HTML), con su origen en cada línea.
 */
export function RunOutputPanel({
  projectId,
  runId,
  live,
  refreshSignal,
}: Props) {
  const id = useId();
  const load = useCallback(
    () => getRunOutput(projectId, runId),
    [projectId, runId],
  );
  const { state, run } = useIpcResource(load);
  const reload = useCallback(() => {
    run(load, { keepData: true });
  }, [run, load]);
  useRefreshSignal(refreshSignal, reload);

  // Un solo temporizador a la vez: se crea al empezar a correr y se limpia al parar o desmontar.
  useEffect(() => {
    if (!live) return;
    const timer = setInterval(reload, OUTPUT_POLL_MS);
    return () => {
      clearInterval(timer);
    };
  }, [live, reload]);

  return (
    <section className="run-output" aria-labelledby={`${id}-title`}>
      <div className="repo__summary">
        <h4 id={`${id}-title`} className="relations__title">
          Salida del proceso
        </h4>
        <button
          type="button"
          className="button button--secondary"
          onClick={reload}
        >
          Actualizar <span className="visually-hidden">la salida</span>
        </button>
      </div>
      <p className="field__hint">
        Últimas líneas (como mucho 500), tal como llegaron. Cada una indica su
        origen: «stdout» y «stderr» del proceso, «app» para lo que observó la
        app y «agente (reportado)» para lo que dice el agente, sin comprobar.
        {live && " Se actualiza sola cada pocos segundos mientras corre."}
      </p>

      {state.status === "loading" && (
        <p className="panel__muted" role="status">
          Cargando salida…
        </p>
      )}
      {state.status === "error" && (
        <p className="panel__error" role="alert">
          No se pudo leer la salida: {state.message}
        </p>
      )}
      {state.status === "ready" && <OutputBody output={state.data} />}
    </section>
  );
}

/** Margen (px) para considerar que la persona está al final de la salida. */
const BOTTOM_SLACK_PX = 24;

function OutputBody({ output }: { output: RunOutputView }) {
  const cut = output.lines.some((l) => l.cut);
  // Se muestran las líneas más recientes: al cargar o recargar se desplaza al final, salvo si
  // la persona subió para leer (entonces no se mueve). No toca el foco.
  const scrollRef = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && follow.current) el.scrollTop = el.scrollHeight;
  }, [output]);
  return (
    <>
      {output.earlierDropped && (
        <p className="field__hint">
          La salida anterior se descartó por tamaño: solo se conservan las
          líneas más recientes.
        </p>
      )}
      {output.skippedCorrupt > 0 && (
        <p className="field__hint">
          Se saltaron {count.format(output.skippedCorrupt)} líneas ilegibles.
        </p>
      )}
      {cut && (
        <p className="field__hint">
          Algunas líneas superaban 4 KiB y se muestran cortadas (marcadas
          «[cortada]»).
        </p>
      )}
      {output.lines.length === 0 ? (
        <p className="panel__muted">Sin salida todavía.</p>
      ) : (
        // Región desplazable: con tabIndex se puede recorrer con el teclado.
        <div
          className="run-output__scroll"
          role="region"
          aria-label="Líneas de salida"
          tabIndex={0}
          ref={scrollRef}
          onScroll={(event) => {
            const el = event.currentTarget;
            follow.current =
              el.scrollHeight - el.scrollTop - el.clientHeight <=
              BOTTOM_SLACK_PX;
          }}
        >
          <ol className="run-output__lines">
            {output.lines.map((line, index) => (
              <li
                // Las líneas no tienen id: la posición en esta lectura basta (lista de solo lectura).
                key={`${String(line.atMs)}-${String(index)}`}
                className={lineClass(line)}
              >
                <span className="run-output__stream">[{lineOrigin(line)}]</span>{" "}
                {line.kind === "permission" && (
                  <strong className="run-output__flag">Permiso: </strong>
                )}
                <span className="run-output__text">{line.text}</span>
                {line.cut && (
                  <span className="run-output__cut"> [cortada]</span>
                )}
              </li>
            ))}
          </ol>
        </div>
      )}
    </>
  );
}

/** Origen en texto; en las líneas del agente, también el tipo («agente (reportado) · plan»). */
function lineOrigin(line: RunOutputLine): string {
  const origin = STREAM_LABEL[line.stream];
  return line.stream === "agent" && line.kind !== null
    ? `${origin} · ${OUTPUT_KIND_LABEL[line.kind]}`
    : origin;
}

function lineClass(line: RunOutputLine): string {
  const base = `run-output__line run-output__line--${line.stream}`;
  return line.kind === "permission"
    ? `${base} run-output__line--permission`
    : base;
}
