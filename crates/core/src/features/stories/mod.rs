//! Slice `stories` (mínimo, HU-004): crear y mover tarjetas aplicando la regla WIP.
//! HU-005 añade contenido (Markdown, prioridad, criterios, archivo); HU-007 el tablero.

mod create;
mod list;
mod move_story;
mod story;

pub use create::{CreateStoryInput, create_story};
pub use list::list_stories;
pub use move_story::{MoveStoryInput, move_story};
pub use story::StorySummary;
