//! Contrato IPC de columnas: argumentos camelCase, respuestas y errores estructurados.

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
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap())
}

#[test]
fn column_lifecycle_through_ipc() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());
    let project = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p-1", "name": "Tablero" } }),
    )
    .unwrap();
    let pid = project["id"].as_str().unwrap().to_owned();

    let cols = invoke(&app, "list_columns", json!({ "projectId": pid })).unwrap();
    let cols = cols.as_array().unwrap();
    assert_eq!(cols.len(), 5);
    assert_eq!(cols[0]["category"], "backlog");
    assert_eq!(cols[4]["category"], "done");
    assert_eq!(cols[0]["wipLimit"], Value::Null);
    assert_eq!(cols[0]["cardCount"], 0);

    let qa = invoke(
        &app,
        "create_column",
        json!({ "input": {
            "requestId": "c-1", "projectId": pid, "name": "QA",
            "category": "review", "wipLimit": 3,
        } }),
    )
    .unwrap();
    assert_eq!(qa["position"], 5);

    let updated = invoke(
        &app,
        "update_column",
        json!({ "input": {
            "projectId": pid, "id": qa["id"], "expectedRevision": 1,
            "name": "Calidad", "category": "review", "wipLimit": null,
        } }),
    )
    .unwrap();
    assert_eq!(updated["wipLimit"], Value::Null);

    let moved = invoke(
        &app,
        "reorder_column",
        json!({ "input": {
            "projectId": pid, "id": qa["id"], "expectedRevision": 2, "toIndex": 0,
        } }),
    )
    .unwrap();
    assert_eq!(moved["position"], 0);

    invoke(
        &app,
        "delete_column",
        json!({ "input": { "projectId": pid, "id": qa["id"], "expectedRevision": 3 } }),
    )
    .unwrap();
    let cols = invoke(&app, "list_columns", json!({ "projectId": pid })).unwrap();
    assert_eq!(cols.as_array().unwrap().len(), 5);
}

#[test]
fn column_errors_are_structured() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());
    let project = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p-1", "name": "Errores" } }),
    )
    .unwrap();

    let error = invoke(
        &app,
        "create_column",
        json!({ "input": {
            "requestId": "c-bad", "projectId": project["id"], "name": "X",
            "category": "active", "wipLimit": 0,
        } }),
    )
    .unwrap_err();
    assert_eq!(error["code"], "validation");
    assert_eq!(error["field"], "wipLimit");
    assert_eq!(error["requestId"], "c-bad");

    // Categoría desconocida: la deserialización la rechaza antes de llegar al núcleo.
    assert!(
        invoke(
            &app,
            "create_column",
            json!({ "input": {
                "requestId": "c-2", "projectId": project["id"], "name": "X",
                "category": "blocked", "wipLimit": null,
            } }),
        )
        .is_err()
    );
}
