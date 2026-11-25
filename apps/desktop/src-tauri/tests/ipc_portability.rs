//! Contrato IPC de portabilidad (HU-010): exportar, previsualizar y aplicar desde la app.

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
fn export_preview_and_apply_through_ipc() {
    let source_dir = tempfile::tempdir().unwrap();
    let target_dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let source = build_app(source_dir.path());
    let pid = invoke(
        &source,
        "create_project",
        json!({ "input": { "requestId": "p", "name": "Portátil ñ" } }),
    )
    .unwrap()["id"]
        .clone();
    invoke(
        &source,
        "create_story",
        json!({ "input": { "requestId": "s", "projectId": pid, "title": "Historia 🚀" } }),
    )
    .unwrap();
    let exported = invoke(
        &source,
        "export_project",
        json!({ "input": { "projectId": pid, "destination": out.path().to_string_lossy() } }),
    )
    .unwrap();
    assert_eq!(exported["stories"], 1);
    let package = out
        .path()
        .join(exported["folderName"].as_str().unwrap())
        .to_string_lossy()
        .to_string();

    let target = build_app(target_dir.path());
    let preview = invoke(&target, "preview_import", json!({ "path": package })).unwrap();
    assert_eq!(preview["projectExists"], false);
    assert_eq!(preview["items"][0]["action"], "create");
    let result = invoke(
        &target,
        "apply_import",
        json!({ "input": {
            "path": package,
            "fingerprint": preview["fingerprint"],
            "resolutions": []
        } }),
    )
    .unwrap();
    assert_eq!(result["projectId"], pid);
    assert_eq!(result["created"], 1);
    let stories = invoke(
        &target,
        "list_stories",
        json!({ "projectId": pid, "includeArchived": true }),
    )
    .unwrap();
    assert_eq!(stories[0]["title"], "Historia 🚀");

    // Huella obsoleta → error estructurado y nada cambia.
    let err = invoke(
        &target,
        "apply_import",
        json!({ "input": { "path": package, "fingerprint": "x", "resolutions": [] } }),
    )
    .unwrap_err();
    assert_eq!(err["code"], "validation");
    // Una carpeta que no es un paquete.
    let err = invoke(
        &target,
        "preview_import",
        json!({ "path": out.path().to_string_lossy() }),
    )
    .unwrap_err();
    assert_eq!(err["code"], "validation");
    assert!(
        !err["message"]
            .as_str()
            .unwrap()
            .contains(out.path().to_str().unwrap())
    );
}
