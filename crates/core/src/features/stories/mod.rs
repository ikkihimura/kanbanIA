//! Slice `stories`: historias con contenido (HU-005) y movimiento con regla WIP (HU-004).
//! El tablero visual llega en HU-007; la aceptación (columnas «hecho») en HU-009.

mod archive;
mod create;
mod list;
mod move_story;
mod placement;
mod story;
mod update;

pub use archive::{ArchiveStoryInput, RestoreStoryInput, archive_story, restore_story};
pub use create::{CreateStoryInput, create_story};
pub use list::{get_story, list_stories};
pub use move_story::{MoveStoryInput, move_story};
pub use story::{AcceptanceCriterion, Priority, StoryDetail, StorySummary};
pub use update::{CriterionDraft, UpdateStoryInput, update_story};

pub(crate) use placement::{append_to_column, place_at_index};
pub(crate) use story::{load_detail, load_story};
