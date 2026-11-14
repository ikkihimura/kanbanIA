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

pub(crate) use create::{NewStory, insert_story};
pub(crate) use placement::{append_to_column, place_at_index};
pub(crate) use story::{
    MAX_CRITERIA, count_in_column, load_detail, load_story, validate_criterion,
    validate_description as validate_story_description, validate_title as validate_story_title,
};
pub(crate) use update::{Document, replace_document};
