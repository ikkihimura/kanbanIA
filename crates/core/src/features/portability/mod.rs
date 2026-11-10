//! Slice `portability` (HU-010): exportar e importar la planificación como una carpeta de
//! Markdown con manifiesto versionado. Es una operación explícita: nunca hay sincronización
//! automática (ni con Git). Solo personas (`Actor::Human`).
//!
//! - Exporta historias (también archivadas), épicas, criterios, relaciones y columnas; el
//!   estado de revisión va como dato informativo. Excluye repositorios (rutas absolutas),
//!   entregas, recibos, perfil y datos de agentes salvo id y nombre.
//! - Importa en dos pasos: vista previa (plan) y aplicación en una transacción. Empareja por
//!   id: reimportar no duplica. Nunca borra datos locales ni importa aceptaciones.
//!
//! Decisiones:
//! - Escribe en tablas de otros slices dentro de **una** transacción (los casos de uso abren
//!   la suya y no permiten una importación atómica), reutilizando sus reglas `pub(crate)`:
//!   validadores, `ensure_capacity`, comprobación de ciclos y `stories::replace_document`.
//!   Una regla nueva en crear/editar historias, épicas, columnas o relaciones debe aplicarse
//!   también aquí.
//! - Como el editor: no modifica historias archivadas aquí ni crea relaciones nuevas hacia
//!   ellas. Las relaciones de historias que llegan archivadas se restauran tal cual.
//! - Sin `request_id`: emparejar por id ya hace idempotente la reimportación, y la huella del
//!   plan impide aplicar algo distinto de lo previsualizado.
//! - Las historias y épicas no se borran en la app, así que nada «reaparece» al reimportar.
//! - Formato: todo cambio (campos o hash de contenido) sube `VERSION`; las revisiones del
//!   manifiesto son informativas.

mod apply;
mod export;
mod package;
mod plan;
mod snapshot;

pub use apply::{
    ApplyImportInput, ImportResolution, ImportResult, ResolutionChoice, apply_import,
    preview_import,
};
pub use export::{ExportProjectInput, ExportResult, export_project};
pub use plan::{ImportAction, ImportEntityKind, ImportItem, ImportPreview};
