pub mod app_info;
pub mod columns;
pub mod projects;
pub mod repositories;
pub mod storage;

/// Actor de las mutaciones que llegan desde la ventana de escritorio (recibos de idempotencia).
pub(crate) const UI_ACTOR: &str = "ui";
