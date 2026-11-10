pub mod app_info;
pub mod columns;
pub mod epics;
pub mod exit;
pub mod projects;
pub mod relations;
pub mod repositories;
pub mod review;
pub mod storage;
pub mod stories;
pub mod team;

/// Actor de las mutaciones que llegan desde la ventana de escritorio (recibos de idempotencia).
pub(crate) const UI_ACTOR: &str = "ui";
