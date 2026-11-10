use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use crate::error::CoreError;

pub(crate) const ENTITY: &str = "Épica";
const MAX_TITLE_CHARS: usize = 120;
const MAX_DESCRIPTION_CHARS: usize = 5_000;

/// Avance de una épica.
///
/// Regla (HU-006): `accepted / total` sobre las historias **no archivadas** de la épica;
/// las archivadas no cuentan en ningún lado. Con `total == 0` el avance no está definido:
/// `percent` es `None` y la UI muestra «Sin historias» (ni 0 % ni 100 %).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EpicProgress {
    #[ts(type = "number")]
    pub accepted: i64,
    #[ts(type = "number")]
    pub total: i64,
    /// Porcentaje entero redondeado hacia abajo; `null` sin historias.
    #[ts(type = "number | null")]
    pub percent: Option<i64>,
}

impl EpicProgress {
    pub(crate) fn new(accepted: i64, total: i64) -> Self {
        Self {
            accepted,
            total,
            percent: (total > 0).then(|| accepted * 100 / total),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Epic {
    pub id: String,
    pub project_id: String,
    /// Se muestra como E-n.
    #[ts(type = "number")]
    pub number: i64,
    pub title: String,
    pub description: String,
    #[ts(type = "number")]
    pub revision: i64,
    pub progress: EpicProgress,
}

#[derive(sqlx::FromRow)]
struct EpicRow {
    id: String,
    project_id: String,
    number: i64,
    title: String,
    description: String,
    revision: i64,
    accepted: i64,
    total: i64,
}

impl From<EpicRow> for Epic {
    fn from(row: EpicRow) -> Self {
        Self {
            progress: EpicProgress::new(row.accepted, row.total),
            id: row.id,
            project_id: row.project_id,
            number: row.number,
            title: row.title,
            description: row.description,
            revision: row.revision,
        }
    }
}

macro_rules! select_epic {
    ($tail:literal) => {
        concat!(
            "SELECT e.id, e.project_id, e.number, e.title, e.description, e.revision, \
             (SELECT COUNT(*) FROM story_epics se JOIN stories s ON s.id = se.story_id \
              WHERE se.epic_id = e.id AND s.archived_at IS NULL \
                AND s.accepted_at IS NOT NULL) AS accepted, \
             (SELECT COUNT(*) FROM story_epics se JOIN stories s ON s.id = se.story_id \
              WHERE se.epic_id = e.id AND s.archived_at IS NULL) AS total \
             FROM epics e ",
            $tail
        )
    };
}

pub(crate) async fn load_epic(
    conn: &mut SqliteConnection,
    project_id: &str,
    id: &str,
) -> Result<Option<Epic>, CoreError> {
    let row: Option<EpicRow> = sqlx::query_as(select_epic!("WHERE e.project_id = ? AND e.id = ?"))
        .bind(project_id)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row.map(Epic::from))
}

pub(super) async fn epics_of_project(
    conn: &mut SqliteConnection,
    project_id: &str,
) -> Result<Vec<Epic>, CoreError> {
    let rows: Vec<EpicRow> =
        sqlx::query_as(select_epic!("WHERE e.project_id = ? ORDER BY e.number"))
            .bind(project_id)
            .fetch_all(&mut *conn)
            .await?;
    Ok(rows.into_iter().map(Epic::from).collect())
}

pub(crate) fn validate_title(raw: &str) -> Result<String, CoreError> {
    let title = raw.trim();
    let invalid = |reason: &str| CoreError::Validation {
        field: "title",
        reason: reason.to_owned(),
    };
    if title.is_empty() {
        return Err(invalid("El título de la épica es obligatorio."));
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(invalid("El título admite como máximo 120 caracteres."));
    }
    if title.chars().any(char::is_control) {
        return Err(invalid(
            "El título no puede contener caracteres de control.",
        ));
    }
    Ok(title.to_owned())
}

pub(crate) fn validate_description(raw: &str) -> Result<String, CoreError> {
    if raw.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(CoreError::Validation {
            field: "description",
            reason: "La descripción de la épica admite como máximo 5 000 caracteres.".into(),
        });
    }
    if raw
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(CoreError::Validation {
            field: "description",
            reason: "La descripción contiene caracteres de control no válidos.".into(),
        });
    }
    Ok(raw.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_rule_and_zero_stories() {
        assert_eq!(EpicProgress::new(0, 0).percent, None);
        assert_eq!(EpicProgress::new(0, 4).percent, Some(0));
        assert_eq!(EpicProgress::new(1, 3).percent, Some(33));
        assert_eq!(EpicProgress::new(3, 3).percent, Some(100));
    }
}
