//! Pruebas de HU-002 a través de la API pública del núcleo, con SQLite temporal real.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use kanbania_core::error::{CoreError, ErrorCode};
use kanbania_core::platform::idempotency::MutationKey;
use kanbania_core::platform::revision;
use kanbania_core::platform::storage::{DATABASE_FILE, Store, latest_schema_version};
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

// ---------- utilidades ----------

/// Tabla de prueba con el patrón de revisión; los slices reales la tendrán en sus migraciones.
async fn create_fixture_tables(store: &Store) {
    store
        .write(|conn| {
            Box::pin(async move {
                sqlx::query(
                    "CREATE TABLE fixture_items (
                        id INTEGER PRIMARY KEY,
                        title TEXT NOT NULL,
                        revision INTEGER NOT NULL DEFAULT 1
                    ) STRICT",
                )
                .execute(&mut *conn)
                .await?;
                sqlx::query(
                    "CREATE TABLE fixture_children (
                        id INTEGER PRIMARY KEY,
                        item_id INTEGER NOT NULL REFERENCES fixture_items(id)
                    ) STRICT",
                )
                .execute(&mut *conn)
                .await?;
                Ok(())
            })
        })
        .await
        .unwrap();
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Item {
    id: i64,
    title: String,
    revision: i64,
}

async fn load_item(conn: &mut SqliteConnection, id: i64) -> Result<Option<Item>, CoreError> {
    let row: Option<(i64, String, i64)> =
        sqlx::query_as("SELECT id, title, revision FROM fixture_items WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(row.map(|(id, title, revision)| Item {
        id,
        title,
        revision,
    }))
}

async fn insert_item(conn: &mut SqliteConnection, title: &str) -> Result<Item, CoreError> {
    let id = sqlx::query("INSERT INTO fixture_items (title) VALUES (?)")
        .bind(title)
        .execute(&mut *conn)
        .await?
        .last_insert_rowid();
    Ok(Item {
        id,
        title: title.to_owned(),
        revision: 1,
    })
}

async fn rename_item(
    store: &Store,
    id: i64,
    expected_revision: i64,
    title: &str,
) -> Result<(), CoreError> {
    let title = title.to_owned();
    store
        .write(move |conn| {
            Box::pin(async move {
                let rows = sqlx::query(
                    "UPDATE fixture_items SET title = ?, revision = revision + 1 \
                     WHERE id = ? AND revision = ?",
                )
                .bind(&title)
                .bind(id)
                .bind(expected_revision)
                .execute(&mut *conn)
                .await?
                .rows_affected();
                revision::ensure_updated("elemento", rows, load_item(conn, id)).await
            })
        })
        .await
}

#[derive(Debug, Serialize)]
struct CreateItem<'a> {
    title: &'a str,
}

/// Mutación idempotente que cuenta cuántas veces se ejecuta realmente.
async fn create_item_idempotent(
    store: &Store,
    key: &MutationKey,
    title: &str,
    executions: &Arc<AtomicUsize>,
) -> Result<Item, CoreError> {
    let executions = Arc::clone(executions);
    let title_owned = title.to_owned();
    store
        .write_idempotent(key, &CreateItem { title }, move |conn| {
            Box::pin(async move {
                executions.fetch_add(1, Ordering::SeqCst);
                insert_item(conn, &title_owned).await
            })
        })
        .await
}

async fn item_count(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM fixture_items")
        .fetch_one(store.reader())
        .await
        .unwrap()
}

// ---------- CA-1: SQLite en datos de aplicación, WAL, claves foráneas, migraciones ----------

#[tokio::test]
async fn creates_database_with_wal_foreign_keys_and_latest_schema() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(&data.path().join("perfil con espacios ñ"))
        .await
        .unwrap();

    assert_eq!(store.database_path().file_name().unwrap(), DATABASE_FILE);
    assert!(store.database_path().exists());
    assert_eq!(
        store.schema_version().await.unwrap(),
        latest_schema_version()
    );
    assert!(latest_schema_version() >= 2);

    // Cada conexión del pool debe tener la configuración, no solo la primera.
    let mut conns = Vec::new();
    for _ in 0..3 {
        conns.push(store.reader().acquire().await.unwrap());
    }
    for conn in &mut conns {
        let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut **conn)
            .await
            .unwrap();
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut **conn)
            .await
            .unwrap();
        let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&mut **conn)
            .await
            .unwrap();
        assert_eq!(journal, "wal");
        assert_eq!(foreign_keys, 1);
        assert!(busy_timeout > 0);
    }
}

#[tokio::test]
async fn foreign_keys_are_enforced() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    create_fixture_tables(&store).await;

    let result = store
        .write(|conn| {
            Box::pin(async move {
                sqlx::query("INSERT INTO fixture_children (item_id) VALUES (999)")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .await;

    assert!(matches!(result, Err(CoreError::Database(_))));
}

#[tokio::test]
async fn data_survives_restart() {
    let data = tempfile::tempdir().unwrap();
    {
        let store = Store::open(data.path()).await.unwrap();
        create_fixture_tables(&store).await;
        store
            .write(|conn| Box::pin(async move { insert_item(conn, "persistente").await }))
            .await
            .unwrap();
        store.close().await;
    }

    let store = Store::open(data.path()).await.unwrap();
    let item = store
        .write(|conn| Box::pin(async move { load_item(conn, 1).await }))
        .await
        .unwrap();
    assert_eq!(item.map(|i| i.title).as_deref(), Some("persistente"));
}

#[tokio::test]
async fn failed_write_rolls_back_the_whole_transaction() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    create_fixture_tables(&store).await;

    let result: Result<(), CoreError> = store
        .write(|conn| {
            Box::pin(async move {
                insert_item(conn, "no debe quedar").await?;
                Err(CoreError::Validation {
                    field: "title",
                    reason: "fallo simulado".into(),
                })
            })
        })
        .await;

    assert!(result.is_err());
    assert_eq!(item_count(&store).await, 0);
}

// ---------- CA-2: un solo escritor por directorio de datos ----------

#[tokio::test]
async fn second_open_of_same_data_dir_is_rejected_until_first_closes() {
    let data = tempfile::tempdir().unwrap();
    let first = Store::open(data.path()).await.unwrap();

    let error = Store::open(data.path()).await.unwrap_err();
    assert_eq!(error.code(), ErrorCode::StorageLocked);
    assert!(error.to_payload(None).recoverable);

    first.close().await;
    drop(first);
    Store::open(data.path()).await.unwrap();
}

#[tokio::test]
async fn concurrent_opens_produce_exactly_one_owner() {
    let data = tempfile::tempdir().unwrap();
    let path = data.path().to_path_buf();

    let attempts: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            tokio::spawn(async move { Store::open(&path).await })
        })
        .collect();

    let mut owners = Vec::new();
    let mut locked = 0;
    for attempt in attempts {
        match attempt.await.unwrap() {
            Ok(store) => owners.push(store),
            Err(error) => {
                assert_eq!(error.code(), ErrorCode::StorageLocked);
                locked += 1;
            }
        }
    }
    assert_eq!(owners.len(), 1);
    assert_eq!(locked, 7);
}

const HOLDER_ENV: &str = "KANBANIA_TEST_HOLD_DATA_DIR";

/// Proceso auxiliar: abre el almacenamiento, avisa por stdout y lo mantiene hasta que se
/// cierre su stdin. Solo se ejecuta cuando lo lanza `second_process_cannot_become_writer`.
#[tokio::test]
#[ignore = "proceso auxiliar"]
async fn helper_hold_storage() {
    let Some(dir) = std::env::var_os(HOLDER_ENV) else {
        return;
    };
    let _store = Store::open(Path::new(&dir)).await.unwrap();
    println!("HOLDING");
    std::io::stdout().flush().unwrap();
    let mut sink = String::new();
    while std::io::stdin().read_line(&mut sink).unwrap() > 0 {}
}

#[tokio::test]
async fn second_process_cannot_become_writer() {
    let data = tempfile::tempdir().unwrap();
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "helper_hold_storage", "--nocapture"])
        .env(HOLDER_ENV, data.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let stdout = holder.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    assert!(
        lines.any(|line| line.unwrap().contains("HOLDING")),
        "el proceso auxiliar no llegó a abrir el almacenamiento"
    );

    let error = Store::open(data.path()).await.unwrap_err();
    assert_eq!(error.code(), ErrorCode::StorageLocked);

    drop(holder.stdin.take());
    assert!(holder.wait().unwrap().success());

    // Al terminar el primer proceso, el bloqueo queda libre.
    Store::open(data.path()).await.unwrap();
}

// ---------- CA-3: revisión optimista, idempotencia y error estructurado ----------

#[tokio::test]
async fn stale_revision_returns_conflict_with_current_state() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    create_fixture_tables(&store).await;
    store
        .write(|conn| Box::pin(async move { insert_item(conn, "original").await }))
        .await
        .unwrap();

    rename_item(&store, 1, 1, "primera edición").await.unwrap();
    let error = rename_item(&store, 1, 1, "edición obsoleta")
        .await
        .unwrap_err();

    let payload = error.to_payload(Some("req-9"));
    assert_eq!(payload.code, ErrorCode::Conflict);
    assert_eq!(payload.request_id.as_deref(), Some("req-9"));
    let current: Item = serde_json::from_value(payload.current.unwrap()).unwrap();
    assert_eq!(current.title, "primera edición");
    assert_eq!(current.revision, 2);

    let missing = rename_item(&store, 42, 1, "x").await.unwrap_err();
    assert_eq!(missing.code(), ErrorCode::NotFound);
}

#[tokio::test]
async fn same_request_and_payload_returns_original_result_once() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    create_fixture_tables(&store).await;
    let executions = Arc::new(AtomicUsize::new(0));
    let key = MutationKey::new("ui", "fixture.create", "req-1").unwrap();

    let first = create_item_idempotent(&store, &key, "una vez", &executions)
        .await
        .unwrap();
    let retry = create_item_idempotent(&store, &key, "una vez", &executions)
        .await
        .unwrap();

    assert_eq!(first, retry);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert_eq!(item_count(&store).await, 1);
}

#[tokio::test]
async fn same_request_with_different_payload_or_operation_conflicts() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    create_fixture_tables(&store).await;
    let executions = Arc::new(AtomicUsize::new(0));
    let key = MutationKey::new("ui", "fixture.create", "req-1").unwrap();
    create_item_idempotent(&store, &key, "A", &executions)
        .await
        .unwrap();

    let different_payload = create_item_idempotent(&store, &key, "B", &executions)
        .await
        .unwrap_err();
    assert_eq!(different_payload.code(), ErrorCode::IdempotencyConflict);

    let other_operation = MutationKey::new("ui", "fixture.other", "req-1").unwrap();
    let different_op = create_item_idempotent(&store, &other_operation, "A", &executions)
        .await
        .unwrap_err();
    assert_eq!(different_op.code(), ErrorCode::IdempotencyConflict);

    // El mismo request_id de otro actor es independiente.
    let other_actor = MutationKey::new("mcp:session-1", "fixture.create", "req-1").unwrap();
    create_item_idempotent(&store, &other_actor, "A", &executions)
        .await
        .unwrap();

    assert_eq!(executions.load(Ordering::SeqCst), 2);
    assert_eq!(item_count(&store).await, 2);
}

#[tokio::test]
async fn failed_mutation_leaves_no_receipt_so_retry_executes() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    create_fixture_tables(&store).await;
    let key = MutationKey::new("ui", "fixture.create", "req-retry").unwrap();

    let failed: Result<Item, CoreError> = store
        .write_idempotent(&key, &CreateItem { title: "x" }, |conn| {
            Box::pin(async move {
                insert_item(conn, "x").await?;
                Err(CoreError::Validation {
                    field: "title",
                    reason: "fallo simulado".into(),
                })
            })
        })
        .await;
    assert!(failed.is_err());
    assert_eq!(item_count(&store).await, 0);

    let executions = Arc::new(AtomicUsize::new(0));
    create_item_idempotent(&store, &key, "x", &executions)
        .await
        .unwrap();
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn idempotent_replay_survives_restart() {
    let data = tempfile::tempdir().unwrap();
    let key = MutationKey::new("ui", "fixture.create", "req-restart").unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let original = {
        let store = Store::open(data.path()).await.unwrap();
        create_fixture_tables(&store).await;
        let item = create_item_idempotent(&store, &key, "tras reinicio", &executions)
            .await
            .unwrap();
        store.close().await;
        item
    };

    let store = Store::open(data.path()).await.unwrap();
    let replay = create_item_idempotent(&store, &key, "tras reinicio", &executions)
        .await
        .unwrap();

    assert_eq!(replay, original);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

/// Los comandos asíncronos de Tauri y el futuro servicio local exigen futuros `Send`.
#[test]
fn store_futures_are_send() {
    fn assert_send<T: Send>(_: T) {}
    let path = Path::new("unused");
    assert_send(Store::open(path));
    fn _write_is_send(store: &Store, key: &MutationKey) {
        assert_send(store.write(|_| Box::pin(async { Ok(()) })));
        assert_send(store.write_idempotent(key, &(), |_| Box::pin(async { Ok(()) })));
    }
}
