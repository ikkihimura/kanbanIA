//! Contrato IPC de épicas y relaciones: argumentos, avance y error de ciclo.

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

fn create(app: &App<MockRuntime>, cmd: &str, input: Value) -> Value {
    invoke(app, cmd, json!({ "input": input })).unwrap()
}

#[test]
fn epics_and_relations_through_ipc() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());
    let pid = create(
        &app,
        "create_project",
        json!({ "requestId": "p", "name": "Rel" }),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let epic = create(
        &app,
        "create_epic",
        json!({ "requestId": "e", "projectId": pid, "title": "Pagos" }),
    );
    assert_eq!(epic["number"], 1);
    assert_eq!(epic["progress"]["percent"], Value::Null, "sin historias");

    let story = |n: &str| {
        create(
            &app,
            "create_story",
            json!({ "requestId": n, "projectId": pid, "title": n }),
        )["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let (a, b) = (story("a"), story("b"));

    let rel = invoke(
        &app,
        "set_story_epic",
        json!({ "input": { "projectId": pid, "storyId": a, "epicId": epic["id"] } }),
    )
    .unwrap();
    assert_eq!(rel["epic"]["title"], "Pagos");

    let rel = invoke(
        &app,
        "add_story_dependency",
        json!({ "input": { "projectId": pid, "storyId": a, "dependsOnId": b } }),
    )
    .unwrap();
    assert_eq!(rel["pendingPrerequisites"], 1);
    assert_eq!(rel["prerequisites"][0]["state"], "pending");

    let cycle = invoke(
        &app,
        "add_story_dependency",
        json!({ "input": { "projectId": pid, "storyId": b, "dependsOnId": a } }),
    )
    .unwrap_err();
    assert_eq!(cycle["code"], "cycle");
    assert_eq!(cycle["field"], "dependsOnId");

    let rel = invoke(
        &app,
        "set_story_parent",
        json!({ "input": { "projectId": pid, "storyId": b, "parentId": a } }),
    )
    .unwrap();
    assert_eq!(rel["parent"]["id"], a.as_str());

    let rel = invoke(
        &app,
        "get_story_relations",
        json!({ "projectId": pid, "storyId": a }),
    )
    .unwrap();
    assert_eq!(rel["children"][0]["id"], b.as_str());

    let rel = invoke(
        &app,
        "remove_story_dependency",
        json!({ "input": { "projectId": pid, "storyId": a, "dependsOnId": b } }),
    )
    .unwrap();
    assert_eq!(rel["pendingPrerequisites"], 0);

    let epics = invoke(&app, "list_epics", json!({ "projectId": pid })).unwrap();
    assert_eq!(epics[0]["progress"]["total"], 1);
    let summaries = invoke(
        &app,
        "list_stories",
        json!({ "projectId": pid, "includeArchived": false }),
    )
    .unwrap();
    let a_summary = summaries
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == a.as_str())
        .unwrap();
    assert_eq!(a_summary["epicId"], epic["id"]);
}
