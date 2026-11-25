//! Contrato IPC de copias (HU-011): crear, listar, restaurar con el almacén en uso y seguir.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use kanbania_desktop_lib::StorageSlot;
use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager, WebviewUrl, WebviewWindowBuilder};

fn build_app(data_dir: &Path) -> App<MockRuntime> {
    let app = kanbania_desktop_lib::register_commands(mock_builder())
        .build(mock_context(noop_assets()))
        .unwrap();
    let slot = tauri::async_runtime::block_on(StorageSlot::open(data_dir.to_path_buf()));
    app.manage(slot);
    WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .unwrap();
    app
}

fn invoke(app: &App<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    get_ipc_response(
        &app.get_webview_window("main").unwrap(),
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: (if cfg!(windows) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            })
            .parse()
            .unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap())
}

fn titles(app: &App<MockRuntime>, pid: &Value) -> Vec<String> {
    invoke(
        app,
        "list_stories",
        json!({ "projectId": pid, "includeArchived": true }),
    )
    .unwrap()
    .as_array()
    .unwrap()
    .iter()
    .map(|s| s["title"].as_str().unwrap().to_owned())
    .collect()
}

#[test]
fn backup_and_restore_through_ipc_keep_the_app_usable() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());
    let pid = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p", "name": "P" } }),
    )
    .unwrap()["id"]
        .clone();
    invoke(
        &app,
        "create_story",
        json!({ "input": { "requestId": "a", "projectId": pid, "title": "A" } }),
    )
    .unwrap();
    let backup = invoke(&app, "create_backup", json!({})).unwrap();
    assert_eq!(backup["kind"], "manual");
    invoke(
        &app,
        "create_story",
        json!({ "input": { "requestId": "b", "projectId": pid, "title": "B" } }),
    )
    .unwrap();
    let list = invoke(&app, "list_backups", json!({})).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Restaurar con el almacén abierto: el host lo cierra, sustituye y lo reabre.
    let outcome = invoke(
        &app,
        "restore_backup",
        json!({ "source": { "kind": "backup", "id": backup["id"] } }),
    )
    .unwrap();
    assert_eq!(outcome["safetyBackup"]["kind"], "pre_restore");
    assert_eq!(titles(&app, &pid), ["A"]);
    // La app sigue escribiendo con normalidad tras restaurar.
    invoke(
        &app,
        "create_story",
        json!({ "input": { "requestId": "c", "projectId": pid, "title": "C" } }),
    )
    .unwrap();
    assert_eq!(titles(&app, &pid), ["A", "C"]);

    // Errores estructurados y nada cambia.
    let err = invoke(
        &app,
        "restore_backup",
        json!({ "source": { "kind": "backup", "id": "../x" } }),
    )
    .unwrap_err();
    assert_eq!(err["code"], "validation");
    let err = invoke(
        &app,
        "restore_backup",
        json!({ "source": { "kind": "folder", "path": data.path().to_string_lossy() } }),
    )
    .unwrap_err();
    assert_eq!(err["code"], "validation");
    assert_eq!(titles(&app, &pid), ["A", "C"]);
}

#[test]
fn restore_works_when_storage_is_unavailable_but_not_with_another_instance() {
    use kanbania_core::platform::storage::{BackupKind, Store, save_backup_copy};

    // Copia buena guardada fuera, desde otro perfil.
    let good = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let folder = tauri::async_runtime::block_on(async {
        let store = Store::open(good.path()).await.unwrap();
        let backup = store.create_backup(BackupKind::Manual).await.unwrap();
        store.close().await;
        save_backup_copy(good.path(), &backup.id, external.path()).unwrap()
    });
    let source = external.path().join(folder).to_string_lossy().to_string();

    // Base de una versión más reciente: la app arranca sin almacenamiento, pero puede restaurar.
    let data = tempfile::tempdir().unwrap();
    tauri::async_runtime::block_on(async {
        let store = Store::open(data.path()).await.unwrap();
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, \
             execution_time) VALUES (99999, 'futura', 1, x'00', 0)",
        )
        .execute(store.reader())
        .await
        .unwrap();
        store.close().await;
    });
    let app = build_app(data.path());
    let status = invoke(&app, "get_storage_status", json!({})).unwrap();
    assert_eq!(status["status"], "unavailable");
    let outcome = invoke(
        &app,
        "restore_backup",
        json!({ "source": { "kind": "folder", "path": source } }),
    )
    .unwrap();
    assert!(
        outcome["safetyBackup"]["id"].is_string(),
        "la base «futura» quedó guardada"
    );
    let status = invoke(&app, "get_storage_status", json!({})).unwrap();
    assert_eq!(status["status"], "ready");

    // Con otra instancia abierta sobre los mismos datos: storage_locked y nada cambia.
    let locked = tempfile::tempdir().unwrap();
    let other = tauri::async_runtime::block_on(Store::open(locked.path())).unwrap();
    let app = build_app(locked.path());
    let err = invoke(
        &app,
        "restore_backup",
        json!({ "source": { "kind": "folder", "path": source } }),
    )
    .unwrap_err();
    assert_eq!(err["code"], "storage_locked");
    drop(other);
}
