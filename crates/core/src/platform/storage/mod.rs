//! Almacenamiento SQLite local: un único propietario por directorio de datos.
//!
//! - `Store::open` toma el bloqueo exclusivo *antes* de tocar la base, configura WAL,
//!   claves foráneas y busy timeout, y aplica migraciones (cada una en su transacción).
//! - Si hay migraciones pendientes sobre una base existente, primero guarda una copia
//!   consistente (`VACUUM INTO`, incluye el WAL) en `backups/` (formato de `backup.rs`).
//! - Las mutaciones usan `write`/`write_idempotent` (`BEGIN IMMEDIATE`). Las lecturas
//!   pueden usar `reader()` directamente.

mod backup;
mod lock;
mod status;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use sqlx::migrate::{MigrateError, Migrator};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::{Sqlite, SqliteConnection, Transaction};

pub use backup::{
    BackupInfo, BackupKind, MAX_KEPT, RestoreOutcome, RestoreSource, backup_path, list_backups,
    restore_backup, restore_backup_waiting, save_backup_copy, verify_backup,
};
pub use status::StorageStatus;

use crate::error::CoreError;
use crate::platform::clock::{Clock, SystemClock};
use lock::DataDirLock;

pub const DATABASE_FILE: &str = "kanbania.sqlite3";
pub const BACKUP_DIR: &str = "backups";

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONNECTIONS: u32 = 4;
const MIGRATIONS_TABLE: &str = "_sqlx_migrations";

pub(crate) static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Versión de esquema que deja `Store::open` (última migración embebida).
pub fn latest_schema_version() -> i64 {
    MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0)
}

/// Futuro devuelto por las operaciones dentro de una transacción. `Send` para poder usarse
/// desde comandos asíncronos de Tauri y, en M2, del servicio local.
pub type TxFuture<'c, T> = Pin<Box<dyn Future<Output = Result<T, CoreError>> + Send + 'c>>;

#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
    database_path: PathBuf,
    _lock: Arc<DataDirLock>,
    clock: Arc<dyn Clock>,
    /// Hay un supervisor de ejecuciones activo sobre este almacén (ver
    /// `features::runs::SupervisorToken`). Compartido por los clones.
    supervisor: Arc<AtomicBool>,
}

impl Store {
    pub async fn open(data_dir: &Path) -> Result<Self, CoreError> {
        Self::open_with(data_dir, &MIGRATOR).await
    }

    pub(crate) async fn open_with(data_dir: &Path, migrator: &Migrator) -> Result<Self, CoreError> {
        let lock = DataDirLock::acquire(data_dir)?;
        let database_path = data_dir.join(DATABASE_FILE);
        let existed = database_path.exists();

        let options = SqliteConnectOptions::new()
            .filename(&database_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT)
            // Defensa: el esquema no puede invocar funciones con efectos (triggers/vistas).
            .pragma("trusted_schema", "OFF");
        let pool = SqlitePoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .connect_with(options)
            .await?;

        let store = Self {
            pool,
            database_path,
            _lock: Arc::new(lock),
            clock: Arc::new(SystemClock),
            supervisor: Arc::new(AtomicBool::new(false)),
        };

        let mut backup = None;
        if let Err(error) = store
            .prepare(migrator, existed, data_dir, &mut backup)
            .await
        {
            store.close().await;
            // Con varias migraciones pendientes, sqlx confirma cada una por separado: si una
            // posterior falla, se restaura la copia previa para dejar la base exactamente igual.
            if let (CoreError::MigrationFailed(_), Some(backup)) = (&error, &backup) {
                restore_previous(backup, &store.database_path).map_err(CoreError::StorageIo)?;
            }
            return Err(error);
        }
        Ok(store)
    }

    async fn prepare(
        &self,
        migrator: &Migrator,
        existed: bool,
        data_dir: &Path,
        backup: &mut Option<PathBuf>,
    ) -> Result<(), CoreError> {
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&self.pool)
            .await?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            // WAL no disponible (p. ej. sistema de archivos de red): no operar sin él.
            return Err(CoreError::StorageIo(std::io::Error::other(format!(
                "journal_mode={journal_mode}, se requiere WAL en disco local"
            ))));
        }

        let applied = self.applied_versions().await?;
        if let Some(unknown) = applied.iter().find(|v| !migrator.version_exists(**v)) {
            return Err(CoreError::NewerSchema(*unknown));
        }

        let pending = migrator
            .iter()
            .any(|migration| !applied.contains(&migration.version));
        if pending && existed {
            *backup = Some(backup::pre_migration_backup(self, data_dir).await?);
        }

        migrator.run(&self.pool).await.map_err(|error| match error {
            MigrateError::VersionMissing(version) => CoreError::NewerSchema(version),
            other => CoreError::MigrationFailed(other),
        })
    }

    async fn applied_versions(&self) -> Result<Vec<i64>, CoreError> {
        let has_table: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?)",
        )
        .bind(MIGRATIONS_TABLE)
        .fetch_one(&self.pool)
        .await?;
        if !has_table {
            return Ok(Vec::new());
        }

        let versions = sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success = 1")
            .fetch_all(&self.pool)
            .await?;
        Ok(versions)
    }

    /// Sustituye el reloj (pruebas con tiempo controlado). Afecta a esta copia del almacén y a
    /// las que se clonen de ella.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Hora del reloj del almacén, en milisegundos Unix.
    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// Marca del supervisor de ejecuciones de este almacén (y de sus clones).
    pub(crate) fn supervisor_flag(&self) -> &Arc<AtomicBool> {
        &self.supervisor
    }

    /// Directorio de datos (carpeta de la base).
    pub(crate) fn data_dir(&self) -> Option<&Path> {
        self.database_path.parent()
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Pool para lecturas. Las mutaciones deben pasar por `write` o `write_idempotent`.
    pub fn reader(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn schema_version(&self) -> Result<i64, CoreError> {
        Ok(self
            .applied_versions()
            .await?
            .into_iter()
            .max()
            .unwrap_or(0))
    }

    /// Ejecuta `mutation` en una transacción `BEGIN IMMEDIATE` corta y la confirma si tiene éxito.
    pub async fn write<T, F>(&self, mutation: F) -> Result<T, CoreError>
    where
        T: Send,
        F: for<'c> FnOnce(&'c mut SqliteConnection) -> TxFuture<'c, T> + Send,
    {
        let mut tx = self.begin_write().await?;
        let result = mutation(&mut tx).await?;
        tx.commit().await?;
        Ok(result)
    }

    /// `BEGIN IMMEDIATE` toma el bloqueo de escritura al inicio: evita fallos `SQLITE_BUSY`
    /// al promover una transacción de lectura a escritura y respeta el busy timeout.
    pub(crate) async fn begin_write(&self) -> Result<Transaction<'static, Sqlite>, CoreError> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

/// Sustituye la base por la copia previa. Requiere el pool cerrado y el bloqueo tomado.
/// Elimina WAL/SHM (pertenecen al estado fallido) y reemplaza el archivo de forma atómica.
fn restore_previous(backup: &Path, database: &Path) -> std::io::Result<()> {
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = database.as_os_str().to_owned();
        sidecar.push(suffix);
        match std::fs::remove_file(PathBuf::from(sidecar)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }

    let mut staging = database.as_os_str().to_owned();
    staging.push(".restoring");
    let staging = PathBuf::from(staging);
    std::fs::copy(backup, &staging)?;
    std::fs::File::open(&staging)?.sync_all()?;
    std::fs::rename(&staging, database)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    /// Última versión real; las migraciones de prueba usan 9001+ para ir siempre después.
    fn latest() -> i64 {
        latest_schema_version()
    }

    /// Migrador con todas las migraciones reales (mismo contenido y checksum) más `extra`.
    async fn migrator_with(extra: &[(&str, &str)]) -> (tempfile::TempDir, Migrator) {
        let dir = tempfile::tempdir().unwrap();
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for entry in std::fs::read_dir(real).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
        }
        for (name, sql) in extra {
            std::fs::write(dir.path().join(name), sql).unwrap();
        }
        let migrator = Migrator::new(dir.path()).await.unwrap();
        (dir, migrator)
    }

    async fn insert_receipt(store: &Store, request_id: &str) {
        let request_id = request_id.to_owned();
        store
            .write(move |conn| {
                Box::pin(async move {
                    sqlx::query(
                        "INSERT INTO mutation_receipts \
                         (actor, request_id, operation, payload_hash, response_json) \
                         VALUES ('test', ?, 'op', 'h', '{}')",
                    )
                    .bind(request_id)
                    .execute(&mut *conn)
                    .await?;
                    Ok(())
                })
            })
            .await
            .unwrap();
    }

    async fn receipt_count(store: &Store) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM mutation_receipts")
            .fetch_one(store.reader())
            .await
            .unwrap()
    }

    async fn table_exists(store: &Store, name: &str) -> bool {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = ?)")
            .bind(name)
            .fetch_one(store.reader())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn failed_migration_rolls_back_and_keeps_previous_data() {
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).await.unwrap();
        insert_receipt(&store, "before-upgrade").await;
        store.close().await;
        drop(store);

        // La 9001 crea una tabla y luego falla: nada de ella debe quedar aplicado.
        let (_src, broken) = migrator_with(&[(
            "9001_broken.sql",
            "CREATE TABLE should_not_exist (x INTEGER);\nINSERT INTO missing_table VALUES (1);",
        )])
        .await;
        let error = Store::open_with(data.path(), &broken).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::StorageMigrationFailed);
        assert!(error.to_payload(None).recoverable);

        // Copia previa consistente creada antes de migrar.
        let backups = list_backups(data.path()).unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].kind, BackupKind::PreMigration);
        assert_eq!(backups[0].schema_version, Some(latest()));

        // El bloqueo se liberó y la versión actual abre la base intacta.
        let store = Store::open(data.path()).await.unwrap();
        assert_eq!(store.schema_version().await.unwrap(), latest());
        assert_eq!(receipt_count(&store).await, 1);
        assert!(!table_exists(&store, "should_not_exist").await);
    }

    #[tokio::test]
    async fn later_failure_with_several_pending_migrations_restores_previous_database() {
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).await.unwrap();
        insert_receipt(&store, "before-upgrade").await;
        store.close().await;
        drop(store);

        // 9001 se aplica y confirma (cambia datos y esquema); 9002 falla después.
        let (_src, partial) = migrator_with(&[
            (
                "9001_ok.sql",
                "CREATE TABLE added_by_9001 (x INTEGER);\n\
                 UPDATE mutation_receipts SET operation = 'changed_by_9001';",
            ),
            ("9002_broken.sql", "INSERT INTO missing_table VALUES (1);"),
        ])
        .await;
        let error = Store::open_with(data.path(), &partial).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::StorageMigrationFailed);

        let store = Store::open(data.path()).await.unwrap();
        assert_eq!(store.schema_version().await.unwrap(), latest());
        assert!(!table_exists(&store, "added_by_9001").await);
        let operation: String = sqlx::query_scalar("SELECT operation FROM mutation_receipts")
            .fetch_one(store.reader())
            .await
            .unwrap();
        assert_eq!(operation, "op");
    }

    #[tokio::test]
    async fn database_from_newer_version_is_rejected_without_changes() {
        let data = tempfile::tempdir().unwrap();
        let (_src, newer) = migrator_with(&[(
            "9001_future.sql",
            "CREATE TABLE future_feature (x INTEGER);",
        )])
        .await;
        let store = Store::open_with(data.path(), &newer).await.unwrap();
        insert_receipt(&store, "from-future").await;
        store.close().await;
        drop(store);

        let error = Store::open(data.path()).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::StorageNewerSchema);

        let store = Store::open_with(data.path(), &newer).await.unwrap();
        assert_eq!(store.schema_version().await.unwrap(), 9001);
        assert!(table_exists(&store, "future_feature").await);
        assert_eq!(receipt_count(&store).await, 1);
    }

    #[tokio::test]
    async fn new_database_does_not_create_a_backup() {
        let data = tempfile::tempdir().unwrap();
        let _store = Store::open(data.path()).await.unwrap();

        assert!(!data.path().join(BACKUP_DIR).exists());
    }

    /// HU-015: la 0010 reconstruye `story_claims`. Una base en la 0009 con reclamos abiertos y
    /// cerrados conserva sus filas y el índice de un solo reclamo abierto por historia.
    #[tokio::test]
    async fn migration_0010_keeps_claims_and_their_unique_index() {
        use crate::features::access::{CreateAgentCredentialInput, create_agent_credential};
        use crate::features::projects::{CreateProjectInput, create_project};
        use crate::features::stories::{CreateStoryInput, create_story};
        use crate::features::team::{AgentFields, AvatarSpec, CreateAgentInput, create_agent};
        use crate::platform::actor::Actor;

        let migrations = tempfile::tempdir().unwrap();
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for entry in std::fs::read_dir(real).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            if name.as_str() < "0010" {
                std::fs::copy(entry.path(), migrations.path().join(&name)).unwrap();
            }
        }
        let v9 = Migrator::new(migrations.path()).await.unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = Store::open_with(data.path(), &v9).await.unwrap();
        assert_eq!(store.schema_version().await.unwrap(), 9);
        let project = create_project(
            &store,
            "ui",
            &CreateProjectInput {
                request_id: "p".into(),
                name: "P".into(),
            },
        )
        .await
        .unwrap();
        let agent = create_agent(
            &store,
            "ui",
            &CreateAgentInput {
                request_id: "a".into(),
                fields: AgentFields {
                    name: "Bot".into(),
                    role: String::new(),
                    provider: String::new(),
                    model: String::new(),
                    capabilities: vec![],
                    avatar: AvatarSpec::DEFAULT_AGENT,
                },
            },
        )
        .await
        .unwrap();
        let credential = create_agent_credential(
            &store,
            &Actor::Human,
            &CreateAgentCredentialInput {
                project_id: project.id.clone(),
                agent_id: agent.id.clone(),
            },
        )
        .await
        .unwrap()
        .credential;
        let story = create_story(
            &store,
            "ui",
            &CreateStoryInput {
                request_id: "s".into(),
                project_id: project.id.clone(),
                title: "S".into(),
                column_id: None,
                priority: None,
            },
        )
        .await
        .unwrap();
        for (id, ended) in [("k-cerrado", Some(5_i64)), ("k-abierto", None)] {
            sqlx::query(
                "INSERT INTO story_claims (id, project_id, story_id, agent_id, credential_id, \
                 claimed_at_ms, expires_at_ms, ended_at_ms, end_reason) \
                 VALUES (?, ?, ?, ?, ?, 1, 10, ?, ?)",
            )
            .bind(id)
            .bind(&project.id)
            .bind(&story.id)
            .bind(&agent.id)
            .bind(&credential.id)
            .bind(ended)
            .bind(ended.map(|_| "released"))
            .execute(store.reader())
            .await
            .unwrap();
        }
        store.close().await;
        drop(store);

        let store = Store::open(data.path()).await.unwrap();
        assert_eq!(store.schema_version().await.unwrap(), latest());
        let rows: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT id, end_reason FROM story_claims ORDER BY id")
                .fetch_all(store.reader())
                .await
                .unwrap();
        assert_eq!(
            rows,
            vec![
                ("k-abierto".to_owned(), None),
                ("k-cerrado".to_owned(), Some("released".to_owned()))
            ]
        );
        // El índice único sigue: otro reclamo abierto de la misma historia falla.
        let duplicate = sqlx::query(
            "INSERT INTO story_claims (id, project_id, story_id, agent_id, credential_id, \
             claimed_at_ms, expires_at_ms) VALUES ('k-otro', ?, ?, ?, ?, 1, 10)",
        )
        .bind(&project.id)
        .bind(&story.id)
        .bind(&agent.id)
        .bind(&credential.id)
        .execute(store.reader())
        .await;
        assert!(duplicate.is_err());
        // Y los motivos nuevos son válidos.
        sqlx::query(
            "UPDATE story_claims SET ended_at_ms = 6, end_reason = 'delivered' \
             WHERE id = 'k-abierto'",
        )
        .execute(store.reader())
        .await
        .unwrap();
    }

    /// La restauración retoca la copia **antes de migrarla**. Una copia en la 0011 (sin las
    /// tablas de la 0012) con un intento en cola se restaura y, tras migrar, queda
    /// `interrupted` con su evento.
    #[tokio::test]
    async fn restoring_a_schema_11_copy_interrupts_its_runs_before_migrating() {
        use crate::features::projects::{CreateProjectInput, create_project};
        use crate::features::stories::{CreateStoryInput, create_story};
        use crate::features::team::{AgentFields, AvatarSpec, CreateAgentInput, create_agent};

        let migrations = tempfile::tempdir().unwrap();
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for entry in std::fs::read_dir(real).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            if name.as_str() < "0012" {
                std::fs::copy(entry.path(), migrations.path().join(&name)).unwrap();
            }
        }
        let v11 = Migrator::new(migrations.path()).await.unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = Store::open_with(data.path(), &v11).await.unwrap();
        assert_eq!(store.schema_version().await.unwrap(), 11);
        let project = create_project(
            &store,
            "ui",
            &CreateProjectInput {
                request_id: "p".into(),
                name: "P".into(),
            },
        )
        .await
        .unwrap();
        let agent = create_agent(
            &store,
            "ui",
            &CreateAgentInput {
                request_id: "a".into(),
                fields: AgentFields {
                    name: "Bot".into(),
                    role: String::new(),
                    provider: String::new(),
                    model: String::new(),
                    capabilities: vec![],
                    avatar: AvatarSpec::DEFAULT_AGENT,
                },
            },
        )
        .await
        .unwrap();
        let story = create_story(
            &store,
            "ui",
            &CreateStoryInput {
                request_id: "s".into(),
                project_id: project.id.clone(),
                title: "S".into(),
                column_id: None,
                priority: None,
            },
        )
        .await
        .unwrap();
        // Intento en cola tal como lo dejaba la versión de la 0011 (sin sus funciones actuales,
        // que ya leen columnas de la 0012).
        let (project_id, story_id, agent_id) = (project.id.clone(), story.id.clone(), agent.id);
        store
            .write(move |conn| {
                Box::pin(async move {
                    sqlx::query(
                        "INSERT INTO runs (id, project_id, story_id, number, agent_id, state, \
                         snapshot, snapshot_sha256, last_sequence, created_at_ms, updated_at_ms) \
                         VALUES ('run-11', ?, ?, 1, ?, 'queued', '{}', ?, 1, 1, 1)",
                    )
                    .bind(&project_id)
                    .bind(&story_id)
                    .bind(&agent_id)
                    .bind("0".repeat(64))
                    .execute(&mut *conn)
                    .await?;
                    sqlx::query(
                        "INSERT INTO run_events (run_id, sequence, kind, payload, at_ms) VALUES \
                         ('run-11', 1, 'state_changed', \
                          '{\"kind\":\"state_changed\",\"to\":\"queued\",\"reason\":null}', 1)",
                    )
                    .execute(&mut *conn)
                    .await?;
                    Ok(())
                })
            })
            .await
            .unwrap();
        let backup = store.create_backup(BackupKind::Manual).await.unwrap();
        assert_eq!(backup.schema_version, Some(11));
        store.close().await;
        drop(store);

        let outcome = restore_backup(data.path(), &backup_path(data.path(), &backup.id).unwrap())
            .await
            .unwrap();
        assert_eq!(outcome.restored_schema_version, 11);
        let store = Store::open(data.path()).await.unwrap();
        assert_eq!(
            store.schema_version().await.unwrap(),
            latest_schema_version()
        );
        let (state, reason, last): (String, Option<String>, i64) =
            sqlx::query_as("SELECT state, end_reason, last_sequence FROM runs WHERE id = 'run-11'")
                .fetch_one(store.reader())
                .await
                .unwrap();
        assert_eq!(state, "interrupted");
        assert!(reason.unwrap().contains("restauró una copia"));
        assert_eq!(last, 2);
        let events: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM run_events WHERE run_id = 'run-11'")
                .fetch_one(store.reader())
                .await
                .unwrap();
        assert_eq!(events, 2);
    }
}
