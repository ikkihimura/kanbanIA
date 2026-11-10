//! Slice `columns`: columnas configurables con categoría semántica y límite WIP opcional.
//! La regla WIP (`wip`) es la única puerta de entrada de tarjetas a una columna.

mod column;
mod create;
mod delete;
mod list;
mod reorder;
mod update;
mod wip;

pub use column::{Column, ColumnCategory};
pub use create::{CreateColumnInput, create_column};
pub use delete::{DeleteColumnInput, delete_column};
pub use list::list_columns;
pub use reorder::{ReorderColumnInput, reorder_column};
pub use update::{UpdateColumnInput, update_column};

pub(crate) use column::{
    MAX_WIP, columns_of_project, load_column, seed_default_columns,
    validate_name as validate_column_name, validate_wip,
};
pub(crate) use wip::ensure_capacity;
