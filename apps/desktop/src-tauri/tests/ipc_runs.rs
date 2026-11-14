//! Contrato IPC de intentos (HU-017): crear, un activo por historia, cancelar, listar y
//! detalle con instantánea inmutable; otro proyecto → no encontrado.

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
fn runs_flow_through_ipc() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());
    let pid = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p", "name": "P" } }),
    )
    .unwrap()["id"]
        .clone();
    let agent = invoke(
        &app,
        "create_agent",
        json!({ "input": {
            "requestId": "a", "name": "Bot", "role": "", "provider": "", "model": "m1",
            "capabilities": [],
            "avatar": { "head": 0, "skin": 0, "eyes": 0, "mouth": 0, "hair": 0,
                        "hairColor": 0, "accessory": 0, "background": 0 },
        } }),
    )
    .unwrap();
    let story = invoke(
        &app,
        "create_story",
        json!({ "input": { "requestId": "s", "projectId": pid, "title": "Original" } }),
    )
    .unwrap();
    let sid = story["id"].clone();
    invoke(
        &app,
        "set_story_agent",
        json!({ "input": { "projectId": pid, "storyId": sid, "agentId": agent["id"] } }),
    )
    .unwrap();

    let first = invoke(
        &app,
        "create_run",
        json!({ "input": {
            "requestId": "r1", "projectId": pid, "storyId": sid, "note": "Ve con cuidado",
        } }),
    )
    .unwrap();
    assert_eq!(first["number"], 1);
    assert_eq!(first["state"], "queued");
    assert_eq!(first["usage"]["inputTokens"], Value::Null);
    assert_eq!(first["usage"]["costUsdMicros"], Value::Null);

    // Un solo intento activo por historia.
    let active = invoke(
        &app,
        "create_run",
        json!({ "input": { "requestId": "r2", "projectId": pid, "storyId": sid } }),
    )
    .unwrap_err();
    assert_eq!(active["code"], "run_active");
    assert_eq!(active["requestId"], "r2");
    assert!(active["message"].as_str().unwrap().contains("Intento #1"));

    let cancelled = invoke(
        &app,
        "cancel_run",
        json!({ "input": { "requestId": "c1", "projectId": pid, "runId": first["id"] } }),
    )
    .unwrap();
    assert_eq!(cancelled["state"], "cancelled");
    assert!(cancelled["endedAt"].is_string());
    let again = invoke(
        &app,
        "cancel_run",
        json!({ "input": { "requestId": "c2", "projectId": pid, "runId": first["id"] } }),
    )
    .unwrap_err();
    assert_eq!(again["code"], "invalid_run_transition");

    // Editar la historia después no altera la instantánea del intento #1.
    let current = invoke(&app, "get_story", json!({ "projectId": pid, "id": sid })).unwrap();
    invoke(
        &app,
        "update_story",
        json!({ "input": {
            "projectId": pid, "id": sid, "expectedRevision": current["revision"],
            "title": "Editada", "description": "Nueva", "priority": "low",
            "criteria": [{ "text": "CA nuevo" }],
        } }),
    )
    .unwrap();
    let second = invoke(
        &app,
        "create_run",
        json!({ "input": { "requestId": "r3", "projectId": pid, "storyId": sid } }),
    )
    .unwrap();
    assert_eq!(second["number"], 2);

    let detail = invoke(
        &app,
        "get_run",
        json!({ "projectId": pid, "runId": first["id"] }),
    )
    .unwrap();
    assert_eq!(detail["snapshot"]["story"]["title"], "Original");
    assert_eq!(detail["snapshot"]["note"], "Ve con cuidado");
    assert_eq!(detail["snapshot"]["schemaVersion"], 1);
    assert_eq!(detail["snapshotIntact"], true);
    assert_eq!(detail["run"]["snapshotSha256"], first["snapshotSha256"]);
    assert_eq!(detail["events"][0]["event"]["kind"], "state_changed");
    assert_eq!(detail["events"][0]["event"]["to"], "queued");
    assert_eq!(detail["events"][1]["event"]["to"], "cancelled");
    assert_eq!(detail["eventsTruncated"], false);
    let detail = invoke(
        &app,
        "get_run",
        json!({ "projectId": pid, "runId": second["id"] }),
    )
    .unwrap();
    assert_eq!(detail["snapshot"]["story"]["title"], "Editada");
    assert_eq!(detail["snapshot"]["criteria"][0]["text"], "CA nuevo");

    let list = invoke(
        &app,
        "list_story_runs",
        json!({ "projectId": pid, "storyId": sid }),
    )
    .unwrap();
    let numbers: Vec<_> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["number"].as_i64().unwrap(), r["state"].clone()))
        .collect();
    assert_eq!(numbers, vec![(2, json!("queued")), (1, json!("cancelled"))]);

    // Otro proyecto: no encontrado en todas las operaciones.
    let other = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p2", "name": "Otro" } }),
    )
    .unwrap()["id"]
        .clone();
    for (cmd, args) in [
        (
            "get_run",
            json!({ "projectId": other, "runId": second["id"] }),
        ),
        (
            "list_story_runs",
            json!({ "projectId": other, "storyId": sid }),
        ),
        (
            "cancel_run",
            json!({ "input": { "requestId": "c3", "projectId": other, "runId": second["id"] } }),
        ),
        (
            "create_run",
            json!({ "input": { "requestId": "r4", "projectId": other, "storyId": sid } }),
        ),
    ] {
        let error = invoke(&app, cmd, args).unwrap_err();
        assert_eq!(error["code"], "not_found", "{cmd}");
    }
}
