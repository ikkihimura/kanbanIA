use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use crate::error::CoreError;

pub(super) const ENTITY: &str = "Tarjeta";
const MAX_TITLE_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StorySummary {
    pub id: String,
    pub project_id: String,
    pub column_id: String,
    #[ts(type = "number")]
    pub position: i64,
    pub title: String,
    #[ts(type = "number")]
    pub revision: i64,
}

macro_rules! select_story {
    ($tail:literal) => {
        concat!(
            "SELECT id, project_id, column_id, position, title, revision FROM stories ",
            $tail
        )
    };
}
pub(super) use select_story;

pub(super) async fn load_story(
    conn: &mut SqliteConnection,
    project_id: &str,
    id: &str,
) -> Result<Option<StorySummary>, CoreError> {
    Ok(
        sqlx::query_as(select_story!("WHERE project_id = ? AND id = ?"))
            .bind(project_id)
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

pub(super) async fn count_in_column(
    conn: &mut SqliteConnection,
    column_id: &str,
) -> Result<i64, CoreError> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM stories WHERE column_id = ?")
            .bind(column_id)
            .fetch_one(&mut *conn)
            .await?,
    )
}

pub(super) fn validate_title(raw: &str) -> Result<String, CoreError> {
    let title = raw.trim();
    let invalid = |reason: &str| CoreError::Validation {
        field: "title",
        reason: reason.to_owned(),
    };
    if title.is_empty() {
        return Err(invalid("El título es obligatorio."));
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(invalid("El título admite como máximo 200 caracteres."));
    }
    if title.chars().any(char::is_control) {
        return Err(invalid(
            "El título no puede contener caracteres de control.",
        ));
    }
    Ok(title.to_owned())
}
