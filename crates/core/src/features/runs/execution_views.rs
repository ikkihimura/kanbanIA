//! Tipos de lectura y aviso de la ejecución supervisada (HU-018) para la app. Solo contrato: la
//! salida y el estado del worktree los lee el host del disco con `crates/runner`; aquí viven
//! porque solo `kanbania-core --lib` genera los tipos TS. Nada de esto se expone a agentes.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Carpeta de la salida de los intentos dentro del directorio de datos:
/// `<data_dir>/runs/<run_id>/output-<n>.jsonl`.
pub const RUN_OUTPUT_DIR: &str = "runs";

/// Origen de una línea de salida.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunOutputStream {
    Stdout,
    Stderr,
    /// Mensaje del supervisor (no del proceso), p. ej. el detalle de un error de Git.
    System,
    /// Reportado por el agente vía ACP (HU-019): lo que el agente **dice**, no algo observado
    /// por la app. El tipo va en `RunOutputLine.kind`.
    Agent,
}

/// Tipo de una línea `agent` (HU-019). Siempre `null` en los demás orígenes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunOutputKind {
    /// Texto de un mensaje del agente (agrupado por mensaje o en trozos de hasta 4 KiB).
    Message,
    /// El agente estuvo «pensando»: solo la longitud, nunca el contenido.
    Thought,
    /// Llamada a una herramienta: título, tipo y estado (sin entrada ni contenido).
    Tool,
    /// Plan del agente: una entrada por línea de texto con su estado.
    Plan,
    /// Uso de contexto y coste tal como lo informa el agente.
    Usage,
    /// Permiso que pidió el agente y cómo lo respondió la app.
    Permission,
}

/// Una línea de salida. `text` es dato no confiable: se muestra como texto plano, nunca como
/// Markdown ni HTML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunOutputLine {
    /// Hora de recepción en ms Unix (reloj del sistema; informativa).
    #[ts(type = "number")]
    pub at_ms: i64,
    pub stream: RunOutputStream,
    pub text: String,
    /// La línea superaba 4 KiB y se cortó.
    pub cut: bool,
    /// Tipo de una línea `agent`; `null` en los demás orígenes.
    pub kind: Option<RunOutputKind>,
}

/// Últimas líneas de salida de un intento (como mucho 500), de la más vieja a la más nueva.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunOutputView {
    pub lines: Vec<RunOutputLine>,
    /// Líneas ilegibles que se saltaron.
    #[ts(type = "number")]
    pub skipped_corrupt: u64,
    /// La rotación ya borró salida anterior a la que se muestra.
    pub earlier_dropped: bool,
}

/// Cambios del worktree de un intento terminado según `git status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorktreeChanges {
    /// La carpeta ya no existe (eliminarlo solo lo marcará como eliminado).
    pub missing: bool,
    /// Entradas con cambios (modificadas, borradas, renombradas, sin seguimiento…).
    #[ts(type = "number")]
    pub changes: u64,
    /// Archivos o carpetas ignorados (`.gitignore`): eliminar el worktree los borra siempre,
    /// también sin forzar. La UI debe avisarlo.
    #[ts(type = "number")]
    pub ignored: u64,
    /// `false` si la salida de Git superó el límite: los recuentos son entonces un mínimo.
    pub complete: bool,
    /// HEAD no está en la rama del intento o hay commits que no están en ninguna rama:
    /// eliminarlo sin forzar se rechaza (como con cambios).
    pub commits_at_risk: bool,
}

/// Eliminar el worktree de un intento terminado. `force` = «Descartar cambios y eliminar»:
/// solo con una acción explícita de la persona. La rama se conserva siempre.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoveRunWorktreeInput {
    pub project_id: String,
    pub run_id: String,
    pub force: bool,
}

/// Aviso `run-change` del host tras un cambio del supervisor (despachar, iniciar, terminar,
/// interrumpir…). Solo sirve para recargar: el núcleo sigue siendo la autoridad.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunChange {
    pub run_ids: Vec<String>,
    pub story_ids: Vec<String>,
}
