//! Errores tipados del núcleo y su forma serializable para los transportes.
//! El mensaje al usuario es fijo por código: nunca incluye detalles internos ni trazas.

use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// `reason` es un texto en español apto para el usuario.
    #[error("{reason}")]
    Validation { field: &'static str, reason: String },

    #[error("{entity} ya existe")]
    AlreadyExists {
        entity: &'static str,
        field: &'static str,
    },

    #[error("{entity} no encontrado")]
    NotFound { entity: &'static str },

    /// La revisión esperada no coincide; incluye el estado vigente para que el cliente decida.
    #[error("conflicto de revisión")]
    Conflict { current: serde_json::Value },

    #[error("request_id reutilizado con otra operación o payload")]
    IdempotencyConflict,

    /// La columna destino alcanzó su límite de trabajo en curso.
    #[error(
        "La columna «{column}» admite como máximo {limit} tarjetas y ya tiene {current}. \
         Mueve o termina alguna antes, o sube el límite."
    )]
    WipLimitExceeded {
        column: String,
        limit: i64,
        current: i64,
    },

    /// Completar trabajo (columna de categoría «hecho») exige aceptación humana explícita.
    #[error("Para mover una tarjeta a «{column}» hace falta aceptarla explícitamente.")]
    AcceptanceRequired { column: String },

    #[error("los datos están abiertos por otra instancia")]
    StorageLocked,

    #[error("la base de datos tiene la migración {0}, desconocida para esta versión")]
    NewerSchema(i64),

    #[error("falló la migración de la base de datos")]
    MigrationFailed(#[source] sqlx::migrate::MigrateError),

    #[error("error de E/S en el almacenamiento")]
    StorageIo(#[source] std::io::Error),

    #[error("error de base de datos")]
    Database(#[from] sqlx::Error),

    #[error("error de serialización")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorCode {
    Validation,
    NotFound,
    AlreadyExists,
    Conflict,
    IdempotencyConflict,
    WipLimitExceeded,
    AcceptanceRequired,
    StorageLocked,
    StorageNewerSchema,
    StorageMigrationFailed,
    StorageUnavailable,
    Internal,
}

/// Error estructurado que cruza la frontera hacia UI, servicio local o MCP.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
    /// El usuario puede resolverlo (cerrar otra instancia, actualizar, reintentar).
    pub recoverable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub request_id: Option<String>,
    /// Campo de entrada al que se refiere el error, para mostrarlo junto al formulario.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub field: Option<String>,
    /// Estado vigente de la entidad en un conflicto de revisión.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub current: Option<serde_json::Value>,
}

impl CoreError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Validation { .. } => ErrorCode::Validation,
            Self::NotFound { .. } => ErrorCode::NotFound,
            Self::AlreadyExists { .. } => ErrorCode::AlreadyExists,
            Self::Conflict { .. } => ErrorCode::Conflict,
            Self::IdempotencyConflict => ErrorCode::IdempotencyConflict,
            Self::WipLimitExceeded { .. } => ErrorCode::WipLimitExceeded,
            Self::AcceptanceRequired { .. } => ErrorCode::AcceptanceRequired,
            Self::StorageLocked => ErrorCode::StorageLocked,
            Self::NewerSchema(_) => ErrorCode::StorageNewerSchema,
            Self::MigrationFailed(_) => ErrorCode::StorageMigrationFailed,
            Self::StorageIo(_) | Self::Database(_) => ErrorCode::StorageUnavailable,
            Self::Serialization(_) => ErrorCode::Internal,
        }
    }

    pub fn to_payload(&self, request_id: Option<&str>) -> ErrorPayload {
        let (message, recoverable) = match self {
            // Describen datos del propio usuario (nombres, límites): es seguro mostrarlos.
            Self::Validation { .. }
            | Self::NotFound { .. }
            | Self::AlreadyExists { .. }
            | Self::WipLimitExceeded { .. }
            | Self::AcceptanceRequired { .. } => (self.to_string(), true),
            Self::Conflict { .. } => (
                "Otra edición cambió estos datos. Revisa el estado actual y vuelve a intentarlo."
                    .into(),
                true,
            ),
            Self::IdempotencyConflict => (
                "Este identificador de petición ya se usó para otra operación.".into(),
                false,
            ),
            Self::StorageLocked => (
                "Kanban IA ya está abierto con estos datos en otra ventana o proceso. \
                 Ciérralo y vuelve a intentarlo."
                    .into(),
                true,
            ),
            Self::NewerSchema(_) => (
                "Los datos fueron creados por una versión más reciente de Kanban IA. \
                 Actualiza la aplicación; no se modificó nada."
                    .into(),
                true,
            ),
            Self::MigrationFailed(_) => (
                "No se pudo actualizar la base de datos. Los datos anteriores se conservaron \
                 sin cambios."
                    .into(),
                true,
            ),
            Self::StorageIo(_) | Self::Database(_) => {
                ("No se pudo acceder al almacenamiento local.".into(), true)
            }
            Self::Serialization(_) => ("Error interno inesperado.".into(), false),
        };

        ErrorPayload {
            code: self.code(),
            message,
            recoverable,
            request_id: request_id.map(str::to_owned),
            field: match self {
                Self::Validation { field, .. } | Self::AlreadyExists { field, .. } => {
                    Some((*field).to_owned())
                }
                _ => None,
            },
            current: match self {
                Self::Conflict { current } => Some(current.clone()),
                _ => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_details_never_reach_the_payload() {
        let error = CoreError::Database(sqlx::Error::Protocol(
            "secret table name at /Users/someone/path".into(),
        ));
        let payload = error.to_payload(Some("req-1"));

        assert_eq!(payload.code, ErrorCode::StorageUnavailable);
        assert!(!payload.message.contains("secret"));
        assert!(!payload.message.contains("/Users"));
        assert_eq!(payload.request_id.as_deref(), Some("req-1"));
    }

    #[test]
    fn conflict_carries_current_state() {
        let error = CoreError::Conflict {
            current: serde_json::json!({ "revision": 3 }),
        };
        let json = serde_json::to_value(error.to_payload(None)).unwrap();

        assert_eq!(json["code"], "conflict");
        assert_eq!(json["current"]["revision"], 3);
        assert!(json.get("requestId").is_none());
    }
}
