use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use crate::error::CoreError;

pub(super) const ENTITY: &str = "Historia";
const MAX_TITLE_CHARS: usize = 200;
pub(super) const MAX_DESCRIPTION_CHARS: usize = 20_000;
pub(super) const MAX_CRITERIA: usize = 50;
const MAX_CRITERION_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Priority {
    Urgent,
    High,
    Medium,
    Low,
}

impl Priority {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Urgent => "urgent",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    fn parse(value: &str) -> Result<Self, CoreError> {
        Ok(match value {
            "urgent" => Self::Urgent,
            "high" => Self::High,
            "medium" => Self::Medium,
            "low" => Self::Low,
            other => {
                return Err(CoreError::Database(sqlx::Error::Decode(
                    format!("prioridad desconocida: {other}").into(),
                )));
            }
        })
    }
}

/// Historia en listas y tablero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StorySummary {
    pub id: String,
    pub project_id: String,
    #[ts(type = "number")]
    pub number: i64,
    pub column_id: String,
    /// `null` si está archivada (fuera del orden del tablero).
    #[ts(type = "number | null")]
    pub position: Option<i64>,
    pub title: String,
    pub priority: Priority,
    pub archived_at: Option<String>,
    /// Aceptación vigente (la establece HU-009).
    pub accepted_at: Option<String>,
    pub epic_id: Option<String>,
    pub parent_id: Option<String>,
    /// Agente asignado (solo asignación; no implica actividad).
    pub agent_id: Option<String>,
    /// Prerequisitos sin aceptar: bloqueada si > 0.
    #[ts(type = "number")]
    pub pending_prerequisites: i64,
    #[ts(type = "number")]
    pub criteria_count: i64,
    #[ts(type = "number")]
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcceptanceCriterion {
    pub id: String,
    /// Se muestra como CA-n; no cambia aunque se borren otros criterios.
    #[ts(type = "number")]
    pub number: i64,
    pub text: String,
}

/// Historia completa para el editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoryDetail {
    pub id: String,
    pub project_id: String,
    #[ts(type = "number")]
    pub number: i64,
    pub column_id: String,
    #[ts(type = "number | null")]
    pub position: Option<i64>,
    pub title: String,
    /// Markdown tal como lo escribió el usuario; la UI lo renderiza sin HTML crudo.
    pub description: String,
    pub priority: Priority,
    pub archived_at: Option<String>,
    pub accepted_at: Option<String>,
    pub criteria: Vec<AcceptanceCriterion>,
    #[ts(type = "number")]
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(sqlx::FromRow)]
struct StoryRow {
    id: String,
    project_id: String,
    number: i64,
    column_id: String,
    position: Option<i64>,
    title: String,
    description: String,
    priority: String,
    archived_at: Option<String>,
    accepted_at: Option<String>,
    epic_id: Option<String>,
    parent_id: Option<String>,
    agent_id: Option<String>,
    pending_prerequisites: i64,
    criteria_count: i64,
    revision: i64,
    created_at: String,
    updated_at: String,
}

macro_rules! select_story {
    ($tail:literal) => {
        concat!(
            "SELECT s.id, s.project_id, s.number, s.column_id, s.position, s.title, \
             s.description, s.priority, s.archived_at, s.accepted_at, s.revision, \
             s.created_at, s.updated_at, \
             (SELECT COUNT(*) FROM acceptance_criteria a WHERE a.story_id = s.id) \
                 AS criteria_count, \
             (SELECT epic_id FROM story_epics se WHERE se.story_id = s.id) AS epic_id, \
             (SELECT parent_id FROM story_parents sp WHERE sp.story_id = s.id) AS parent_id, \
             (SELECT agent_id FROM story_agents sg WHERE sg.story_id = s.id) AS agent_id, \
             (SELECT COUNT(*) FROM story_dependencies d JOIN stories p ON p.id = d.depends_on_id \
              WHERE d.story_id = s.id AND p.accepted_at IS NULL) AS pending_prerequisites \
             FROM stories s ",
            $tail
        )
    };
}

impl StoryRow {
    fn into_summary(self) -> Result<StorySummary, CoreError> {
        Ok(StorySummary {
            priority: Priority::parse(&self.priority)?,
            id: self.id,
            project_id: self.project_id,
            number: self.number,
            column_id: self.column_id,
            position: self.position,
            title: self.title,
            archived_at: self.archived_at,
            accepted_at: self.accepted_at,
            epic_id: self.epic_id,
            parent_id: self.parent_id,
            agent_id: self.agent_id,
            pending_prerequisites: self.pending_prerequisites,
            criteria_count: self.criteria_count,
            revision: self.revision,
        })
    }

    fn into_detail(self, criteria: Vec<AcceptanceCriterion>) -> Result<StoryDetail, CoreError> {
        Ok(StoryDetail {
            priority: Priority::parse(&self.priority)?,
            id: self.id,
            project_id: self.project_id,
            number: self.number,
            column_id: self.column_id,
            position: self.position,
            title: self.title,
            description: self.description,
            archived_at: self.archived_at,
            accepted_at: self.accepted_at,
            criteria,
            revision: self.revision,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

async fn load_row(
    conn: &mut SqliteConnection,
    project_id: &str,
    id: &str,
) -> Result<Option<StoryRow>, CoreError> {
    Ok(
        sqlx::query_as(select_story!("WHERE s.project_id = ? AND s.id = ?"))
            .bind(project_id)
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

pub(crate) async fn load_story(
    conn: &mut SqliteConnection,
    project_id: &str,
    id: &str,
) -> Result<Option<StorySummary>, CoreError> {
    load_row(conn, project_id, id)
        .await?
        .map(StoryRow::into_summary)
        .transpose()
}

pub(crate) async fn load_detail(
    conn: &mut SqliteConnection,
    project_id: &str,
    id: &str,
) -> Result<Option<StoryDetail>, CoreError> {
    let Some(row) = load_row(conn, project_id, id).await? else {
        return Ok(None);
    };
    let criteria = sqlx::query_as::<_, (String, i64, String)>(
        "SELECT id, number, text FROM acceptance_criteria WHERE story_id = ? ORDER BY position",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|(id, number, text)| AcceptanceCriterion { id, number, text })
    .collect();
    row.into_detail(criteria).map(Some)
}

pub(super) async fn list_rows(
    conn: &mut SqliteConnection,
    project_id: &str,
    include_archived: bool,
) -> Result<Vec<StorySummary>, CoreError> {
    let rows: Vec<StoryRow> = sqlx::query_as(select_story!(
        "WHERE s.project_id = ? AND (? OR s.archived_at IS NULL) \
         ORDER BY s.archived_at IS NOT NULL, \
                  (SELECT position FROM columns c WHERE c.id = s.column_id), \
                  s.position, s.number"
    ))
    .bind(project_id)
    .bind(include_archived)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter().map(StoryRow::into_summary).collect()
}

/// Tarjetas activas (no archivadas) de una columna: lo que ocupa posiciones y cuenta para WIP.
pub(super) async fn count_in_column(
    conn: &mut SqliteConnection,
    column_id: &str,
) -> Result<i64, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM stories WHERE column_id = ? AND archived_at IS NULL",
    )
    .bind(column_id)
    .fetch_one(&mut *conn)
    .await?)
}

fn invalid(field: &'static str, reason: impl Into<String>) -> CoreError {
    CoreError::Validation {
        field,
        reason: reason.into(),
    }
}

pub(super) fn validate_title(raw: &str) -> Result<String, CoreError> {
    let title = raw.trim();
    if title.is_empty() {
        return Err(invalid("title", "El título es obligatorio."));
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(invalid(
            "title",
            "El título admite como máximo 200 caracteres.",
        ));
    }
    if title.chars().any(char::is_control) {
        return Err(invalid(
            "title",
            "El título no puede contener caracteres de control.",
        ));
    }
    Ok(title.to_owned())
}

/// Markdown libre (saltos de línea y tabuladores permitidos), con límite de tamaño.
/// Se guarda tal cual salvo los espacios finales.
pub(super) fn validate_description(raw: &str) -> Result<String, CoreError> {
    if raw.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(invalid(
            "description",
            "La descripción admite como máximo 20 000 caracteres.",
        ));
    }
    if raw
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid(
            "description",
            "La descripción contiene caracteres de control no válidos.",
        ));
    }
    Ok(raw.trim_end().to_owned())
}

pub(super) fn validate_criterion(raw: &str) -> Result<String, CoreError> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(invalid("criteria", "Un criterio no puede estar vacío."));
    }
    if text.chars().count() > MAX_CRITERION_CHARS {
        return Err(invalid(
            "criteria",
            "Cada criterio admite como máximo 500 caracteres.",
        ));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid(
            "criteria",
            "Los criterios no pueden contener saltos de línea ni caracteres de control.",
        ));
    }
    Ok(text.to_owned())
}

pub(super) fn not_archived(story: &StorySummary) -> Result<(), CoreError> {
    if story.archived_at.is_some() {
        return Err(invalid(
            "id",
            format!(
                "La historia #{} está archivada; restáurala antes.",
                story.number
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_keeps_markdown_and_limits_size() {
        let md = "# Título\n\n- [ ] tarea\n\tcódigo\n\n";
        assert_eq!(
            validate_description(md).unwrap(),
            "# Título\n\n- [ ] tarea\n\tcódigo"
        );
        assert!(validate_description(&"a".repeat(20_000)).is_ok());
        assert!(validate_description(&"a".repeat(20_001)).is_err());
        assert!(validate_description("nulo\u{0}").is_err());
    }

    #[test]
    fn criteria_and_title_are_validated() {
        assert_eq!(
            validate_criterion("  El usuario ve X ").unwrap(),
            "El usuario ve X"
        );
        for bad in ["", "  ", "dos\nlíneas", &"x".repeat(501)] {
            assert!(validate_criterion(bad).is_err(), "{bad:?}");
        }
        assert!(validate_title("").is_err());
        assert!(validate_title(&"t".repeat(201)).is_err());
    }

    #[test]
    fn priority_round_trips() {
        for p in [
            Priority::Urgent,
            Priority::High,
            Priority::Medium,
            Priority::Low,
        ] {
            assert_eq!(Priority::parse(p.as_str()).unwrap(), p);
        }
    }
}
