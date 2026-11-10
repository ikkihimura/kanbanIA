use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use crate::error::CoreError;

pub(crate) const ENTITY: &str = "Columna";
const MAX_NAME_CHARS: usize = 60;
const MAX_WIP: i64 = 999;

/// Categoría semántica: las reglas usan la categoría, nunca el nombre visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ColumnCategory {
    Backlog,
    Ready,
    Active,
    Review,
    Done,
}

impl ColumnCategory {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Ready => "ready",
            Self::Active => "active",
            Self::Review => "review",
            Self::Done => "done",
        }
    }

    fn parse(value: &str) -> Result<Self, CoreError> {
        Ok(match value {
            "backlog" => Self::Backlog,
            "ready" => Self::Ready,
            "active" => Self::Active,
            "review" => Self::Review,
            "done" => Self::Done,
            other => {
                return Err(CoreError::Database(sqlx::Error::Decode(
                    format!("categoría desconocida: {other}").into(),
                )));
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Column {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub category: ColumnCategory,
    #[ts(type = "number")]
    pub position: i64,
    #[ts(type = "number | null")]
    pub wip_limit: Option<i64>,
    #[ts(type = "number")]
    pub revision: i64,
    #[ts(type = "number")]
    pub card_count: i64,
}

#[derive(sqlx::FromRow)]
struct ColumnRow {
    id: String,
    project_id: String,
    name: String,
    category: String,
    position: i64,
    wip_limit: Option<i64>,
    revision: i64,
    card_count: i64,
}

impl TryFrom<ColumnRow> for Column {
    type Error = CoreError;

    fn try_from(row: ColumnRow) -> Result<Self, CoreError> {
        Ok(Self {
            category: ColumnCategory::parse(&row.category)?,
            id: row.id,
            project_id: row.project_id,
            name: row.name,
            position: row.position,
            wip_limit: row.wip_limit,
            revision: row.revision,
            card_count: row.card_count,
        })
    }
}

/// SQL estático con el número de tarjetas de cada columna.
macro_rules! select_column {
    ($tail:literal) => {
        concat!(
            "SELECT c.id, c.project_id, c.name, c.category, c.position, c.wip_limit, c.revision, \
             (SELECT COUNT(*) FROM stories s WHERE s.column_id = c.id) AS card_count \
             FROM columns c ",
            $tail
        )
    };
}

pub(crate) async fn load_column(
    conn: &mut SqliteConnection,
    project_id: &str,
    id: &str,
) -> Result<Option<Column>, CoreError> {
    let row: Option<ColumnRow> =
        sqlx::query_as(select_column!("WHERE c.project_id = ? AND c.id = ?"))
            .bind(project_id)
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    row.map(Column::try_from).transpose()
}

pub(crate) async fn columns_of_project(
    conn: &mut SqliteConnection,
    project_id: &str,
) -> Result<Vec<Column>, CoreError> {
    let rows: Vec<ColumnRow> = sqlx::query_as(select_column!(
        "WHERE c.project_id = ? ORDER BY c.position, c.id"
    ))
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter().map(Column::try_from).collect()
}

pub(crate) fn validate_name(raw: &str) -> Result<String, CoreError> {
    let name = raw.trim();
    let invalid = |reason: &str| CoreError::Validation {
        field: "name",
        reason: reason.to_owned(),
    };
    if name.is_empty() {
        return Err(invalid("El nombre de la columna no puede estar vacío."));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(invalid("El nombre admite como máximo 60 caracteres."));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid(
            "El nombre no puede contener caracteres de control.",
        ));
    }
    Ok(name.to_owned())
}

pub(crate) fn validate_wip(limit: Option<i64>) -> Result<Option<i64>, CoreError> {
    match limit {
        Some(value) if !(1..=MAX_WIP).contains(&value) => Err(CoreError::Validation {
            field: "wipLimit",
            reason: "El límite debe estar entre 1 y 999, o vacío para no limitar.".into(),
        }),
        other => Ok(other),
    }
}

/// Invariante: el proyecto conserva al menos una columna utilizable (no «hecho»),
/// para que siempre haya dónde crear o devolver trabajo. `excluding` simula la operación.
pub(crate) async fn ensure_usable_column_remains(
    conn: &mut SqliteConnection,
    project_id: &str,
    excluding: &str,
) -> Result<(), CoreError> {
    let usable: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM columns WHERE project_id = ? AND id <> ? AND category <> 'done'",
    )
    .bind(project_id)
    .bind(excluding)
    .fetch_one(&mut *conn)
    .await?;
    if usable == 0 {
        return Err(CoreError::Validation {
            field: "category",
            reason: "Debe quedar al menos una columna que no sea de tipo «hecho».".into(),
        });
    }
    Ok(())
}

/// Flujo inicial de un proyecto nuevo: backlog → listo → en curso → revisión → hecho.
pub(crate) async fn seed_default_columns(
    conn: &mut SqliteConnection,
    project_id: &str,
) -> Result<(), CoreError> {
    let defaults = [
        ("Backlog", ColumnCategory::Backlog),
        ("Listo", ColumnCategory::Ready),
        ("En curso", ColumnCategory::Active),
        ("Revisión", ColumnCategory::Review),
        ("Hecho", ColumnCategory::Done),
    ];
    for (position, (name, category)) in (0_i64..).zip(defaults) {
        sqlx::query(
            "INSERT INTO columns (id, project_id, name, category, position) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(project_id)
        .bind(name)
        .bind(category.as_str())
        .bind(position)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_name_and_wip() {
        assert_eq!(validate_name("  En curso ⚙️ ").unwrap(), "En curso ⚙️");
        assert!(validate_name("").is_err());
        assert!(validate_name(&"x".repeat(61)).is_err());
        assert_eq!(validate_wip(None).unwrap(), None);
        assert_eq!(validate_wip(Some(3)).unwrap(), Some(3));
        for bad in [0, -1, 1000] {
            assert!(validate_wip(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn category_round_trips() {
        for category in [
            ColumnCategory::Backlog,
            ColumnCategory::Ready,
            ColumnCategory::Active,
            ColumnCategory::Review,
            ColumnCategory::Done,
        ] {
            assert_eq!(ColumnCategory::parse(category.as_str()).unwrap(), category);
        }
        assert!(ColumnCategory::parse("blocked").is_err());
    }
}
