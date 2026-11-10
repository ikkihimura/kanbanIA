//! Contrato IPC del equipo: perfil de usuario, agentes y asignación.

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
fn team_profile_agents_and_assignment_through_ipc() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());

    let profile = invoke(&app, "get_user_profile", json!({})).unwrap();
    assert_eq!(profile["name"], "Yo");
    assert!(profile["avatar"]["hairColor"].is_number());

    let avatar = json!({
        "head": 2, "skin": 6, "eyes": 4, "mouth": 3, "hair": 6,
        "hairColor": 7, "accessory": 5, "background": 3,
    });
    let agent = invoke(
        &app,
        "create_agent",
        json!({ "input": {
            "requestId": "a-1", "name": "Robota", "role": "Backend",
            "provider": "Anthropic", "model": "claude-opus-5-5",
            "capabilities": ["implement", "test"], "avatar": avatar,
        } }),
    )
    .unwrap();
    assert_eq!(agent["number"], 1);
    assert_eq!(agent["capabilities"], json!(["implement", "test"]));

    let bad = invoke(
        &app,
        "create_agent",
        json!({ "input": {
            "requestId": "a-2", "name": "Otro", "role": "", "provider": "", "model": "",
            "capabilities": ["fly"], "avatar": avatar,
        } }),
    );
    assert!(bad.is_err(), "capacidad fuera del conjunto fijo");

    let pid = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p", "name": "P" } }),
    )
    .unwrap()["id"]
        .clone();
    let sid = invoke(
        &app,
        "create_story",
        json!({ "input": { "requestId": "s", "projectId": pid, "title": "T" } }),
    )
    .unwrap()["id"]
        .clone();
    let rel = invoke(
        &app,
        "set_story_agent",
        json!({ "input": { "projectId": pid, "storyId": sid, "agentId": agent["id"] } }),
    )
    .unwrap();
    assert_eq!(rel["agent"]["name"], "Robota");
    assert_eq!(rel["agent"]["avatar"]["head"], 2);

    let agents = invoke(&app, "list_agents", json!({})).unwrap();
    assert_eq!(agents[0]["assignedStories"], 1);
}
