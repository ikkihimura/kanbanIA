//! Contrato IPC de proyectos y repositorios: nombres de comando y argumentos (camelCase),
//! forma de las respuestas y del `ErrorPayload`.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

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
    let webview = app.get_webview_window("main").unwrap();
    get_ipc_response(
        &webview,
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

fn git_repo(parent: &Path, name: &str) -> PathBuf {
    let root = parent.join(name);
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
    root
}

#[test]
fn project_and_repository_flow_through_ipc() {
    let data = tempfile::tempdir().unwrap();
    let repos = tempfile::tempdir().unwrap();
    let app = build_app(data.path());

    let project = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "req-create-1", "name": "Señal" } }),
    )
    .unwrap();
    assert_eq!(project["name"], "Señal");
    assert_eq!(project["revision"], 1);
    let id = project["id"].as_str().unwrap().to_owned();

    let renamed = invoke(
        &app,
        "rename_project",
        json!({ "input": { "id": id, "expectedRevision": 1, "name": "Señal 2" } }),
    )
    .unwrap();
    assert_eq!(renamed["revision"], 2);

    invoke(&app, "open_project", json!({ "id": id })).unwrap();
    let last = invoke(&app, "get_last_opened_project", json!({})).unwrap();
    assert_eq!(last["id"], id.as_str());

    let repo = git_repo(repos.path(), "mi repo");
    let link = invoke(
        &app,
        "link_repository",
        json!({ "input": {
            "requestId": "req-link-1",
            "projectId": id,
            "path": repo.to_str().unwrap(),
        } }),
    )
    .unwrap();
    assert_eq!(link["status"], "available");
    assert_eq!(link["name"], "mi repo");

    let listed = invoke(&app, "list_repositories", json!({ "projectId": id })).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);

    let summaries = invoke(&app, "list_projects", json!({})).unwrap();
    assert_eq!(summaries[0]["repositoryCount"], 1);

    invoke(
        &app,
        "unlink_repository",
        json!({ "input": {
            "projectId": id,
            "id": link["id"],
            "expectedRevision": link["revision"],
        } }),
    )
    .unwrap();
    let listed = invoke(&app, "list_repositories", json!({ "projectId": id })).unwrap();
    assert!(listed.as_array().unwrap().is_empty());
}

#[test]
fn errors_cross_the_boundary_as_structured_payloads() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());
    let project = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "req-1", "name": "P" } }),
    )
    .unwrap();

    let error = invoke(
        &app,
        "link_repository",
        json!({ "input": {
            "requestId": "req-link-bad",
            "projectId": project["id"],
            "path": "relativa/no/valida",
        } }),
    )
    .unwrap_err();
    assert_eq!(error["code"], "validation");
    assert_eq!(error["field"], "path");
    assert_eq!(error["requestId"], "req-link-bad");
    assert_eq!(error["recoverable"], true);

    let conflict = invoke(
        &app,
        "rename_project",
        json!({ "input": { "id": project["id"], "expectedRevision": 5, "name": "X" } }),
    )
    .unwrap_err();
    assert_eq!(conflict["code"], "conflict");
    assert_eq!(conflict["current"]["name"], "P");
}

#[test]
fn project_commands_report_storage_unavailable() {
    let data = tempfile::tempdir().unwrap();
    let _owner = build_app(data.path());
    let second = build_app(data.path());

    let error = invoke(&second, "list_projects", json!({})).unwrap_err();
    assert_eq!(error["code"], "storage_locked");
}
