//! HU-011: copias consistentes con WAL y escrituras concurrentes, restauración verificada,
//! copia previa para deshacer, corrupción, esquema incompatible y retención.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use kanbania_core::error::ErrorCode;
use kanbania_core::features::projects::{CreateProjectInput, create_project};
use kanbania_core::features::stories::{CreateStoryInput, create_story};
use kanbania_core::platform::storage::{
    BackupKind, DATABASE_FILE, MAX_KEPT, Store, backup_path, list_backups, restore_backup,
    restore_backup_waiting, save_backup_copy, verify_backup,
};

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

async fn project(store: &Store) -> String {
    create_project(
        store,
        "ui",
        &CreateProjectInput {
            request_id: rid(),
            name: "Copias".into(),
        },
    )
    .await
    .unwrap()
    .id
}

async fn add_story(store: &Store, project: &str, title: &str) {
    create_story(
        store,
        "ui",
        &CreateStoryInput {
            request_id: rid(),
            project_id: project.into(),
            title: title.into(),
            column_id: None,
            priority: None,
        },
    )
    .await
    .unwrap();
}

async fn titles(store: &Store) -> Vec<String> {
    sqlx::query_scalar("SELECT title FROM stories ORDER BY number")
        .fetch_all(store.reader())
        .await
        .unwrap()
}

async fn file_titles(db: &Path) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(db, dir.path().join(DATABASE_FILE)).unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    let t = titles(&store).await;
    store.close().await;
    t
}

#[tokio::test]
async fn backup_is_consistent_while_writes_continue() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    for i in 0..20 {
        add_story(&store, &p, &format!("antes {i}")).await;
    }

    // Escritor concurrente controlado: 200 historias mientras se crea la copia.
    let writer = {
        let store = store.clone();
        let p = p.clone();
        tokio::spawn(async move {
            for i in 0..200 {
                add_story(&store, &p, &format!("durante {i}")).await;
            }
        })
    };
    // La copia empieza con el escritor ya en marcha.
    while titles(&store).await.len() < 30 {
        tokio::task::yield_now().await;
    }
    let info = store.create_backup(BackupKind::Manual).await.unwrap();
    let finished_before_backup = writer.is_finished();
    writer.await.unwrap();

    let db = backup_path(data.path(), &info.id)
        .unwrap()
        .join(DATABASE_FILE);
    let copied = file_titles(&db).await;
    // Instantánea coherente: las 20 previas y un prefijo de las concurrentes, sin huecos.
    assert!(copied.len() >= 30 && copied.len() <= 220);
    if !finished_before_backup {
        assert!(
            copied.len() < 220,
            "la copia no espera a que acaben las escrituras"
        );
    }
    for (i, title) in copied.iter().skip(20).enumerate() {
        assert_eq!(title, &format!("durante {i}"));
    }
    assert_eq!(info.kind, BackupKind::Manual);
    assert!(info.schema_version.is_some());
    assert!(
        !info.excludes.is_empty(),
        "el manifiesto dice qué no incluye"
    );
}

#[tokio::test]
async fn restore_replaces_data_and_the_safety_backup_undoes_it() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    add_story(&store, &p, "A").await;
    let backup = store.create_backup(BackupKind::Manual).await.unwrap();
    add_story(&store, &p, "B").await;

    // Con el almacén abierto, restaurar no puede tomar el bloqueo: nada cambia.
    let source = backup_path(data.path(), &backup.id).unwrap();
    let err = restore_backup(data.path(), &source).await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::StorageLocked);
    assert_eq!(titles(&store).await, ["A", "B"]);
    store.close().await;
    drop(store);

    let outcome = restore_backup(data.path(), &source).await.unwrap();
    let safety = outcome.safety_backup.expect("copia previa");
    assert_eq!(safety.kind, BackupKind::PreRestore);
    let store = Store::open(data.path()).await.unwrap();
    assert_eq!(titles(&store).await, ["A"]);
    store.close().await;
    drop(store);

    // Deshacer: restaurar la copia previa devuelve el estado anterior.
    let undo = backup_path(data.path(), &safety.id).unwrap();
    restore_backup(data.path(), &undo).await.unwrap();
    let store = Store::open(data.path()).await.unwrap();
    assert_eq!(titles(&store).await, ["A", "B"]);
}

#[tokio::test]
async fn safety_backup_keeps_writes_that_were_only_in_the_wal() {
    // Instantánea «en caliente» del directorio (base + WAL sin consolidar), como tras un cierre
    // abrupto. Restaurar encima debe guardar en la copia previa también lo que estaba en el WAL.
    let live = tempfile::tempdir().unwrap();
    let store = Store::open(live.path()).await.unwrap();
    let p = project(&store).await;
    add_story(&store, &p, "en la base").await;
    let backup = store.create_backup(BackupKind::Manual).await.unwrap();
    add_story(&store, &p, "solo en el WAL").await;

    let crashed = tempfile::tempdir().unwrap();
    let wal = live.path().join(format!("{DATABASE_FILE}-wal"));
    assert!(wal.exists(), "el WAL está activo");
    std::fs::copy(
        live.path().join(DATABASE_FILE),
        crashed.path().join(DATABASE_FILE),
    )
    .unwrap();
    std::fs::copy(&wal, crashed.path().join(format!("{DATABASE_FILE}-wal"))).unwrap();
    let source = backup_path(live.path(), &backup.id).unwrap();

    let outcome = restore_backup(crashed.path(), &source).await.unwrap();
    assert!(!crashed.path().join(format!("{DATABASE_FILE}-wal")).exists());
    let reopened = Store::open(crashed.path()).await.unwrap();
    assert_eq!(titles(&reopened).await, ["en la base"]);
    reopened.close().await;
    let safety = backup_path(crashed.path(), &outcome.safety_backup.unwrap().id).unwrap();
    assert_eq!(
        file_titles(&safety.join(DATABASE_FILE)).await,
        ["en la base", "solo en el WAL"]
    );
}

#[tokio::test]
async fn corrupted_incompatible_or_foreign_copies_change_nothing() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    add_story(&store, &p, "intacta").await;
    let backup = store.create_backup(BackupKind::Manual).await.unwrap();
    store.close().await;
    drop(store);
    let original = backup_path(data.path(), &backup.id).unwrap();
    let work = tempfile::tempdir().unwrap();
    let before = std::fs::read(data.path().join(DATABASE_FILE)).unwrap();

    let variant = |name: &str| {
        let dir = work.path().join(name);
        std::fs::create_dir(&dir).unwrap();
        for f in ["manifest.json", DATABASE_FILE] {
            std::fs::copy(original.join(f), dir.join(f)).unwrap();
        }
        dir
    };
    let rehash = |dir: &Path| {
        // Manifiesto coherente con un contenido manipulado: así se prueba la verificación
        // de la base, no solo la del hash.
        let bytes = std::fs::read(dir.join(DATABASE_FILE)).unwrap();
        let sha: String = {
            use sha2::Digest;
            sha2::Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        };
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        manifest["files"][0]["sha256"] = sha.into();
        manifest["files"][0]["bytes"] = bytes.len().into();
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    };

    // 1. Bytes alterados sin actualizar el manifiesto.
    let flipped = variant("flipped");
    let mut bytes = std::fs::read(flipped.join(DATABASE_FILE)).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    std::fs::write(flipped.join(DATABASE_FILE), &bytes).unwrap();
    let err = restore_backup(data.path(), &flipped).await.unwrap_err();
    assert!(err.to_string().contains("dañada"), "{err}");

    // 2. Páginas corruptas con el manifiesto «al día».
    let corrupt = variant("corrupt");
    let mut bytes = std::fs::read(corrupt.join(DATABASE_FILE)).unwrap();
    for b in bytes.iter_mut().skip(4096).take(4096) {
        *b = 0x5a;
    }
    std::fs::write(corrupt.join(DATABASE_FILE), &bytes).unwrap();
    rehash(&corrupt);
    let err = restore_backup(data.path(), &corrupt).await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::Validation, "{err}");

    // 3. Esquema más reciente que esta app.
    let newer = variant("newer");
    {
        let store = {
            let dir = tempfile::tempdir().unwrap();
            std::fs::copy(newer.join(DATABASE_FILE), dir.path().join(DATABASE_FILE)).unwrap();
            let s = Store::open(dir.path()).await.unwrap();
            sqlx::query(
                "INSERT INTO _sqlx_migrations (version, description, success, checksum, \
                 execution_time) VALUES (99999, 'futura', 1, x'00', 0)",
            )
            .execute(s.reader())
            .await
            .unwrap();
            s.close().await;
            std::fs::copy(dir.path().join(DATABASE_FILE), newer.join(DATABASE_FILE)).unwrap();
            dir
        };
        drop(store);
    }
    rehash(&newer);
    let err = restore_backup(data.path(), &newer).await.unwrap_err();
    assert!(err.to_string().contains("más reciente"), "{err}");

    // 4. Una carpeta cualquiera, sin manifiesto.
    let foreign = work.path().join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    let err = restore_backup(data.path(), &foreign).await.unwrap_err();
    assert!(err.to_string().contains("no contiene una copia"), "{err}");

    // 5. Manifiesto de una versión futura del formato.
    let future = variant("future");
    let text = std::fs::read_to_string(future.join("manifest.json")).unwrap();
    std::fs::write(
        future.join("manifest.json"),
        text.replace("\"version\": 1", "\"version\": 2"),
    )
    .unwrap();
    let err = restore_backup(data.path(), &future).await.unwrap_err();
    assert!(err.to_string().contains("más reciente"), "{err}");

    assert_eq!(
        std::fs::read(data.path().join(DATABASE_FILE)).unwrap(),
        before
    );
    assert_eq!(
        list_backups(data.path()).unwrap().len(),
        1,
        "sin copias previas inútiles"
    );
    let store = Store::open(data.path()).await.unwrap();
    assert_eq!(titles(&store).await, ["intacta"]);
}

#[tokio::test]
async fn retention_keeps_the_latest_and_never_touches_foreign_folders() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let foreign = data.path().join("backups").join("mis-cosas");
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(foreign.join("nota.txt"), "no borrar").unwrap();

    let mut ids = Vec::new();
    for _ in 0..(MAX_KEPT + 2) {
        ids.push(store.create_backup(BackupKind::Manual).await.unwrap().id);
    }
    let kept: Vec<String> = list_backups(data.path())
        .unwrap()
        .into_iter()
        .map(|b| b.id)
        .collect();
    assert_eq!(kept.len(), MAX_KEPT);
    assert!(kept.contains(ids.last().unwrap()));
    assert!(!kept.contains(&ids[0]) && !kept.contains(&ids[1]));
    assert!(foreign.join("nota.txt").exists());
}

#[tokio::test]
async fn a_copy_saved_to_another_folder_can_be_restored_from_there() {
    let data = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    add_story(&store, &p, "guardada fuera").await;
    let backup = store.create_backup(BackupKind::Manual).await.unwrap();
    let folder = save_backup_copy(data.path(), &backup.id, external.path()).unwrap();
    let err = save_backup_copy(data.path(), &backup.id, external.path()).unwrap_err();
    assert!(err.to_string().contains("Ya existe"));
    let err = save_backup_copy(data.path(), "../x", external.path()).unwrap_err();
    assert_eq!(err.code(), ErrorCode::Validation);
    store.close().await;
    drop(store);

    // Otro perfil (instalación nueva) restaura desde la carpeta externa.
    let other = tempfile::tempdir().unwrap();
    let outcome = restore_backup(other.path(), &external.path().join(folder))
        .await
        .unwrap();
    assert!(outcome.safety_backup.is_none(), "no había base que guardar");
    let store = Store::open(other.path()).await.unwrap();
    assert_eq!(titles(&store).await, ["guardada fuera"]);
}

fn write_manifest(dir: &Path, schema_version: i64) {
    let bytes = std::fs::read(dir.join(DATABASE_FILE)).unwrap();
    let sha: String = {
        use sha2::Digest;
        sha2::Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let manifest = serde_json::json!({
        "format": "kanbania-backup",
        "version": 1,
        "kind": "manual",
        "createdAt": "2026-01-01T00:00:00.000Z",
        "appVersion": "0.0.1",
        "schemaVersion": schema_version,
        "files": [{ "name": DATABASE_FILE, "bytes": bytes.len(), "sha256": sha }],
        "excludes": []
    });
    std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
}

#[tokio::test]
async fn retention_protects_manual_copies_and_the_copy_being_restored() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let manual = store.create_backup(BackupKind::Manual).await.unwrap();
    let mut automatic = Vec::new();
    for _ in 0..MAX_KEPT {
        automatic.push(
            store
                .create_backup(BackupKind::PreMigration)
                .await
                .unwrap()
                .id,
        );
    }
    store.close().await;
    drop(store);

    // Restaurar la automática más antigua: la copia previa (automática) no la borra.
    let oldest = &automatic[0];
    restore_backup(data.path(), &backup_path(data.path(), oldest).unwrap())
        .await
        .unwrap();
    let ids: Vec<String> = list_backups(data.path())
        .unwrap()
        .into_iter()
        .map(|b| b.id)
        .collect();
    assert!(ids.contains(oldest), "la copia restaurada sigue disponible");
    assert!(
        ids.contains(&manual.id),
        "las automáticas no desplazan a las manuales"
    );
}

#[tokio::test]
async fn a_copy_with_a_tampered_schema_is_rejected() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    add_story(&store, &p, "intacta").await;
    store.close().await;
    drop(store);

    // Copia «ajena» con un trigger que marcaría historias como aceptadas.
    let evil = tempfile::tempdir().unwrap();
    {
        let other = tempfile::tempdir().unwrap();
        let s = Store::open(other.path()).await.unwrap();
        sqlx::query(
            "CREATE TRIGGER auto_accept AFTER UPDATE ON stories BEGIN \
             UPDATE stories SET accepted_at = 'x' WHERE id = NEW.id; END",
        )
        .execute(s.reader())
        .await
        .unwrap();
        let backup = s.create_backup(BackupKind::Manual).await.unwrap();
        let dir = backup_path(other.path(), &backup.id).unwrap();
        for f in ["manifest.json", DATABASE_FILE] {
            std::fs::copy(dir.join(f), evil.path().join(f)).unwrap();
        }
        s.close().await;
    }
    let err = restore_backup(data.path(), evil.path()).await.unwrap_err();
    assert!(err.to_string().contains("esquema"), "{err}");
    let store = Store::open(data.path()).await.unwrap();
    assert_eq!(titles(&store).await, ["intacta"]);
}

#[tokio::test]
async fn an_older_copy_is_restored_and_then_migrated() {
    // Base con todas las migraciones menos la última, como la dejaría una versión anterior.
    let migrations = tempfile::tempdir().unwrap();
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut names: Vec<_> = std::fs::read_dir(&real)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    names.pop();
    for name in &names {
        std::fs::copy(real.join(name), migrations.path().join(name)).unwrap();
    }
    let migrator = sqlx::migrate::Migrator::new(migrations.path())
        .await
        .unwrap();
    let old_version = migrator.iter().map(|m| m.version).max().unwrap();
    let old = tempfile::tempdir().unwrap();
    {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(old.path().join(DATABASE_FILE))
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        migrator.run(&pool).await.unwrap();
        pool.close().await;
    }
    write_manifest(old.path(), old_version);

    let data = tempfile::tempdir().unwrap();
    let outcome = restore_backup(data.path(), old.path()).await.unwrap();
    assert_eq!(outcome.restored_schema_version, old_version);
    let store = Store::open(data.path()).await.unwrap();
    assert!(
        store.schema_version().await.unwrap() > old_version,
        "se migró al abrir"
    );
    let kinds: Vec<BackupKind> = list_backups(data.path())
        .unwrap()
        .into_iter()
        .map(|b| b.kind)
        .collect();
    assert!(kinds.contains(&BackupKind::PreMigration));
}

#[tokio::test]
async fn restore_waits_for_an_in_flight_handle_to_be_released() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    add_story(&store, &p, "A").await;
    let backup = store.create_backup(BackupKind::Manual).await.unwrap();

    // Un «comando» con su copia del almacén escribe en bucle hasta que el pool se cierra.
    let writer = {
        let store = store.clone();
        let p = p.clone();
        tokio::spawn(async move {
            let mut i = 0;
            loop {
                let result = create_story(
                    &store,
                    "ui",
                    &CreateStoryInput {
                        request_id: rid(),
                        project_id: p.clone(),
                        title: format!("w{i}"),
                        column_id: None,
                        priority: None,
                    },
                )
                .await;
                if result.is_err() {
                    // Simula el trabajo que queda tras el error antes de soltar el manejador.
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    return;
                }
                i += 1;
            }
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    store.close().await;
    drop(store);

    let source = backup_path(data.path(), &backup.id).unwrap();
    restore_backup_waiting(data.path(), &source, std::time::Duration::from_secs(3))
        .await
        .unwrap();
    writer.await.unwrap();
    let store = Store::open(data.path()).await.unwrap();
    assert_eq!(
        titles(&store).await,
        ["A"],
        "ninguna escritura llegó a la base restaurada"
    );
}

/// Verificar una copia no necesita cerrar el almacén ni lo toca: así el host puede rechazar una
/// carpeta no válida antes de detener nada (procesos de intentos incluidos).
#[tokio::test]
async fn a_copy_is_verified_with_the_store_open_and_without_changes() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    add_story(&store, &p, "sigue").await;
    let backup = store.create_backup(BackupKind::Manual).await.unwrap();
    let path = backup_path(data.path(), &backup.id).unwrap();
    let version = verify_backup(data.path(), &path).await.unwrap();
    assert_eq!(Some(version), backup.schema_version);

    // Carpeta sin manifiesto y copia con bytes alterados: validación, con el almacén abierto.
    let empty = tempfile::tempdir().unwrap();
    let err = verify_backup(data.path(), empty.path()).await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::Validation, "{err}");
    let flipped = tempfile::tempdir().unwrap();
    for f in ["manifest.json", DATABASE_FILE] {
        std::fs::copy(path.join(f), flipped.path().join(f)).unwrap();
    }
    let mut bytes = std::fs::read(flipped.path().join(DATABASE_FILE)).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    std::fs::write(flipped.path().join(DATABASE_FILE), &bytes).unwrap();
    let err = verify_backup(data.path(), flipped.path())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("dañada"), "{err}");

    // El almacén sigue abierto y con sus datos; no quedan carpetas de trabajo.
    assert_eq!(titles(&store).await, ["sigue"]);
    let leftovers = std::fs::read_dir(data.path().join("backups"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".work-verify")
        })
        .count();
    assert_eq!(leftovers, 0);
}
