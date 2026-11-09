use serde::Serialize;
use ts_rs::TS;

use super::Store;
use crate::error::{CoreError, ErrorPayload};

/// Estado del almacenamiento local visible para la UI.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "status", rename_all = "camelCase")]
#[ts(export)]
pub enum StorageStatus {
    Ready {
        #[serde(rename = "schemaVersion")]
        #[ts(rename = "schemaVersion")]
        schema_version: u32,
        #[serde(rename = "databasePath")]
        #[ts(rename = "databasePath")]
        database_path: String,
    },
    Unavailable {
        error: ErrorPayload,
    },
}

impl StorageStatus {
    pub async fn of(store: &Store) -> Self {
        match store.schema_version().await {
            Ok(version) => Self::Ready {
                schema_version: u32::try_from(version).unwrap_or(u32::MAX),
                database_path: store.database_path().display().to_string(),
            },
            Err(error) => Self::from_error(&error),
        }
    }

    pub fn from_error(error: &CoreError) -> Self {
        Self::Unavailable {
            error: error.to_payload(None),
        }
    }
}
