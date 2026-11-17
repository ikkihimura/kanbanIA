import type { AcpAgent } from "../../contracts/generated/AcpAgent";
import { ACP_AGENT_NAME, NOT_AVAILABLE } from "./runFormat";

/** Texto de una celda; `unverified` = deducido del código del adaptador, sin prueba real. */
interface Cell {
  text: string;
  unverified?: true;
}

interface Row {
  topic: string;
  values: Record<AcpAgent, Cell>;
}

/** Leyenda de la marca de lo no comprobado en real (se quitará al confirmarlo). */
export const UNVERIFIED_NOTE =
  "según el código del adaptador 2.5.0; sin comprobar en real";

const said = (text: string): Cell => ({ text });
const fromCode = (text: string): Cell => ({ text, unverified: true });

/**
 * Qué hace cada agente real dentro de la app (HU-021, CA-3). Solo lo comprobado: tokens,
 * coste y reanudación no se afirman para ninguno, y lo no probado se dice «no comprobado».
 * Texto, nunca color.
 */
export const AGENT_COMPARISON: readonly Row[] = [
  {
    topic: "Editar o ejecutar comandos con efectos",
    values: {
      claudeCode: said("Te pide permiso (Permitir una vez / Rechazar)."),
      codex: said("Te pide permiso (Permitir una vez / Rechazar)."),
    },
  },
  {
    topic: "Leer archivos",
    values: {
      claudeCode: said(
        "Comandos de solo lectura dentro del worktree sin preguntar; para leer fuera del worktree pide permiso.",
      ),
      codex: said(
        "Lee sin preguntar, también fuera del worktree, salvo los datos de la app (fila siguiente).",
      ),
    },
  },
  {
    topic: "Datos de Kanban IA (base, copias, descriptor del servicio)",
    values: {
      claudeCode: said(
        "En macOS, un sandbox del sistema le impide leerlos o modificarlos directamente, aunque apruebes el permiso; un comando aprobado podría lanzar procesos fuera del sandbox. En Windows no hay sandbox: podría leerlos.",
      ),
      codex: fromCode(
        "La app le da un perfil de permisos que su sandbox aplica a sus comandos (comprobado con «codex sandbox»; en la app, el modelo no intentó leerlos). Windows sin comprobar. Un comando que apruebes para salir del sandbox ya no queda limitado.",
      ),
    },
  },
  {
    topic: "Al rechazar una escritura",
    values: {
      claudeCode: said("El agente sigue con su turno."),
      codex: said(
        "Su turno se detiene: Codex no admite «rechazar y continuar» al escribir archivos.",
      ),
    },
  },
  {
    topic: "Red",
    values: {
      claudeCode: said("No comprobado."),
      codex: fromCode("Pide permiso; sin permiso, su sandbox la bloquea."),
    },
  },
  {
    topic: "Tu configuración",
    values: {
      claudeCode: said(
        "No carga tus ajustes de Claude Code ni los del repositorio; usa tu sesión iniciada.",
      ),
      codex: said(
        "Usa una carpeta de Codex propia de la app, sin tu configuración, servidores MCP, reglas ni historial, sin tus skills (~/.agents) ni los plugins de tu cuenta; solo ve sus skills de sistema. No se lanza si el repositorio contiene .codex/.",
      ),
    },
  },
  {
    topic: "Llamadas adicionales al modelo",
    values: {
      claudeCode: said("No comprobado."),
      codex: fromCode("Una, para poner título a la sesión."),
    },
  },
  {
    topic: "Tokens y coste",
    values: {
      claudeCode: said(`No se guardan en el intento (${NOT_AVAILABLE}).`),
      codex: said(`No se guardan en el intento (${NOT_AVAILABLE}).`),
    },
  },
  {
    topic: "Retomar una sesión",
    values: {
      claudeCode: said("No disponible en la app."),
      codex: said("No disponible en la app."),
    },
  },
];

const AGENTS: readonly AcpAgent[] = ["claudeCode", "codex"];

/** Tabla accesible (cabeceras de fila y columna) con las diferencias entre agentes. */
export function AgentComparison() {
  return (
    <div className="agent-compare">
      <table>
        <caption>Qué admite cada agente en la app</caption>
        <thead>
          <tr>
            <th scope="col">Aspecto</th>
            {AGENTS.map((agent) => (
              <th key={agent} scope="col">
                {ACP_AGENT_NAME[agent]}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {AGENT_COMPARISON.map((row) => (
            <tr key={row.topic}>
              <th scope="row">{row.topic}</th>
              {AGENTS.map((agent) => (
                <td key={agent}>
                  <CellText cell={row.values[agent]} />
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <p className="field__hint">
        <span aria-hidden="true">*</span> Dato {UNVERIFIED_NOTE}.
      </p>
    </div>
  );
}

/**
 * Celda con la marca «*» si no está comprobada en real. Para lectores de pantalla, la marca
 * se lee como la leyenda completa (no dependen de asociar el asterisco).
 */
function CellText({ cell }: { cell: Cell }) {
  if (!cell.unverified) return <>{cell.text}</>;
  return (
    <>
      {cell.text}
      <span aria-hidden="true"> *</span>
      <span className="visually-hidden"> ({UNVERIFIED_NOTE})</span>
    </>
  );
}
