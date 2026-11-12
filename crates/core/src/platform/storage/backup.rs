//! Copias de seguridad locales (HU-011).
//!
//! Cada copia es una carpeta `backups/<id>/` con la base (`kanbania.sqlite3`) y un
//! `manifest.json` (formato, versión de esquema, sha256 de cada archivo y lo que **no**
//! incluye). Se crean con `VACUUM INTO`: una transacción de lectura que produce un archivo
//! coherente con el WAL activo y con escrituras concurrentes.
//!
//! Restaurar verifica la copia (manifiesto, hashes, `integrity_check`, claves foráneas y que
//! el esquema no sea más reciente) sobre una copia de trabajo, guarda antes una copia
//! «previa a restaurar» del estado actual y solo entonces sustituye la base. Requiere el
//! almacén cerrado: toma el mismo bloqueo exclusivo que `Store::open`, así que nadie escribe
//! durante la sustitución.
//!
//! Se conservan las `MAX_KEPT` copias más recientes. Solo se borran carpetas con un manifiesto
//! válido de este formato; nunca otra cosa.

use crate::features::claims::close_restored_claims;
use crate::platform::clock::{Clock, SystemClock};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, SqliteConnection};
use ts_rs::TS;

use super::lock::DataDirLock;
use super::{BACKUP_DIR, DATABASE_FILE, MIGRATOR, Store};
use crate::error::CoreError;
use crate::features::access::{current_revocations, reconcile_restored};

const FORMAT: &str = "kanbania-backup";
const VERSION: u32 = 1;
const MANIFEST: &str = "manifest.json";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const WAL_FILE: &str = "kanbania.sqlite3-wal";
/// Copias que se conservan de cada grupo (manuales / automáticas); las más antiguas se borran
/// al crear una nueva. Por grupos, para que las automáticas nunca desplacen a las manuales.
pub const MAX_KEPT: usize = 10;

/// Lo que una copia no contiene. Se guarda en el manifiesto y se muestra en la app.
const EXCLUDES: [&str; 3] = [
    "El contenido de los repositorios vinculados (solo se guardan sus rutas).",
    "Registros de ejecuciones de agentes: llegarán en M3 y se guardarán fuera de la base.",
    "Las carpetas exportadas en Markdown: son una exportación de la planificación, no una copia.",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BackupKind {
    /// Creada por la persona.
    Manual,
    /// Automática: estado anterior a una restauración (para deshacerla).
    PreRestore,
    /// Automática: estado anterior a aplicar migraciones.
    PreMigration,
}

impl BackupKind {
    fn slug(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::PreRestore => "pre-restore",
            Self::PreMigration => "pre-migration",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    kind: BackupKind,
    created_at: String,
    app_version: String,
    schema_version: Option<i64>,
    files: Vec<FileEntry>,
    excludes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileEntry {
    name: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupInfo {
    /// Nombre de la carpeta dentro de `backups/`.
    pub id: String,
    pub kind: BackupKind,
    pub created_at: String,
    pub app_version: String,
    /// `null` si no se pudo leer (copia en bruto de una base dañada).
    #[ts(type = "number | null")]
    pub schema_version: Option<i64>,
    #[ts(type = "number")]
    pub bytes: i64,
    pub excludes: Vec<String>,
}

/// Origen de una restauración: una copia interna (por id) o una carpeta elegida.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum RestoreSource {
    Backup { id: String },
    Folder { path: String },
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreOutcome {
    /// Copia del estado anterior, para deshacer la restauración.
    pub safety_backup: Option<BackupInfo>,
    /// Versión de esquema de la copia restaurada (se migrará al abrir si es anterior).
    #[ts(type = "number")]
    pub restored_schema_version: i64,
}

fn invalid(reason: impl Into<String>) -> CoreError {
    CoreError::Validation {
        field: "backup",
        reason: reason.into(),
    }
}

fn io(error: std::io::Error) -> CoreError {
    CoreError::StorageIo(error)
}

/// Marca de tiempo UTC: (ISO 8601, compacta para nombres de carpeta).
fn now_utc() -> (String, String) {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = i64::try_from(elapsed.as_secs()).unwrap_or(0);
    let millis = elapsed.subsec_millis();
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3600, rem % 3600 / 60, rem % 60);
    (
        format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{millis:03}Z"),
        format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}-{millis:03}"),
    )
}

/// Días desde 1970-01-01 → (año, mes, día) en el calendario gregoriano (algoritmo de
/// Howard Hinnant), sin dependencias de fecha.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn sha256_file(path: &Path) -> Result<(u64, String), CoreError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(io)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer).map_err(io)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += u64::try_from(read).unwrap_or(0);
    }
    let hex = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((total, hex))
}

fn sync_file(path: &Path) -> Result<(), CoreError> {
    std::fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(io)
}

/// Tras renombrar, sincroniza el directorio (Unix) para que el cambio sobreviva a un corte de
/// energía. Es una mejora de durabilidad: si falla, no se aborta.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Err(error) = std::fs::File::open(dir).and_then(|d| d.sync_all()) {
        eprintln!("[backup] no se pudo sincronizar el directorio: {error}");
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// El esquema real de la copia debe ser idéntico al que dejan las migraciones de esta app
/// hasta su versión: una copia ajena no puede traer triggers, vistas o tablas alteradas que
/// rompan reglas del núcleo (p. ej. un trigger que marque historias como aceptadas).
async fn verify_schema(
    conn: &mut SqliteConnection,
    work: &Path,
    version: i64,
) -> Result<(), CoreError> {
    let tampered =
        || invalid("El esquema de la copia no coincide con el de esta app: no se restaura.");
    let applied: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations WHERE success = 1")
            .fetch_all(&mut *conn)
            .await?;
    for (applied_version, checksum) in &applied {
        let known = MIGRATOR
            .iter()
            .find(|m| m.version == *applied_version)
            .ok_or_else(tampered)?;
        if known.checksum.as_ref() != checksum.as_slice() {
            return Err(tampered());
        }
    }

    let reference_path = work.join("reference.sqlite3");
    let mut reference = SqliteConnectOptions::new()
        .filename(&reference_path)
        .create_if_missing(true)
        .connect()
        .await?;
    for migration in MIGRATOR
        .iter()
        .filter(|m| m.version <= version && !m.migration_type.is_down_migration())
    {
        sqlx::raw_sql(migration.sql.clone())
            .execute(&mut reference)
            .await?;
    }
    let schema = "SELECT type, name, tbl_name, ifnull(sql, '') FROM sqlite_master \
                  WHERE name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations' \
                  ORDER BY type, name";
    let expected: Vec<(String, String, String, String)> =
        sqlx::query_as(schema).fetch_all(&mut reference).await?;
    reference.close().await.ok();
    let actual: Vec<(String, String, String, String)> =
        sqlx::query_as(schema).fetch_all(&mut *conn).await?;
    if actual != expected {
        return Err(tampered());
    }
    Ok(())
}

fn path_str(path: &Path) -> Result<&str, CoreError> {
    path.to_str()
        .ok_or_else(|| io(std::io::Error::other("ruta no UTF-8")))
}

async fn vacuum_into(conn: &mut SqliteConnection, target: &Path) -> Result<(), CoreError> {
    sqlx::query("VACUUM INTO ?")
        .bind(path_str(target)?)
        .execute(&mut *conn)
        .await?;
    // VACUUM INTO no sincroniza su salida.
    sync_file(target)
}

async fn schema_version_of(conn: &mut SqliteConnection) -> Result<Option<i64>, CoreError> {
    let has_table: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' \
         AND name = '_sqlx_migrations')",
    )
    .fetch_one(&mut *conn)
    .await?;
    if !has_table {
        return Ok(None);
    }
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success = 1")
            .fetch_all(&mut *conn)
            .await?;
    if let Some(unknown) = versions.iter().find(|v| !MIGRATOR.version_exists(**v)) {
        return Err(CoreError::NewerSchema(*unknown));
    }
    Ok(versions.into_iter().max())
}

async fn connect(path: &Path) -> Result<SqliteConnection, CoreError> {
    Ok(SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .connect()
        .await?)
}

fn backups_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(BACKUP_DIR)
}

/// Carpeta de trabajo nueva y vacía dentro de `backups/` (oculta: la lista la ignora).
fn work_dir(data_dir: &Path, label: &str) -> Result<PathBuf, CoreError> {
    let base = backups_dir(data_dir);
    std::fs::create_dir_all(&base).map_err(io)?;
    let (_, stamp) = now_utc();
    for n in 0..100 {
        let dir = base.join(format!(".work-{label}-{stamp}-{n}"));
        if std::fs::create_dir(&dir).is_ok() {
            return Ok(dir);
        }
    }
    Err(io(std::io::Error::other(
        "no se pudo crear la carpeta de trabajo",
    )))
}

/// Escribe el manifiesto y mueve la carpeta de trabajo a `backups/<id>`; aplica retención.
fn finalize(
    data_dir: &Path,
    work: &Path,
    kind: BackupKind,
    schema_version: Option<i64>,
    protect: Option<&str>,
) -> Result<BackupInfo, CoreError> {
    let mut files = Vec::new();
    for name in [DATABASE_FILE, WAL_FILE] {
        let path = work.join(name);
        if path.exists() {
            let (bytes, sha256) = sha256_file(&path)?;
            files.push(FileEntry {
                name: name.into(),
                bytes,
                sha256,
            });
        }
    }
    let (created_at, stamp) = now_utc();
    let manifest = Manifest {
        format: FORMAT.into(),
        version: VERSION,
        kind,
        created_at,
        app_version: env!("CARGO_PKG_VERSION").into(),
        schema_version,
        files,
        excludes: EXCLUDES.iter().map(|s| (*s).to_owned()).collect(),
    };
    let manifest_path = work.join(MANIFEST);
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?).map_err(io)?;
    sync_file(&manifest_path)?;

    let base = backups_dir(data_dir);
    let mut id = format!("{stamp}-{}", kind.slug());
    let mut n = 2;
    while base.join(&id).exists() {
        id = format!("{stamp}-{}-{n}", kind.slug());
        n += 1;
    }
    std::fs::rename(work, base.join(&id)).map_err(io)?;
    sync_dir(&base);
    prune(data_dir, &[Some(id.as_str()), protect]);
    Ok(info(id, &manifest))
}

fn info(id: String, manifest: &Manifest) -> BackupInfo {
    BackupInfo {
        id,
        kind: manifest.kind,
        created_at: manifest.created_at.clone(),
        app_version: manifest.app_version.clone(),
        schema_version: manifest.schema_version,
        bytes: i64::try_from(manifest.files.iter().map(|f| f.bytes).sum::<u64>())
            .unwrap_or(i64::MAX),
        excludes: manifest.excludes.clone(),
    }
}

fn read_manifest(dir: &Path) -> Result<Manifest, CoreError> {
    let path = dir.join(MANIFEST);
    let meta = std::fs::symlink_metadata(&path).map_err(|_| {
        invalid("La carpeta no contiene una copia de Kanban IA (falta manifest.json).")
    })?;
    if !meta.is_file() || meta.len() > MAX_MANIFEST_BYTES {
        return Err(invalid("El manifiesto de la copia no es válido."));
    }
    let bytes = std::fs::read(&path).map_err(io)?;
    let probe: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| invalid("El manifiesto de la copia no es JSON válido."))?;
    if probe.get("format").and_then(serde_json::Value::as_str) != Some(FORMAT) {
        return Err(invalid("La carpeta no contiene una copia de Kanban IA."));
    }
    match probe.get("version").and_then(serde_json::Value::as_u64) {
        Some(v) if v == u64::from(VERSION) => {}
        Some(v) if v > u64::from(VERSION) => {
            return Err(invalid(format!(
                "La copia usa la versión {v} del formato, más reciente que esta app."
            )));
        }
        _ => return Err(invalid("Versión de copia no válida.")),
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|_| invalid("El manifiesto de la copia no cumple el esquema."))?;
    let names_ok = manifest.files.iter().any(|f| f.name == DATABASE_FILE)
        && manifest
            .files
            .iter()
            .all(|f| f.name == DATABASE_FILE || f.name == WAL_FILE);
    if !names_ok {
        return Err(invalid(
            "El manifiesto de la copia lista archivos no válidos.",
        ));
    }
    Ok(manifest)
}

/// Copias de `backups/`, más reciente primero. Ignora carpetas sin manifiesto válido.
pub fn list_backups(data_dir: &Path) -> Result<Vec<BackupInfo>, CoreError> {
    let base = backups_dir(data_dir);
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Ok(Vec::new());
    };
    let mut list = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        if let Ok(manifest) = read_manifest(&entry.path()) {
            list.push(info(name, &manifest));
        }
    }
    list.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    Ok(list)
}

/// Retención: conserva las `MAX_KEPT` más recientes de cada grupo (manuales y automáticas) y
/// siempre la recién creada. Los fallos al borrar no hacen fallar la copia.
fn prune(data_dir: &Path, keep: &[Option<&str>]) {
    let Ok(list) = list_backups(data_dir) else {
        return;
    };
    for manual in [true, false] {
        let group = list
            .iter()
            .filter(|b| (b.kind == BackupKind::Manual) == manual);
        for old in group
            .skip(MAX_KEPT)
            .filter(|b| !keep.contains(&Some(b.id.as_str())))
        {
            let dir = backups_dir(data_dir).join(&old.id);
            if std::fs::remove_dir_all(&dir).is_err() {
                eprintln!("[backup] no se pudo borrar la copia antigua {}", old.id);
            }
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && !id.starts_with('.')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Carpeta de una copia interna por su id (sin permitir salir de `backups/`).
pub fn backup_path(data_dir: &Path, id: &str) -> Result<PathBuf, CoreError> {
    if !valid_id(id) {
        return Err(invalid("Identificador de copia no válido."));
    }
    let dir = backups_dir(data_dir).join(id);
    if !dir.is_dir() {
        return Err(CoreError::NotFound { entity: "Copia" });
    }
    Ok(dir)
}

impl Store {
    /// Crea una copia consistente de la base en `backups/` (aunque haya escrituras en curso).
    pub async fn create_backup(&self, kind: BackupKind) -> Result<BackupInfo, CoreError> {
        let data_dir = self
            .database_path
            .parent()
            .ok_or_else(|| io(std::io::Error::other("sin directorio de datos")))?
            .to_path_buf();
        let work = work_dir(&data_dir, kind.slug())?;
        let result = async {
            let mut conn = self.pool.acquire().await?;
            vacuum_into(&mut conn, &work.join(DATABASE_FILE)).await?;
            let version = schema_version_of(&mut conn).await.ok().flatten();
            drop(conn);
            finalize(&data_dir, &work, kind, version, None)
        }
        .await;
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&work);
        }
        result
    }
}

/// Guarda una copia interna en una carpeta elegida (`kanbania-backup-<id>/`), sin sobrescribir.
/// Verifica los hashes de lo escrito. Devuelve el nombre de la carpeta creada.
pub fn save_backup_copy(
    data_dir: &Path,
    id: &str,
    destination: &Path,
) -> Result<String, CoreError> {
    let source = backup_path(data_dir, id)?;
    let manifest = read_manifest(&source)?;
    if !destination.is_absolute() || !destination.is_dir() {
        return Err(CoreError::Validation {
            field: "destination",
            reason: "Elige una carpeta existente en la que se pueda escribir.".into(),
        });
    }
    let folder = format!("kanbania-backup-{id}");
    let target = destination.join(&folder);
    if target.exists() {
        return Err(CoreError::Validation {
            field: "destination",
            reason: format!("Ya existe una carpeta «{folder}» en el destino."),
        });
    }
    let partial = destination.join(format!(".{folder}.partial"));
    std::fs::create_dir(&partial).map_err(|_| CoreError::Validation {
        field: "destination",
        reason: if partial.exists() {
            format!(
                "Quedó una carpeta «.{folder}.partial» de un intento anterior: bórrala y reintenta."
            )
        } else {
            "Elige una carpeta existente en la que se pueda escribir.".into()
        },
    })?;
    let copied = (|| -> Result<(), CoreError> {
        for file in &manifest.files {
            let to = partial.join(&file.name);
            std::fs::copy(source.join(&file.name), &to).map_err(io)?;
            sync_file(&to)?;
            if sha256_file(&to)?.1 != file.sha256 {
                return Err(invalid(
                    "La copia no coincide con su manifiesto (¿dañada?).",
                ));
            }
        }
        std::fs::copy(source.join(MANIFEST), partial.join(MANIFEST)).map_err(io)?;
        std::fs::rename(&partial, &target).map_err(io)
    })();
    if let Err(error) = copied {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(error);
    }
    Ok(folder)
}

/// Restaura la copia de `source` (carpeta con manifiesto) en `data_dir`. **El almacén debe
/// estar cerrado**: se toma el bloqueo exclusivo, así que con otra instancia abierta falla con
/// `storage_locked` sin tocar nada. Si la verificación falla, la base actual queda intacta.
pub async fn restore_backup(data_dir: &Path, source: &Path) -> Result<RestoreOutcome, CoreError> {
    let _lock = DataDirLock::acquire(data_dir)?;
    let manifest = read_manifest(source)?;
    let work = work_dir(data_dir, "restore")?;
    // Si se restaura una copia interna, la retención no debe borrarla mientras tanto.
    let protect = match (
        dunce::canonicalize(source),
        dunce::canonicalize(backups_dir(data_dir)),
    ) {
        (Ok(src), Ok(base)) if src.parent() == Some(base.as_path()) => {
            src.file_name().map(|n| n.to_string_lossy().into_owned())
        }
        _ => None,
    };
    let result = restore_locked(data_dir, source, &manifest, &work, protect.as_deref()).await;
    let _ = std::fs::remove_dir_all(&work);
    if result.is_err() {
        // Sin restos: la base actual no se tocó (o ya se sustituyó, y entonces no existe).
        let _ = std::fs::remove_file(data_dir.join(format!("{DATABASE_FILE}.restoring")));
    }
    result
}

/// Como `restore_backup`, pero si el bloqueo está tomado reintenta durante `wait`: un comando
/// en curso del mismo proceso puede estar soltando su manejador (con el pool ya cerrado).
/// Con otra instancia abierta, falla con `storage_locked` al agotar el plazo.
pub async fn restore_backup_waiting(
    data_dir: &Path,
    source: &Path,
    wait: std::time::Duration,
) -> Result<RestoreOutcome, CoreError> {
    let started = std::time::Instant::now();
    loop {
        match restore_backup(data_dir, source).await {
            Err(CoreError::StorageLocked) if started.elapsed() < wait => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            other => return other,
        }
    }
}

async fn restore_locked(
    data_dir: &Path,
    source: &Path,
    manifest: &Manifest,
    work: &Path,
    protect: Option<&str>,
) -> Result<RestoreOutcome, CoreError> {
    // 1. Copia de trabajo verificada contra el manifiesto (el origen puede cambiar después).
    for file in &manifest.files {
        let from = source.join(&file.name);
        let meta = std::fs::symlink_metadata(&from)
            .map_err(|_| invalid(format!("Falta {} en la copia.", file.name)))?;
        if !meta.is_file() {
            return Err(invalid(format!("{} no es un archivo normal.", file.name)));
        }
        let to = work.join(&file.name);
        std::fs::copy(&from, &to).map_err(io)?;
        let (bytes, sha) = sha256_file(&to)?;
        if bytes != file.bytes || sha != file.sha256 {
            return Err(invalid(
                "La copia está dañada: su contenido no coincide con el manifiesto.",
            ));
        }
    }

    // 2. Integridad, claves foráneas y esquema compatible. Se consolida en un solo archivo.
    let staging = data_dir.join(format!("{DATABASE_FILE}.restoring"));
    let _ = std::fs::remove_file(&staging);
    let version = {
        let mut conn = connect(&work.join(DATABASE_FILE))
            .await
            .map_err(|_| invalid("La copia no es una base SQLite válida."))?;
        let checks = async {
            let integrity: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
                .fetch_all(&mut conn)
                .await
                .map_err(|_| invalid("La copia no es una base SQLite válida."))?;
            if integrity != ["ok"] {
                return Err(invalid(
                    "La copia está dañada (falla la comprobación de integridad).",
                ));
            }
            let broken: Vec<(String,)> = sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&mut conn)
                .await?;
            if !broken.is_empty() {
                return Err(invalid("La copia tiene referencias rotas entre tablas."));
            }
            let version = match schema_version_of(&mut conn).await {
                Ok(Some(v)) => v,
                Ok(None) => return Err(invalid("La copia no es una base de Kanban IA.")),
                Err(CoreError::NewerSchema(v)) => {
                    return Err(invalid(format!(
                        "La copia es de una versión más reciente de la app (esquema {v})."
                    )));
                }
                Err(other) => return Err(other),
            };
            verify_schema(&mut conn, work, version).await?;
            // Una copia antigua no reactiva credenciales revocadas después (ver `access`).
            let current = data_dir.join(DATABASE_FILE);
            let mut current_conn = if current.exists() {
                connect(&current).await.ok()
            } else {
                None
            };
            let revocations = current_revocations(current_conn.as_mut()).await;
            if let Some(c) = current_conn {
                c.close().await.ok();
            }
            reconcile_restored(&mut conn, &revocations).await?;
            // Ningún reclamo de la copia revive (ver `claims`).
            close_restored_claims(&mut conn, SystemClock.now_ms()).await?;
            vacuum_into(&mut conn, &staging).await?;
            Ok(version)
        }
        .await;
        conn.close().await.ok();
        checks?
    };

    // 3. Copia de seguridad del estado actual, para poder deshacer.
    let database = data_dir.join(DATABASE_FILE);
    let safety = if database.exists() {
        Some(safety_backup(data_dir, &database, protect).await?)
    } else {
        None
    };

    // 4. Sustitución: sin WAL/SHM del estado anterior, renombrado atómico.
    for suffix in ["-wal", "-shm"] {
        let sidecar = data_dir.join(format!("{DATABASE_FILE}{suffix}"));
        match std::fs::remove_file(&sidecar) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(io(error)),
            _ => {}
        }
    }
    std::fs::rename(&staging, &database).map_err(io)?;
    sync_file(&database)?;
    sync_dir(data_dir);

    Ok(RestoreOutcome {
        safety_backup: safety,
        restored_schema_version: version,
    })
}

/// Copia «previa a restaurar». Si la base actual se puede abrir, `VACUUM INTO`; si está dañada,
/// copia en bruto de la base y su WAL (para no perder nada que pudiera recuperarse).
async fn safety_backup(
    data_dir: &Path,
    database: &Path,
    protect: Option<&str>,
) -> Result<BackupInfo, CoreError> {
    let work = work_dir(data_dir, "pre-restore")?;
    let result = async {
        let consistent = async {
            let mut conn = connect(database).await?;
            vacuum_into(&mut conn, &work.join(DATABASE_FILE)).await?;
            let version = schema_version_of(&mut conn).await.ok().flatten();
            conn.close().await.ok();
            Ok::<_, CoreError>(version)
        }
        .await;
        let version = match consistent {
            Ok(version) => version,
            Err(_) => {
                let _ = std::fs::remove_file(work.join(DATABASE_FILE));
                std::fs::copy(database, work.join(DATABASE_FILE)).map_err(io)?;
                let wal = data_dir.join(WAL_FILE);
                if wal.exists() {
                    std::fs::copy(&wal, work.join(WAL_FILE)).map_err(io)?;
                }
                None
            }
        };
        finalize(data_dir, &work, BackupKind::PreRestore, version, protect)
    }
    .await;
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&work);
    }
    result
}

/// Copia previa a migrar (`Store::open`). Devuelve la ruta de la base copiada, que es la que
/// se restaura si la migración falla.
pub(super) async fn pre_migration_backup(
    store: &Store,
    data_dir: &Path,
) -> Result<PathBuf, CoreError> {
    let info = store.create_backup(BackupKind::PreMigration).await?;
    Ok(backups_dir(data_dir).join(info.id).join(DATABASE_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_are_correct() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_402), (2025, 11, 10));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn ids_cannot_escape_the_backup_folder() {
        for bad in ["", "..", "../x", "a/b", ".work-x", "a\\b", "C:"] {
            assert!(!valid_id(bad), "{bad}");
        }
        assert!(valid_id("20261110-214406-123-manual"));
    }
}
