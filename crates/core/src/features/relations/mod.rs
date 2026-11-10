//! Slice `relations`: épica, subtareas (padre) y dependencias de una historia.
//!
//! - Las relaciones son aristas propias: cambiarlas no altera la revisión de la historia, así
//!   que no entran en conflicto con un borrador del editor. Las operaciones son idempotentes
//!   (fijar a X, añadir si falta, quitar si existe): reintentarlas es seguro.
//! - Nunca se relacionan historias de proyectos distintos (núcleo + claves foráneas compuestas)
//!   ni se aceptan ciclos de subtareas o dependencias (comprobado en la transacción).
//! - Una dependencia sin aceptar se expone como prerequisito pendiente. No impide mover la
//!   tarjeta: el bloqueo de *ejecución* llega con reclamaciones (HU-014) y ejecuciones (M3).

mod dependencies;
mod get;
mod graph;
mod set_agent;
mod set_epic;
mod set_parent;

pub use dependencies::{DependencyInput, add_story_dependency, remove_story_dependency};
pub use get::{AgentRef, EpicRef, StoryRelations, get_story_relations};
pub use graph::{RelationState, StoryRef};
pub use set_agent::{SetStoryAgentInput, set_story_agent};
pub use set_epic::{SetStoryEpicInput, set_story_epic};
pub use set_parent::{SetStoryParentInput, set_story_parent};

pub(crate) use dependencies::MAX_PREREQUISITES;
pub(crate) use graph::{is_self_or_ancestor, reaches_through_dependencies};
