//! Contrato IPC de historias: argumentos camelCase, detalle con criterios y conflicto con versión vigente.

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
fn story_lifecycle_and_conflict_through_ipc() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());
    let project = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p-1", "name": "Historias" } }),
    )
    .unwrap();
    let pid = project["id"].as_str().unwrap().to_owned();

    let created = invoke(
        &app,
        "create_story",
        json!({ "input": { "requestId": "s-1", "projectId": pid, "title": "Primera", "priority": "high" } }),
    )
    .unwrap();
    assert_eq!(created["number"], 1);
    assert_eq!(created["priority"], "high");
    assert_eq!(created["archivedAt"], Value::Null);
    let sid = created["id"].as_str().unwrap().to_owned();

    let saved = invoke(
        &app,
        "update_story",
        json!({ "input": {
            "projectId": pid, "id": sid, "expectedRevision": 1,
            "title": "Primera", "description": "**md**", "priority": "low",
            "criteria": [ { "text": "CA uno" }, { "text": "CA dos" } ],
        } }),
    )
    .unwrap();
    assert_eq!(saved["criteria"][1]["number"], 2);
    assert_eq!(saved["revision"], 2);

    // Edición obsoleta: conflicto con la versión vigente completa (incluidos criterios).
    let conflict = invoke(
        &app,
        "update_story",
        json!({ "input": {
            "projectId": pid, "id": sid, "expectedRevision": 1,
            "title": "Obsoleta", "description": "", "priority": "low", "criteria": [],
        } }),
    )
    .unwrap_err();
    assert_eq!(conflict["code"], "conflict");
    assert_eq!(conflict["current"]["description"], "**md**");
    assert_eq!(conflict["current"]["criteria"][0]["text"], "CA uno");

    let detail = invoke(&app, "get_story", json!({ "projectId": pid, "id": sid })).unwrap();
    assert_eq!(detail["description"], "**md**");

    let archived = invoke(
        &app,
        "archive_story",
        json!({ "input": { "projectId": pid, "id": sid, "expectedRevision": 2 } }),
    )
    .unwrap();
    assert_eq!(archived["position"], Value::Null);
    let visible = invoke(
        &app,
        "list_stories",
        json!({ "projectId": pid, "includeArchived": false }),
    )
    .unwrap();
    assert!(visible.as_array().unwrap().is_empty());
    let all = invoke(
        &app,
        "list_stories",
        json!({ "projectId": pid, "includeArchived": true }),
    )
    .unwrap();
    assert_eq!(all[0]["criteriaCount"], 2);

    let restored = invoke(
        &app,
        "restore_story",
        json!({ "input": { "projectId": pid, "id": sid, "expectedRevision": 3 } }),
    )
    .unwrap();
    assert_eq!(restored["position"], 0);
}
