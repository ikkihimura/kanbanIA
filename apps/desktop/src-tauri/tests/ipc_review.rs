//! Contrato IPC de revisión: entrega, aceptación, reapertura, bloqueo y bandeja.

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
fn review_flow_through_ipc() {
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
    let sid = story["id"].clone();
    let detail = invoke(
        &app,
        "update_story",
        json!({ "input": {
            "projectId": pid, "id": sid, "expectedRevision": 1, "title": "T",
            "description": "", "priority": "medium", "criteria": [{ "text": "ok" }],
        } }),
    )
    .unwrap();

    let delivery = invoke(
        &app,
        "submit_delivery",
        json!({ "input": {
            "requestId": "d", "projectId": pid, "storyId": sid, "summary": "Listo",
            "references": [{ "kind": "url", "value": "https://example.com/pr/1" }],
            "sendToReview": true,
        } }),
    )
    .unwrap();
    assert_eq!(delivery["source"], "manual");
    assert_eq!(delivery["references"][0]["status"], "unverified");

    invoke(
        &app,
        "set_story_blocker",
        json!({ "input": { "projectId": pid, "storyId": sid, "reason": "Esperando" } }),
    )
    .unwrap();
    let attention = invoke(&app, "list_attention", json!({ "projectId": pid })).unwrap();
    assert_eq!(attention.as_array().unwrap().len(), 1);
    assert_eq!(attention[0]["pendingDeliveries"], 1);
    assert_eq!(attention[0]["blockerReason"], "Esperando");

    // Revisión vigente tras el envío a revisión.
    let current = invoke(&app, "get_story", json!({ "projectId": pid, "id": sid })).unwrap();
    let accepted = invoke(
        &app,
        "accept_story",
        json!({ "input": {
            "projectId": pid, "storyId": sid, "expectedRevision": current["revision"],
            "satisfiedCriteria": [detail["criteria"][0]["id"]],
        } }),
    )
    .unwrap();
    assert!(accepted["acceptedAt"].is_string());

    let review = invoke(
        &app,
        "get_story_review",
        json!({ "projectId": pid, "storyId": sid }),
    )
    .unwrap();
    assert_eq!(review["history"][0]["kind"], "accepted");
    assert_eq!(review["deliveries"][0]["status"], "accepted");

    let reopened = invoke(
        &app,
        "reopen_story",
        json!({ "input": {
            "projectId": pid, "storyId": sid, "expectedRevision": accepted["revision"],
            "reason": "Regresión",
        } }),
    )
    .unwrap();
    assert_eq!(reopened["acceptedAt"], Value::Null);

    let missing_reason = invoke(
        &app,
        "reopen_story",
        json!({ "input": {
            "projectId": pid, "storyId": sid, "expectedRevision": reopened["revision"],
            "reason": "",
        } }),
    )
    .unwrap_err();
    assert_eq!(missing_reason["field"], "reason");
}
