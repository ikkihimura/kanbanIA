//! Almacenamiento SQLite local: un único propietario por directorio de datos.
//!
//! - `Store::open` toma el bloqueo exclusivo *antes* de tocar la base, configura WAL,
//!   claves foráneas y busy timeout, y aplica migraciones (cada una en su transacción).
//! - Si hay migraciones pendientes sobre una base existente, primero guarda una copia
//!   consistente (`VACUUM INTO`, incluye el WAL) en `backups/`.
//! - Las mutaciones usan `write`/`write_idempotent` (`BEGIN IMMEDIATE`). Las lecturas
//!   pueden usar `reader()` directamente.

mod lock;
mod status;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use sqlx::migrate::{MigrateError, Migrator};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::{Sqlite, SqliteConnection, Transaction};

pub use status::StorageStatus;

use crate::error::CoreError;
use lock::DataDirLock;

pub const DATABASE_FILE: &str = "kanbania.sqlite3";
pub const BACKUP_DIR: &str = "backups";

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONNECTIONS: u32 = 4;
const MIGRATIONS_TABLE: &str = "_sqlx_migrations";

pub(crate) static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Futuro devuelto por las operaciones dentro de una transacción. `Send` para poder usarse
/// desde comandos asíncronos de Tauri y, en M2, del servicio local.
pub type TxFuture<'c, T> = Pin<Box<dyn Future<Output = Result<T, CoreError>> + Send + 'c>>;

#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
    database_path: PathBuf,
    _lock: Arc<DataDirLock>,
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
            .busy_timeout(BUSY_TIMEOUT);
        let pool = SqlitePoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .connect_with(options)
            .await?;

        let store = Self {
            pool,
            database_path,
            _lock: Arc::new(lock),
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
                restore_backup(backup, &store.database_path).map_err(CoreError::StorageIo)?;
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
            let from = applied.iter().max().copied().unwrap_or(0);
            *backup = Some(self.backup_before_migration(data_dir, from).await?);
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

    async fn backup_before_migration(
        &self,
        data_dir: &Path,
        from: i64,
    ) -> Result<PathBuf, CoreError> {
        let backup_dir = data_dir.join(BACKUP_DIR);
        std::fs::create_dir_all(&backup_dir).map_err(CoreError::StorageIo)?;
        let backup_path = backup_dir.join(format!("pre-migration-v{from}.sqlite3"));
        if backup_path.exists() {
            std::fs::remove_file(&backup_path).map_err(CoreError::StorageIo)?;
        }

        let target = backup_path.to_str().ok_or_else(|| {
            CoreError::StorageIo(std::io::Error::other("ruta de copia no es UTF-8"))
        })?;
        sqlx::query("VACUUM INTO ?")
            .bind(target)
            .execute(&self.pool)
            .await?;
        // VACUUM INTO no sincroniza su salida: asegurar que la copia está en disco antes de migrar.
        std::fs::File::open(&backup_path)
            .and_then(|file| file.sync_all())
            .map_err(CoreError::StorageIo)?;
        Ok(backup_path)
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
fn restore_backup(backup: &Path, database: &Path) -> std::io::Result<()> {
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

    const MIGRATION_0001: &str = include_str!("../../../migrations/0001_mutation_receipts.sql");

    async fn migrator_with(extra: &[(&str, &str)]) -> (tempfile::TempDir, Migrator) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("0001_mutation_receipts.sql"),
            MIGRATION_0001,
        )
        .unwrap();
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

        // La 0002 crea una tabla y luego falla: nada de ella debe quedar aplicado.
        let (_src, broken) = migrator_with(&[(
            "0002_broken.sql",
            "CREATE TABLE should_not_exist (x INTEGER);\nINSERT INTO missing_table VALUES (1);",
        )])
        .await;
        let error = Store::open_with(data.path(), &broken).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::StorageMigrationFailed);
        assert!(error.to_payload(None).recoverable);

        // Copia previa consistente creada antes de migrar.
        assert!(
            data.path()
                .join(BACKUP_DIR)
                .join("pre-migration-v1.sqlite3")
                .exists()
        );

        // El bloqueo se liberó y la versión actual abre la base intacta.
        let store = Store::open(data.path()).await.unwrap();
        assert_eq!(store.schema_version().await.unwrap(), 1);
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

        // 0002 se aplica y confirma (cambia datos y esquema); 0003 falla después.
        let (_src, partial) = migrator_with(&[
            (
                "0002_ok.sql",
                "CREATE TABLE added_by_0002 (x INTEGER);\n\
                 UPDATE mutation_receipts SET operation = 'changed_by_0002';",
            ),
            ("0003_broken.sql", "INSERT INTO missing_table VALUES (1);"),
        ])
        .await;
        let error = Store::open_with(data.path(), &partial).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::StorageMigrationFailed);

        let store = Store::open(data.path()).await.unwrap();
        assert_eq!(store.schema_version().await.unwrap(), 1);
        assert!(!table_exists(&store, "added_by_0002").await);
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
            "0002_future.sql",
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
        assert_eq!(store.schema_version().await.unwrap(), 2);
        assert!(table_exists(&store, "future_feature").await);
        assert_eq!(receipt_count(&store).await, 1);
    }

    #[tokio::test]
    async fn new_database_does_not_create_a_backup() {
        let data = tempfile::tempdir().unwrap();
        let _store = Store::open(data.path()).await.unwrap();

        assert!(!data.path().join(BACKUP_DIR).exists());
    }
}
