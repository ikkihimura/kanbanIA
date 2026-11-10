use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use crate::error::CoreError;

pub(super) const MAX_REFERENCES: usize = 10;
const MAX_SUMMARY_CHARS: usize = 2_000;
const MAX_REFERENCE_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DeliverySource {
    /// Registrada por la persona en la app.
    Manual,
    /// Reportada por un agente (MCP, M2). Es lo que el agente afirma, no algo observado.
    Reported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DeliveryStatus {
    /// En espera de revisión humana.
    Pending,
    /// La historia se aceptó con esta entrega pendiente.
    Accepted,
    /// Devuelta con un motivo.
    Returned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ReferenceKind {
    /// http(s). Nunca se visita: se muestra como no verificada.
    Url,
    /// Ruta local absoluta. Se comprueba al mostrarla.
    Path,
}

/// Accesibilidad de una referencia, calculada al leer (no se guarda).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ReferenceStatus {
    Accessible,
    Missing,
    Inaccessible,
    /// URL: la app no hace peticiones de red para comprobarla.
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReferenceInput {
    pub kind: ReferenceKind,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeliveryReference {
    pub kind: ReferenceKind,
    pub value: String,
    pub status: ReferenceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Delivery {
    pub id: String,
    pub story_id: String,
    #[ts(type = "number")]
    pub number: i64,
    pub source: DeliverySource,
    /// Agente que la reportó (solo `reported`).
    pub agent_id: Option<String>,
    pub agent_name: Option<String>,
    pub summary: String,
    pub references: Vec<DeliveryReference>,
    pub status: DeliveryStatus,
    pub return_reason: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

pub(super) fn validate_summary(raw: &str) -> Result<String, CoreError> {
    let summary = raw.trim();
    let invalid = |reason: &str| CoreError::Validation {
        field: "summary",
        reason: reason.to_owned(),
    };
    if summary.is_empty() {
        return Err(invalid("El resumen de la entrega es obligatorio."));
    }
    if summary.chars().count() > MAX_SUMMARY_CHARS {
        return Err(invalid("El resumen admite como máximo 2 000 caracteres."));
    }
    if summary
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        || super::has_invisible_formatting(summary)
    {
        return Err(invalid(
            "El resumen contiene caracteres de control no válidos.",
        ));
    }
    Ok(summary.to_owned())
}

pub(super) fn validate_references(
    refs: &[ReferenceInput],
) -> Result<Vec<ReferenceInput>, CoreError> {
    let invalid = |reason: String| CoreError::Validation {
        field: "references",
        reason,
    };
    if refs.len() > MAX_REFERENCES {
        return Err(invalid(
            "Una entrega admite como máximo 10 referencias.".into(),
        ));
    }
    refs.iter()
        .map(|reference| {
            let value = reference.value.trim();
            if value.is_empty() || value.chars().count() > MAX_REFERENCE_CHARS {
                return Err(invalid(
                    "Cada referencia debe tener entre 1 y 2 000 caracteres.".into(),
                ));
            }
            if value.chars().any(char::is_control) {
                return Err(invalid(
                    "Las referencias no pueden contener caracteres de control.".into(),
                ));
            }
            match reference.kind {
                ReferenceKind::Url => {
                    let lower = value.to_ascii_lowercase();
                    let has_host = value
                        .split_once("://")
                        .is_some_and(|(_, rest)| !rest.is_empty() && !rest.starts_with('/'));
                    if !(lower.starts_with("http://") || lower.starts_with("https://")) || !has_host
                    {
                        return Err(invalid(format!("«{value}» no es una URL http(s) válida.")));
                    }
                }
                ReferenceKind::Path => {
                    if !Path::new(value).is_absolute() {
                        return Err(invalid(format!("«{value}» no es una ruta absoluta.")));
                    }
                    // Comprobar rutas de red (UNC, `//host`, autofs `/net/`) abriría conexiones
                    // salientes (p. ej. SMB con credenciales de Windows) o colgaría la app.
                    let lower = value.to_ascii_lowercase();
                    if value.starts_with("\\\\")
                        || value.starts_with("//")
                        || lower.starts_with("/net/")
                    {
                        return Err(invalid(format!(
                            "«{value}» es una ruta de red: usa una ruta local o una URL."
                        )));
                    }
                }
            }
            Ok(ReferenceInput {
                kind: reference.kind,
                value: value.to_owned(),
            })
        })
        .collect()
}

/// Estado actual de una referencia. Las URL nunca se consultan por red.
fn reference_status(kind: ReferenceKind, value: &str) -> ReferenceStatus {
    match kind {
        ReferenceKind::Url => ReferenceStatus::Unverified,
        ReferenceKind::Path => match std::fs::metadata(value) {
            Ok(_) => ReferenceStatus::Accessible,
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
            {
                ReferenceStatus::Missing
            }
            Err(_) => ReferenceStatus::Inaccessible,
        },
    }
}

impl DeliverySource {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Reported => "reported",
        }
    }
}

impl ReferenceKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Url => "url",
            Self::Path => "path",
        }
    }

    fn parse(value: &str) -> Result<Self, CoreError> {
        match value {
            "url" => Ok(Self::Url),
            "path" => Ok(Self::Path),
            other => Err(decode_error("tipo de referencia", other)),
        }
    }
}

fn decode_error(what: &str, value: &str) -> CoreError {
    CoreError::Database(sqlx::Error::Decode(
        format!("{what} desconocido: {value}").into(),
    ))
}

#[derive(sqlx::FromRow)]
struct DeliveryRow {
    id: String,
    story_id: String,
    number: i64,
    source: String,
    agent_id: Option<String>,
    agent_name: Option<String>,
    summary: String,
    status: String,
    return_reason: Option<String>,
    created_at: String,
    decided_at: Option<String>,
}

/// Entregas de una historia (más reciente primero), sin estado de referencias aún.
pub(super) async fn deliveries_of_story(
    conn: &mut SqliteConnection,
    project_id: &str,
    story_id: &str,
) -> Result<Vec<Delivery>, CoreError> {
    let rows: Vec<DeliveryRow> = sqlx::query_as(
        "SELECT d.id, d.story_id, d.number, d.source, d.agent_id, a.name AS agent_name, \
         d.summary, d.status, d.return_reason, d.created_at, d.decided_at \
         FROM deliveries d LEFT JOIN agents a ON a.id = d.agent_id \
         WHERE d.project_id = ? AND d.story_id = ? ORDER BY d.number DESC",
    )
    .bind(project_id)
    .bind(story_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut deliveries = Vec::with_capacity(rows.len());
    for row in rows {
        let refs: Vec<(String, String)> = sqlx::query_as(
            "SELECT kind, value FROM delivery_references WHERE delivery_id = ? ORDER BY position",
        )
        .bind(&row.id)
        .fetch_all(&mut *conn)
        .await?;
        let references = refs
            .into_iter()
            .map(|(kind, value)| {
                let kind = ReferenceKind::parse(&kind)?;
                Ok(DeliveryReference {
                    kind,
                    status: ReferenceStatus::Unverified,
                    value,
                })
            })
            .collect::<Result<Vec<_>, CoreError>>()?;
        deliveries.push(Delivery {
            source: match row.source.as_str() {
                "manual" => DeliverySource::Manual,
                "reported" => DeliverySource::Reported,
                other => return Err(decode_error("origen", other)),
            },
            status: match row.status.as_str() {
                "pending" => DeliveryStatus::Pending,
                "accepted" => DeliveryStatus::Accepted,
                "returned" => DeliveryStatus::Returned,
                other => return Err(decode_error("estado", other)),
            },
            id: row.id,
            story_id: row.story_id,
            number: row.number,
            agent_id: row.agent_id,
            agent_name: row.agent_name,
            summary: row.summary,
            references,
            return_reason: row.return_reason,
            created_at: row.created_at,
            decided_at: row.decided_at,
        });
    }
    Ok(deliveries)
}

const STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Calcula la accesibilidad de las referencias fuera del runtime asíncrono y con límite de
/// tiempo: si el disco no responde, quedan como `Unverified` en vez de bloquear la app.
pub(super) async fn resolve_reference_status(deliveries: &mut [Delivery]) {
    let targets: Vec<(ReferenceKind, String)> = deliveries
        .iter()
        .flat_map(|d| d.references.iter().map(|r| (r.kind, r.value.clone())))
        .collect();
    if targets.is_empty() {
        return;
    }
    let task = tokio::task::spawn_blocking(move || {
        targets
            .iter()
            .map(|(kind, value)| reference_status(*kind, value))
            .collect::<Vec<_>>()
    });
    let Ok(Ok(statuses)) = tokio::time::timeout(STATUS_TIMEOUT, task).await else {
        return;
    };
    let mut statuses = statuses.into_iter();
    for delivery in deliveries {
        for reference in &mut delivery.references {
            if let Some(status) = statuses.next() {
                reference.status = status;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(kind: ReferenceKind, value: &str) -> ReferenceInput {
        ReferenceInput {
            kind,
            value: value.into(),
        }
    }

    #[test]
    fn only_http_urls_and_absolute_paths_are_accepted() {
        let ok = validate_references(&[
            r(ReferenceKind::Url, " https://github.com/o/r/pull/1 "),
            r(ReferenceKind::Url, "http://localhost:3000/demo"),
            r(ReferenceKind::Path, "/tmp/informe.pdf"),
        ])
        .unwrap();
        assert_eq!(ok[0].value, "https://github.com/o/r/pull/1");

        for bad in [
            r(ReferenceKind::Url, "javascript:alert(1)"),
            r(ReferenceKind::Url, "file:///etc/passwd"),
            r(ReferenceKind::Url, "https://"),
            r(ReferenceKind::Url, "ftp://x"),
            r(ReferenceKind::Path, "relativa/archivo"),
            r(ReferenceKind::Path, "//servidor/compartido/x"),
            r(ReferenceKind::Path, "/net/servidor/x"),
            r(ReferenceKind::Path, ""),
        ] {
            assert!(
                validate_references(std::slice::from_ref(&bad)).is_err(),
                "{bad:?}"
            );
        }
        let many: Vec<_> = (0..11).map(|_| r(ReferenceKind::Path, "/tmp/x")).collect();
        assert!(validate_references(&many).is_err());
    }

    #[test]
    fn path_status_reflects_disk_and_urls_are_never_checked() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("entrega.txt");
        std::fs::write(&file, "ok").unwrap();
        assert_eq!(
            reference_status(ReferenceKind::Path, file.to_str().unwrap()),
            ReferenceStatus::Accessible
        );
        assert_eq!(
            reference_status(ReferenceKind::Path, dir.path().join("no").to_str().unwrap()),
            ReferenceStatus::Missing
        );
        assert_eq!(
            reference_status(ReferenceKind::Url, "https://example.invalid"),
            ReferenceStatus::Unverified
        );
    }
}
