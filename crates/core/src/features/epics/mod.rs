//! Slice `epics`: crear, editar y listar épicas con su avance (regla en `EpicProgress`).
//! La pertenencia de historias a épicas se gestiona en el slice `relations`.

mod create;
mod epic;
mod list;
mod update;

pub use create::{CreateEpicInput, create_epic};
pub use epic::{Epic, EpicProgress};
pub use list::list_epics;
pub use update::{UpdateEpicInput, update_epic};

pub(crate) use epic::load_epic;
