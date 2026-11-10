//! Slice `repositories`: vincular, reubicar, desvincular y listar repositorios Git locales
//! de un proyecto. Nunca modifica el contenido de los repositorios. Política de rutas en
//! `git_path`.

mod git_path;
mod link;
mod list;
mod relocate;
mod repository;
mod unlink;

pub use git_path::{RepositoryStatus, repository_status};
pub use link::{LinkRepositoryInput, link_repository};
pub use list::list_repositories;
pub use relocate::{RelocateRepositoryInput, relocate_repository};
pub use repository::RepositoryLink;
pub use unlink::{UnlinkRepositoryInput, unlink_repository};
