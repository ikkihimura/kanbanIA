//! Prueba de contrato de los comandos de almacenamiento a través del IPC simulado de Tauri.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use kanbania_desktop_lib::StorageSlot;
use serde_json::Value;
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
    app
}

fn invoke(app: &App<MockRuntime>, cmd: &str) -> Value {
    let webview = match app.get_webview_window("main") {
        Some(webview) => webview,
        None => WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
            .build()
            .unwrap(),
    };
    get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::default(),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap())
    .unwrap()
}

#[test]
fn reports_ready_storage_with_schema_version() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());

    let status = invoke(&app, "get_storage_status");

    assert_eq!(status["status"], "ready");
    assert_eq!(status["schemaVersion"], 1);
    assert!(
        status["databasePath"]
            .as_str()
            .unwrap()
            .ends_with("kanbania.sqlite3")
    );
}

#[test]
fn second_instance_reports_locked_and_recovers_after_first_closes() {
    let data = tempfile::tempdir().unwrap();
    let first = build_app(data.path());
    let second = build_app(data.path());

    let status = invoke(&second, "get_storage_status");
    assert_eq!(status["status"], "unavailable");
    assert_eq!(status["error"]["code"], "storage_locked");
    assert_eq!(status["error"]["recoverable"], true);

    tauri::async_runtime::block_on(first.state::<StorageSlot>().close());

    let status = invoke(&second, "retry_open_storage");
    assert_eq!(status["status"], "ready");
}

#[test]
fn closed_storage_is_not_reopened_by_a_late_retry() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());

    tauri::async_runtime::block_on(app.state::<StorageSlot>().close());
    let status = invoke(&app, "retry_open_storage");
    assert_eq!(status["status"], "unavailable");

    // El bloqueo quedó libre: otro propietario puede abrir los datos.
    let other = build_app(data.path());
    assert_eq!(invoke(&other, "get_storage_status")["status"], "ready");
}
