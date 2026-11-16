use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::start::AcpAgent;
use crate::error::CoreError;
use crate::platform::actor::Actor;
use crate::platform::revision;
use crate::platform::storage::Store;
use crate::platform::text::has_invisible_formatting;

/// Límites de la configuración de ejecución (también los impone la base).
pub const MIN_CONCURRENCY: i64 = 1;
pub const MAX_CONCURRENCY: i64 = 8;
pub const MIN_TIMEOUT_MINUTES: i64 = 1;
pub const MAX_TIMEOUT_MINUTES: i64 = 240;
/// Longitud máxima (en caracteres) de la ruta del adaptador (también la impone la base).
pub const MAX_ADAPTER_PATH_CHARS: usize = 4096;
/// Límite de la comprobación en disco de la ruta del adaptador: un disco que no responde no
/// bloquea la app.
const ADAPTER_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

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
    /// Ruta absoluta del ejecutable del adaptador de Claude Code (`claude-agent-acp`), tal como
    /// la escribió la persona; `null` = no configurado. Ruta local: nunca para agentes.
    pub claude_acp_path: Option<String>,
    /// Igual, para el adaptador de Codex (`codex-acp`, HU-021).
    pub codex_acp_path: Option<String>,
    /// Notificación del sistema cuando un agente pide permiso (HU-020; por defecto, no). El
    /// texto nunca lleva el comando ni la ruta.
    pub approval_notifications: bool,
    #[ts(type = "number")]
    pub revision: i64,
}

impl RunSettings {
    /// Ruta configurada del adaptador de ese agente.
    pub fn adapter_path(&self, agent: AcpAgent) -> Option<&str> {
        match agent {
            AcpAgent::ClaudeCode => self.claude_acp_path.as_deref(),
            AcpAgent::Codex => self.codex_acp_path.as_deref(),
        }
    }
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
    /// Vacía o `null` → no configurado. Si no, absoluta y de un archivo ejecutable.
    pub claude_acp_path: Option<String>,
    /// Igual, para Codex (HU-021).
    pub codex_acp_path: Option<String>,
    pub approval_notifications: bool,
}

const ENTITY: &str = "Configuración de ejecución";

pub(super) async fn load_settings(conn: &mut SqliteConnection) -> Result<RunSettings, CoreError> {
    #[allow(clippy::type_complexity)]
    let (
        max_concurrency,
        timeout_minutes,
        claude_acp_path,
        codex_acp_path,
        approval_notifications,
        revision,
    ): (i64, i64, Option<String>, Option<String>, bool, i64) = sqlx::query_as(
        "SELECT max_concurrency, timeout_minutes, claude_acp_path, codex_acp_path, \
         approval_notifications, revision FROM run_settings WHERE id = 1",
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound { entity: ENTITY })?;
    Ok(RunSettings {
        max_concurrency,
        timeout_minutes,
        claude_acp_path,
        codex_acp_path,
        approval_notifications,
        revision,
    })
}

/// Configuración vigente (lectura). La lee también el supervisor en cada despacho.
pub async fn get_run_settings(store: &Store) -> Result<RunSettings, CoreError> {
    let mut conn = store.reader().acquire().await?;
    load_settings(&mut conn).await
}

/// Cambia la configuración. Solo personas, con `expectedRevision` (revisión obsoleta →
/// `conflict` con la vigente). Fuera de rango → `validation` con el campo. La ruta del
/// adaptador se comprueba en disco **antes** de la transacción (fuera del runtime, con límite
/// de tiempo): relativa, de red, inexistente, que no sea un archivo normal tras resolver enlaces
/// o (en Unix) sin permiso de ejecución → `validation` (`claudeAcpPath`/`codexAcpPath`); solo si
/// esa ruta cambia
/// respecto a la guardada en `expectedRevision`. Se guarda tal como la
/// escribió la persona (recortada), no la ruta resuelta. Una transacción
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
    // Solo se comprueba en disco una ruta que cambia: guardar otra cosa (p. ej. la concurrencia)
    // no puede fallar porque el adaptador ya guardado se movió. Se compara con la revisión que
    // la persona editaba; si no coincide, la transacción da `conflict`.
    let current = get_run_settings(store).await?;
    let claude_path = checked_path(
        &current,
        input,
        AcpAgent::ClaudeCode,
        input.claude_acp_path.as_deref(),
    )
    .await?;
    let codex_path = checked_path(
        &current,
        input,
        AcpAgent::Codex,
        input.codex_acp_path.as_deref(),
    )
    .await?;
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let updated = sqlx::query(
                    "UPDATE run_settings SET max_concurrency = ?, timeout_minutes = ?, \
                     claude_acp_path = ?, codex_acp_path = ?, approval_notifications = ?, \
                     revision = revision + 1 WHERE id = 1 AND revision = ?",
                )
                .bind(input.max_concurrency)
                .bind(input.timeout_minutes)
                .bind(claude_path)
                .bind(codex_path)
                .bind(input.approval_notifications)
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

/// Ruta del adaptador de `agent` tal como se guardará: vacía → `None`; si cambia respecto a la
/// revisión que se editaba, comprobada en texto y en disco.
async fn checked_path(
    current: &RunSettings,
    input: &UpdateRunSettingsInput,
    agent: AcpAgent,
    typed: Option<&str>,
) -> Result<Option<String>, CoreError> {
    match typed.map(str::trim) {
        None | Some("") => Ok(None),
        Some(path) => {
            let unchanged = current.revision == input.expected_revision
                && current.adapter_path(agent) == Some(path);
            if !unchanged {
                validate_adapter_path_text(agent, path)?;
                check_adapter_on_disk(agent, path.to_owned()).await?;
            }
            Ok(Some(path.to_owned()))
        }
    }
}

/// Valida una ruta de adaptador como al guardarla (texto y disco, fuera del runtime y con
/// límite de tiempo) y la devuelve recortada. La usa «Comprobar adaptador» antes de lanzarlo.
/// Errores → `validation` (`claudeAcpPath`/`codexAcpPath`) con un mensaje sin la ruta.
pub async fn check_adapter_path(
    agent: AcpAgent,
    path: &str,
) -> Result<std::path::PathBuf, CoreError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(invalid_path(agent, "Indica la ruta del adaptador."));
    }
    validate_adapter_path_text(agent, path)?;
    check_adapter_on_disk(agent, path.to_owned()).await?;
    Ok(std::path::PathBuf::from(path))
}

fn invalid_path(agent: AcpAgent, reason: impl Into<String>) -> CoreError {
    CoreError::Validation {
        field: agent.settings_field(),
        reason: reason.into(),
    }
}

/// Comprobaciones sin tocar el disco: longitud, caracteres, absoluta, normalizada y local.
fn validate_adapter_path_text(agent: AcpAgent, path: &str) -> Result<(), CoreError> {
    if path.chars().count() > MAX_ADAPTER_PATH_CHARS {
        return Err(invalid_path(
            agent,
            format!("La ruta del adaptador admite hasta {MAX_ADAPTER_PATH_CHARS} caracteres."),
        ));
    }
    if path.chars().any(char::is_control) || has_invisible_formatting(path) {
        return Err(invalid_path(
            agent,
            "La ruta del adaptador contiene caracteres no válidos.",
        ));
    }
    if !Path::new(path).is_absolute() {
        return Err(invalid_path(
            agent,
            "Escribe la ruta absoluta del ejecutable del adaptador (p. ej. la que muestra \
             `which claude-agent-acp` o `which codex-acp`).",
        ));
    }
    // `.`/`..` permitirían esconder una ruta de red tras un prefijo local.
    if path
        .split(['/', '\\'])
        .any(|part| part == "." || part == "..")
    {
        return Err(invalid_path(
            agent,
            "La ruta del adaptador contiene «.» o «..»: escribe la ruta normalizada.",
        ));
    }
    // Comprobar rutas de red (UNC, `//host`, autofs `/net/`) abriría conexiones salientes o
    // colgaría la app; además, el adaptador se ejecuta: debe ser local.
    let lower = path.to_ascii_lowercase();
    if path.starts_with("\\\\") || path.starts_with("//") || lower.starts_with("/net/") {
        return Err(invalid_path(
            agent,
            "La ruta del adaptador es una ruta de red: usa un ejecutable local.",
        ));
    }
    Ok(())
}

/// Resultado de la comprobación en disco (sin el detalle del sistema: no se muestra).
enum AdapterCheck {
    Ok,
    Missing,
    NotFile,
    NotExecutable,
}

/// Comprueba en disco, fuera del runtime asíncrono y con límite de tiempo, que la ruta lleva a
/// un archivo normal (tras resolver enlaces simbólicos: la instalación npm usa un enlace en
/// `.bin`) y, en Unix, con permiso de ejecución. En Windows basta con que sea un archivo.
async fn check_adapter_on_disk(agent: AcpAgent, path: String) -> Result<(), CoreError> {
    let task = tokio::task::spawn_blocking(move || {
        let Ok(resolved) = std::fs::canonicalize(&path) else {
            return AdapterCheck::Missing;
        };
        let Ok(metadata) = std::fs::metadata(&resolved) else {
            return AdapterCheck::Missing;
        };
        if !metadata.is_file() {
            return AdapterCheck::NotFile;
        }
        if is_executable(&metadata) {
            AdapterCheck::Ok
        } else {
            AdapterCheck::NotExecutable
        }
    });
    match tokio::time::timeout(ADAPTER_CHECK_TIMEOUT, task).await {
        Ok(Ok(AdapterCheck::Ok)) => Ok(()),
        Ok(Ok(AdapterCheck::Missing)) => Err(invalid_path(
            agent,
            "No existe ningún archivo en esa ruta (o no se puede leer).",
        )),
        Ok(Ok(AdapterCheck::NotFile)) => Err(invalid_path(
            agent,
            "Esa ruta no es un archivo: indica el ejecutable del adaptador, no su carpeta.",
        )),
        Ok(Ok(AdapterCheck::NotExecutable)) => Err(invalid_path(
            agent,
            "Ese archivo no tiene permiso de ejecución.",
        )),
        Ok(Err(_)) | Err(_) => Err(invalid_path(
            agent,
            "No se pudo comprobar la ruta a tiempo: comprueba que el disco está disponible.",
        )),
    }
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    true
}
