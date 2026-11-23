use std::path::{Path, PathBuf};
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
/// Antigüedad (días) a partir de la cual un intento terminado se propone para limpiar (HU-025).
pub const MIN_RETENTION_DAYS: i64 = 1;
pub const MAX_RETENTION_DAYS: i64 = 3650;
/// Longitud máxima (en caracteres) de la ruta del adaptador o de `node` (también la impone la
/// base).
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
    /// Ruta absoluta del ejecutable `node` (HU-026), tal como la escribió la persona; su carpeta
    /// se antepone al `PATH` de los adaptadores (los npm son scripts `#!/usr/bin/env node`).
    /// `null` = el `PATH` con el que se abrió la app. Ruta local: nunca para agentes.
    pub node_path: Option<String>,
    /// Notificación del sistema cuando un agente pide permiso (HU-020; por defecto, no). El
    /// texto nunca lleva el comando ni la ruta.
    pub approval_notifications: bool,
    /// Antigüedad en días a partir de la cual la limpieza propone un intento terminado
    /// (1..3650; por defecto 30). Nunca borra nada sola (HU-025).
    #[ts(type = "number")]
    pub retention_days: i64,
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
    /// Vacía o `null` → sin ruta de node. Si no, absoluta, de un archivo ejecutable llamado
    /// `node` (`node.exe` en Windows) (HU-026).
    pub node_path: Option<String>,
    pub approval_notifications: bool,
    /// Días de retención (HU-025). Ausente o `null` → se conserva el valor vigente.
    #[serde(default)]
    #[ts(type = "number | null", optional)]
    pub retention_days: Option<i64>,
}

const ENTITY: &str = "Configuración de ejecución";

#[derive(sqlx::FromRow)]
struct SettingsRow {
    max_concurrency: i64,
    timeout_minutes: i64,
    claude_acp_path: Option<String>,
    codex_acp_path: Option<String>,
    node_path: Option<String>,
    approval_notifications: bool,
    retention_days: i64,
    revision: i64,
}

pub(super) async fn load_settings(conn: &mut SqliteConnection) -> Result<RunSettings, CoreError> {
    let row: SettingsRow = sqlx::query_as(
        "SELECT max_concurrency, timeout_minutes, claude_acp_path, codex_acp_path, node_path, \
         approval_notifications, retention_days, revision FROM run_settings WHERE id = 1",
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound { entity: ENTITY })?;
    Ok(RunSettings {
        max_concurrency: row.max_concurrency,
        timeout_minutes: row.timeout_minutes,
        claude_acp_path: row.claude_acp_path,
        codex_acp_path: row.codex_acp_path,
        node_path: row.node_path,
        approval_notifications: row.approval_notifications,
        retention_days: row.retention_days,
        revision: row.revision,
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
/// o (en Unix) sin permiso de ejecución → `validation` (`claudeAcpPath`/`codexAcpPath`); igual
/// para `nodePath`, que además debe llamarse `node` (HU-026). Solo si esa ruta cambia respecto
/// a la guardada en `expectedRevision`. Se guarda tal como la
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
    if let Some(days) = input.retention_days
        && !(MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS).contains(&days)
    {
        return Err(CoreError::Validation {
            field: "retentionDays",
            reason: format!(
                "La antigüedad para limpiar debe estar entre {MIN_RETENTION_DAYS} y \
                 {MAX_RETENTION_DAYS} días."
            ),
        });
    }
    // Solo se comprueba en disco una ruta que cambia: guardar otra cosa (p. ej. la concurrencia)
    // no puede fallar porque el adaptador ya guardado se movió. Se compara con la revisión que
    // la persona editaba; si no coincide, la transacción da `conflict`.
    let current = get_run_settings(store).await?;
    let data_dir = store.data_dir().map(Path::to_path_buf);
    let claude_path = checked_path(
        &current,
        input,
        None,
        PathTarget::Adapter(AcpAgent::ClaudeCode),
        input.claude_acp_path.as_deref(),
    )
    .await?;
    let codex_path = checked_path(
        &current,
        input,
        None,
        PathTarget::Adapter(AcpAgent::Codex),
        input.codex_acp_path.as_deref(),
    )
    .await?;
    let node_path = checked_path(
        &current,
        input,
        data_dir,
        PathTarget::Node,
        input.node_path.as_deref(),
    )
    .await?;
    let input = input.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let updated = sqlx::query(
                    "UPDATE run_settings SET max_concurrency = ?, timeout_minutes = ?, \
                     claude_acp_path = ?, codex_acp_path = ?, node_path = ?, \
                     approval_notifications = ?, \
                     retention_days = coalesce(?, retention_days), revision = revision + 1 \
                     WHERE id = 1 AND revision = ?",
                )
                .bind(input.max_concurrency)
                .bind(input.timeout_minutes)
                .bind(claude_path)
                .bind(codex_path)
                .bind(node_path)
                .bind(input.approval_notifications)
                .bind(input.retention_days)
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

/// Qué ruta se valida: la de un adaptador o la de `node` (HU-026). Mismas reglas de texto y
/// disco; cambian el campo del error, los mensajes y, para `node`, el nombre del archivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathTarget {
    Adapter(AcpAgent),
    Node,
}

impl PathTarget {
    fn field(self) -> &'static str {
        match self {
            Self::Adapter(agent) => agent.settings_field(),
            Self::Node => "nodePath",
        }
    }

    /// «del adaptador» / «de node», para los mensajes.
    fn of(self) -> &'static str {
        match self {
            Self::Adapter(_) => "del adaptador",
            Self::Node => "de node",
        }
    }

    fn saved(self, settings: &RunSettings) -> Option<&str> {
        match self {
            Self::Adapter(agent) => settings.adapter_path(agent),
            Self::Node => settings.node_path.as_deref(),
        }
    }
}

/// Ruta tal como se guardará: vacía → `None`; si cambia respecto a la revisión que se editaba,
/// comprobada en texto y en disco.
async fn checked_path(
    current: &RunSettings,
    input: &UpdateRunSettingsInput,
    data_dir: Option<PathBuf>,
    target: PathTarget,
    typed: Option<&str>,
) -> Result<Option<String>, CoreError> {
    match typed.map(str::trim) {
        None | Some("") => Ok(None),
        Some(path) => {
            let unchanged =
                current.revision == input.expected_revision && target.saved(current) == Some(path);
            if !unchanged {
                validate_path_text(target, path)?;
                check_on_disk(target, path.to_owned(), data_dir).await?;
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
    let target = PathTarget::Adapter(agent);
    let path = path.trim();
    if path.is_empty() {
        return Err(invalid_path(target, "Indica la ruta del adaptador."));
    }
    validate_path_text(target, path)?;
    check_on_disk(target, path.to_owned(), None).await?;
    Ok(std::path::PathBuf::from(path))
}

/// Igual para la ruta de `node` (HU-026): vacía → `None` (sin ruta de node). Errores →
/// `validation` en `nodePath`; también si queda dentro de `data_dir` (los worktrees los
/// escriben los agentes). La usa «Comprobar adaptador» con la ruta escrita o la guardada.
pub async fn check_node_path(
    path: &str,
    data_dir: Option<&Path>,
) -> Result<Option<std::path::PathBuf>, CoreError> {
    let path = path.trim();
    if path.is_empty() {
        return Ok(None);
    }
    validate_path_text(PathTarget::Node, path)?;
    check_on_disk(
        PathTarget::Node,
        path.to_owned(),
        data_dir.map(Path::to_path_buf),
    )
    .await?;
    Ok(Some(std::path::PathBuf::from(path)))
}

fn invalid_path(target: PathTarget, reason: impl Into<String>) -> CoreError {
    CoreError::Validation {
        field: target.field(),
        reason: reason.into(),
    }
}

/// Nombre que debe tener el ejecutable de `node`: su carpeta se antepone al `PATH` y
/// `#!/usr/bin/env node` busca ese nombre exacto.
#[cfg(not(windows))]
fn is_node_file_name(name: &str) -> bool {
    name == "node"
}

#[cfg(windows)]
fn is_node_file_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("node.exe")
}

/// `true` si `path` pasaría la validación de texto de una ruta de node (para no proponer en
/// «Buscar node» rutas que después no se podrían guardar).
pub(super) fn is_valid_node_path_text(path: &str) -> bool {
    validate_path_text(PathTarget::Node, path).is_ok()
}

/// Comprobaciones sin tocar el disco: longitud, caracteres, absoluta, normalizada y local.
fn validate_path_text(target: PathTarget, path: &str) -> Result<(), CoreError> {
    let of = target.of();
    if path.chars().count() > MAX_ADAPTER_PATH_CHARS {
        return Err(invalid_path(
            target,
            format!("La ruta {of} admite hasta {MAX_ADAPTER_PATH_CHARS} caracteres."),
        ));
    }
    if path.chars().any(char::is_control) || has_invisible_formatting(path) {
        return Err(invalid_path(
            target,
            format!("La ruta {of} contiene caracteres no válidos."),
        ));
    }
    if !Path::new(path).is_absolute() {
        return Err(invalid_path(
            target,
            match target {
                PathTarget::Adapter(_) => {
                    "Escribe la ruta absoluta del ejecutable del adaptador (p. ej. la que muestra \
                     `which claude-agent-acp` o `which codex-acp`)."
                }
                PathTarget::Node => {
                    "Escribe la ruta absoluta del ejecutable de node (p. ej. la que muestra \
                     `which node` en una terminal)."
                }
            },
        ));
    }
    // `.`/`..` permitirían esconder una ruta de red tras un prefijo local.
    if path
        .split(['/', '\\'])
        .any(|part| part == "." || part == "..")
    {
        return Err(invalid_path(
            target,
            format!("La ruta {of} contiene «.» o «..»: escribe la ruta normalizada."),
        ));
    }
    // Comprobar rutas de red (UNC, `//host`, autofs `/net/`) abriría conexiones salientes o
    // colgaría la app; además, el programa se ejecuta: debe ser local.
    let lower = path.to_ascii_lowercase();
    if path.starts_with("\\\\") || path.starts_with("//") || lower.starts_with("/net/") {
        return Err(invalid_path(
            target,
            format!("La ruta {of} es una ruta de red: usa un ejecutable local."),
        ));
    }
    // Su carpeta va al `PATH`: no puede contener el separador (`:`; `;` en Windows).
    if target == PathTarget::Node
        && Path::new(path)
            .parent()
            .is_some_and(|dir| std::env::join_paths([dir]).is_err())
    {
        return Err(invalid_path(
            target,
            "La carpeta de node contiene el separador del PATH («:» o «;»): usa una ruta sin él.",
        ));
    }
    if target == PathTarget::Node
        && !Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_node_file_name)
    {
        return Err(invalid_path(
            target,
            if cfg!(windows) {
                "Indica el ejecutable node.exe (no su carpeta ni otro programa)."
            } else {
                "Indica el ejecutable llamado «node» (no su carpeta ni otro programa)."
            },
        ));
    }
    Ok(())
}

/// Resultado de la comprobación en disco (sin el detalle del sistema: no se muestra).
enum DiskCheck {
    Ok,
    Missing,
    NotFile,
    NotExecutable,
    /// Dentro de los datos de la app (solo se comprueba para `node`).
    InsideData,
}

/// Comprueba en disco, fuera del runtime asíncrono y con límite de tiempo, que la ruta lleva a
/// un archivo normal (tras resolver enlaces simbólicos: la instalación npm usa un enlace en
/// `.bin`, Homebrew otro para `node`) y, en Unix, con permiso de ejecución. En Windows basta
/// con que sea un archivo.
async fn check_on_disk(
    target: PathTarget,
    path: String,
    data_dir: Option<PathBuf>,
) -> Result<(), CoreError> {
    let task = tokio::task::spawn_blocking(move || {
        let Ok(resolved) = std::fs::canonicalize(&path) else {
            return DiskCheck::Missing;
        };
        // Un `node` dentro de los datos (p. ej. un worktree) lo podría reescribir un agente.
        if let Some(data) = data_dir.and_then(|dir| std::fs::canonicalize(dir).ok())
            && resolved.starts_with(&data)
        {
            return DiskCheck::InsideData;
        }
        let Ok(metadata) = std::fs::metadata(&resolved) else {
            return DiskCheck::Missing;
        };
        if !metadata.is_file() {
            return DiskCheck::NotFile;
        }
        if is_executable(&metadata) {
            DiskCheck::Ok
        } else {
            DiskCheck::NotExecutable
        }
    });
    match tokio::time::timeout(ADAPTER_CHECK_TIMEOUT, task).await {
        Ok(Ok(DiskCheck::Ok)) => Ok(()),
        Ok(Ok(DiskCheck::Missing)) => Err(invalid_path(
            target,
            "No existe ningún archivo en esa ruta (o no se puede leer).",
        )),
        Ok(Ok(DiskCheck::NotFile)) => Err(invalid_path(
            target,
            match target {
                PathTarget::Adapter(_) => {
                    "Esa ruta no es un archivo: indica el ejecutable del adaptador, no su carpeta."
                }
                PathTarget::Node => {
                    "Esa ruta no es un archivo: indica el ejecutable de node, no su carpeta."
                }
            },
        )),
        Ok(Ok(DiskCheck::NotExecutable)) => Err(invalid_path(
            target,
            "Ese archivo no tiene permiso de ejecución.",
        )),
        Ok(Ok(DiskCheck::InsideData)) => Err(invalid_path(
            target,
            "Esa ruta está dentro de los datos de Kanban IA (donde trabajan los agentes): usa una \
             instalación de node fuera de ellos.",
        )),
        Ok(Err(_)) | Err(_) => Err(invalid_path(
            target,
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
