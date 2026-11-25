//! Prueba de contrato: invoca comandos a través del IPC de Tauri (runtime simulado de
//! `tauri::test`), con el mismo registro de comandos que usa la aplicación real.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use serde_json::Value;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{App, WebviewUrl, WebviewWindowBuilder};

/// Origen de la app (en Windows, Tauri sirve la app en `http://tauri.localhost`).
#[cfg(not(windows))]
const LOCAL_ORIGIN: &str = "tauri://localhost";
#[cfg(windows)]
const LOCAL_ORIGIN: &str = "http://tauri.localhost";

fn build_app() -> App<MockRuntime> {
    kanbania_desktop_lib::register_commands(mock_builder())
        .build(mock_context(noop_assets()))
        .unwrap()
}

fn invoke(app: &App<MockRuntime>, cmd: &str, origin: &str) -> Result<Value, Value> {
    let webview = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .build()
        .unwrap();

    get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: origin.parse().unwrap(),
            body: InvokeBody::default(),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap())
}

#[test]
fn get_app_info_returns_typed_payload_through_ipc() {
    let app = build_app();
    let package = app.package_info().clone();

    let value = invoke(&app, "get_app_info", LOCAL_ORIGIN).unwrap();

    assert_eq!(value["productName"], package.name.as_str());
    assert_eq!(value["appVersion"], package.version.to_string());
    assert_eq!(value["coreVersion"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        value["contractVersion"],
        kanbania_core::contract::CONTRACT_VERSION
    );
    assert_eq!(value["os"], std::env::consts::OS);
    assert_eq!(value["arch"], std::env::consts::ARCH);
}

#[test]
fn unknown_command_is_rejected() {
    let app = build_app();

    assert!(invoke(&app, "not_a_command", LOCAL_ORIGIN).is_err());
}

#[test]
fn remote_origin_cannot_reach_app_commands() {
    let app = build_app();

    assert!(invoke(&app, "get_app_info", "https://example.com").is_err());
}
