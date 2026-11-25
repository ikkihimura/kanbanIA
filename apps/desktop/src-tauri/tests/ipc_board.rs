//! Contrato IPC del tablero: mover tarjetas y el caso «hecho» que exige aceptar.

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

#[test]
fn move_story_through_ipc() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());
    let pid = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p", "name": "P" } }),
    )
    .unwrap()["id"]
        .clone();
    let story = invoke(
        &app,
        "create_story",
        json!({ "input": { "requestId": "s", "projectId": pid, "title": "T" } }),
    )
    .unwrap();
    let columns = invoke(&app, "list_columns", json!({ "projectId": pid })).unwrap();
    let column = |category: &str| {
        columns
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["category"] == category)
            .unwrap()["id"]
            .clone()
    };

    let moved = invoke(
        &app,
        "move_story",
        json!({ "input": {
            "projectId": pid, "id": story["id"], "expectedRevision": story["revision"],
            "toColumnId": column("active"), "toIndex": 0,
        } }),
    )
    .unwrap();
    assert_eq!(moved["columnId"], column("active"));

    let done = invoke(
        &app,
        "move_story",
        json!({ "input": {
            "projectId": pid, "id": story["id"], "expectedRevision": moved["revision"],
            "toColumnId": column("done"), "toIndex": 0,
        } }),
    )
    .unwrap_err();
    assert_eq!(done["code"], "acceptance_required");

    let stale = invoke(
        &app,
        "move_story",
        json!({ "input": {
            "projectId": pid, "id": story["id"], "expectedRevision": story["revision"],
            "toColumnId": column("review"), "toIndex": 0,
        } }),
    )
    .unwrap_err();
    assert_eq!(stale["code"], "conflict");
    assert_eq!(stale["current"]["columnId"], column("active"));
    let _: Value = stale;
}
