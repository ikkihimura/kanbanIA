use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use crate::error::CoreError;
use crate::platform::actor::Actor;
use crate::platform::revision;
use crate::platform::storage::Store;

/// Límites de la configuración de ejecución (también los impone la base).
pub const MIN_CONCURRENCY: i64 = 1;
pub const MAX_CONCURRENCY: i64 = 8;
pub const MIN_TIMEOUT_MINUTES: i64 = 1;
pub const MAX_TIMEOUT_MINUTES: i64 = 240;

/// Configuración de ejecución de todo el perfil (sección «Sistema»).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunSettings {
    /// Procesos a la vez (1..8; por defecto 2). Bajarlo no detiene los que ya corren.
    #[ts(type = "number")]
    pub max_concurrency: i64,
    /// Tiempo máximo de un proceso en minutos (1..240; por defecto 30).
    #[ts(type = "number")]
    pub timeout_minutes: i64,
    #[ts(type = "number")]
    pub revision: i64,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateRunSettingsInput {
    #[ts(type = "number")]
    pub expected_revision: i64,
    #[ts(type = "number")]
    pub max_concurrency: i64,
    #[ts(type = "number")]
    pub timeout_minutes: i64,
}

const ENTITY: &str = "Configuración de ejecución";

pub(super) async fn load_settings(conn: &mut SqliteConnection) -> Result<RunSettings, CoreError> {
    let (max_concurrency, timeout_minutes, revision): (i64, i64, i64) = sqlx::query_as(
        "SELECT max_concurrency, timeout_minutes, revision FROM run_settings WHERE id = 1",
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound { entity: ENTITY })?;
    Ok(RunSettings {
        max_concurrency,
        timeout_minutes,
        revision,
    })
}

/// Configuración vigente (lectura). La lee también el supervisor en cada despacho.
pub async fn get_run_settings(store: &Store) -> Result<RunSettings, CoreError> {
    let mut conn = store.reader().acquire().await?;
    load_settings(&mut conn).await
}

/// Cambia la configuración. Solo personas, con `expectedRevision` (revisión obsoleta →
/// `conflict` con la vigente). Fuera de rango → `validation` con el campo. Una transacción
/// `BEGIN IMMEDIATE`; sin `requestId`: aplicar dos veces los mismos valores con la revisión
/// nueva es un `conflict`, nunca un cambio silencioso.
pub async fn update_run_settings(
    store: &Store,
    actor: &Actor,
    input: &UpdateRunSettingsInput,
) -> Result<RunSettings, CoreError> {
    actor.require_human("cambiar la configuración de ejecución")?;
    if !(MIN_CONCURRENCY..=MAX_CONCURRENCY).contains(&input.max_concurrency) {
        return Err(CoreError::Validation {
            field: "maxConcurrency",
            reason: format!(
                "Los procesos a la vez deben estar entre {MIN_CONCURRENCY} y {MAX_CONCURRENCY}."
            ),
        });
    }
    if !(MIN_TIMEOUT_MINUTES..=MAX_TIMEOUT_MINUTES).contains(&input.timeout_minutes) {
        return Err(CoreError::Validation {
            field: "timeoutMinutes",
            reason: format!(
                "El tiempo máximo debe estar entre {MIN_TIMEOUT_MINUTES} y \
                 {MAX_TIMEOUT_MINUTES} minutos."
            ),
        });
    }
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let updated = sqlx::query(
                    "UPDATE run_settings SET max_concurrency = ?, timeout_minutes = ?, \
                     revision = revision + 1 WHERE id = 1 AND revision = ?",
                )
                .bind(input.max_concurrency)
                .bind(input.timeout_minutes)
                .bind(input.expected_revision)
                .execute(&mut *conn)
                .await?
                .rows_affected();
                revision::ensure_updated(ENTITY, updated, async {
                    load_settings(conn).await.map(Some)
                })
                .await?;
                load_settings(conn).await
            })
        })
        .await
}
