use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use crate::error::CoreError;

pub(super) const ENTITY: &str = "Proyecto";
const MAX_NAME_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Project {
    pub id: String,
    pub name: String,
    #[ts(type = "number")]
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: Option<String>,
}

/// SQL estático (sqlx 0.9 rechaza cadenas dinámicas): `select_project!("WHERE ...")`.
macro_rules! select_project {
    ($tail:literal) => {
        concat!(
            "SELECT id, name, revision, created_at, updated_at, last_opened_at FROM projects ",
            $tail
        )
    };
}
pub(super) use select_project;

pub(crate) async fn load_project(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<Project>, CoreError> {
    Ok(sqlx::query_as(select_project!("WHERE id = ?"))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?)
}

pub(crate) async fn ensure_project_exists(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<(), CoreError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM projects WHERE id = ?)")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    if exists {
        Ok(())
    } else {
        Err(CoreError::NotFound { entity: ENTITY })
    }
}

/// Normaliza y valida el nombre: recorta espacios, 1–120 caracteres, sin caracteres de control.
pub(crate) fn validate_name(raw: &str) -> Result<String, CoreError> {
    let name = raw.trim();
    let invalid = |reason: &str| CoreError::Validation {
        field: "name",
        reason: reason.to_owned(),
    };
    if name.is_empty() {
        return Err(invalid("El nombre no puede estar vacío."));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(invalid("El nombre admite como máximo 120 caracteres."));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid(
            "El nombre no puede contener caracteres de control.",
        ));
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_trimmed_and_accepts_unicode() {
        assert_eq!(
            validate_name("  Señal 🚀 Kanban  ").unwrap(),
            "Señal 🚀 Kanban"
        );
        assert_eq!(
            validate_name(&"ñ".repeat(120)).unwrap().chars().count(),
            120
        );
    }

    #[test]
    fn invalid_names_are_rejected_with_field() {
        for bad in ["", "   ", "a\nb", "tab\there", &"x".repeat(121)] {
            let error = validate_name(bad).unwrap_err();
            assert!(
                matches!(error, CoreError::Validation { field: "name", .. }),
                "{bad:?}"
            );
        }
    }
}
