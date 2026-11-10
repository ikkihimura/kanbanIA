//! Slice `projects`: crear, renombrar, listar y abrir proyectos.

mod create;
mod list;
mod open;
mod project;
mod rename;

pub use create::{CreateProjectInput, create_project};
pub use list::{ProjectSummary, list_projects};
pub use open::{last_opened_project, open_project};
pub use project::Project;
pub use rename::{RenameProjectInput, rename_project};

pub(crate) use project::{
    ensure_project_exists, load_project, validate_name as validate_project_name,
};
