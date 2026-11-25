//! Servicio local en el host (HU-012): credenciales por IPC, API sobre el almacén del host y
//! competencia entre dos arranques con el mismo directorio de datos.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use kanbania_desktop_lib::{ServiceSlot, StorageSlot, start_local_service};
use kanbania_local_service::{ClientError, ServiceClient, descriptor};
use serde_json::{Value, json};
use tauri::async_runtime::block_on;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager, WebviewUrl, WebviewWindowBuilder};

/// Arranque como en `run()`: abre el almacén y, si lo posee, publica el servicio.
fn launch(data_dir: &Path) -> App<MockRuntime> {
    let app = kanbania_desktop_lib::register_commands(mock_builder())
        .build(mock_context(noop_assets()))
        .unwrap();
    let slot = block_on(StorageSlot::open(data_dir.to_path_buf()));
    app.manage(slot);
    block_on(start_local_service(app.handle()));
    WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .unwrap();
    app
}

/// Salida como en `RunEvent::Exit`: para el servicio y cierra el almacén.
fn shut_down(app: &App<MockRuntime>) {
    block_on(app.state::<ServiceSlot>().stop());
    block_on(app.state::<StorageSlot>().close());
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

/// Proyecto + agente + credencial creados por IPC, como desde la UI.
fn project_with_credential(app: &App<MockRuntime>, name: &str) -> (Value, Value) {
    let project = invoke(
        app,
        "create_project",
        json!({ "input": { "requestId": format!("p-{name}"), "name": name } }),
    )
    .unwrap();
    let agent = invoke(
        app,
        "create_agent",
        json!({ "input": {
            "requestId": format!("a-{name}"), "name": format!("Robo {name}"), "role": "",
            "provider": "", "model": "", "capabilities": [],
            "avatar": { "head": 0, "skin": 0, "eyes": 0, "mouth": 0, "hair": 0,
                        "hairColor": 0, "accessory": 0, "background": 0 },
        } }),
    )
    .unwrap();
    let created = invoke(
        app,
        "create_agent_credential",
        json!({ "input": { "projectId": project["id"], "agentId": agent["id"] } }),
    )
    .unwrap();
    (project, created)
}

#[test]
fn credentials_from_the_app_open_the_local_service_for_their_project() {
    let data = tempfile::tempdir().unwrap();
    let app = launch(data.path());
    let status = invoke(&app, "get_local_service_status", json!({})).unwrap();
    assert_eq!(status["running"], true);
    let port = status["port"].as_u64().unwrap();
    assert_eq!(u64::from(descriptor::read(data.path()).unwrap().port), port);

    let (project, created) = project_with_credential(&app, "Uno");
    let token = created["token"].as_str().unwrap().to_owned();
    let listed = invoke(
        &app,
        "list_agent_credentials",
        json!({ "projectId": project["id"] }),
    )
    .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(
        !listed.to_string().contains(&token),
        "la lista no lleva el token"
    );

    // La API usa el almacén del host (el mismo que la UI): ve el proyecto recién creado.
    let client = ServiceClient::new(data.path(), token);
    let session = block_on(client.get_json("/v1/session")).unwrap();
    assert_eq!(session["projectId"], project["id"]);
    assert_eq!(session["agentName"], "Robo Uno");

    // Revocar desde la app corta el acceso.
    invoke(
        &app,
        "revoke_agent_credential",
        json!({ "input": { "projectId": project["id"], "id": created["credential"]["id"] } }),
    )
    .unwrap();
    let error = block_on(client.get_json("/v1/session")).unwrap_err();
    assert!(
        matches!(&error, ClientError::Api { status: 401, code, .. } if code == "unauthorized"),
        "{error:?}"
    );

    shut_down(&app);
    assert!(!descriptor::path(data.path()).exists());
    let error = block_on(client.get_json("/v1/session")).unwrap_err();
    assert!(matches!(error, ClientError::NotRunning), "{error:?}");
}

#[test]
fn a_second_launch_never_starts_another_service_or_owner() {
    let data = tempfile::tempdir().unwrap();
    let first = launch(data.path());
    let (_, created) = project_with_credential(&first, "Uno");
    let token = created["token"].as_str().unwrap().to_owned();
    let first_port = descriptor::read(data.path()).unwrap().port;

    // Segundo arranque: sin almacén (bloqueado), sin servicio, sin tocar el descriptor.
    let second = launch(data.path());
    let status = invoke(&second, "get_storage_status", json!({})).unwrap();
    assert_eq!(status["status"], "unavailable", "{status}");
    assert_eq!(status["error"]["code"], "storage_locked");
    let service = invoke(&second, "get_local_service_status", json!({})).unwrap();
    assert_eq!(service, json!({ "running": false }));
    assert_eq!(descriptor::read(data.path()).unwrap().port, first_port);
    let client = ServiceClient::new(data.path(), token);
    block_on(client.get_json("/v1/session")).unwrap();

    // Al cerrar la primera, la segunda puede reintentar y pasa a ser la propietaria.
    shut_down(&first);
    assert!(matches!(
        block_on(client.get_json("/v1/session")).unwrap_err(),
        ClientError::NotRunning
    ));
    let status = invoke(&second, "retry_open_storage", json!({})).unwrap();
    assert_eq!(status["status"], "ready", "{status}");
    let service = invoke(&second, "get_local_service_status", json!({})).unwrap();
    assert_eq!(service["running"], true);
    assert_ne!(descriptor::read(data.path()).unwrap().port, 0);
    let session = block_on(client.get_json("/v1/session")).unwrap();
    assert_eq!(session["agentName"], "Robo Uno");
    shut_down(&second);
}

#[test]
fn the_service_survives_a_restore_and_revocations_survive_it_too() {
    let data = tempfile::tempdir().unwrap();
    let app = launch(data.path());
    let (project, leaked) = project_with_credential(&app, "Uno");
    let kept = invoke(
        &app,
        "create_agent_credential",
        json!({ "input": { "projectId": project["id"], "agentId": leaked["credential"]["agentId"] } }),
    )
    .unwrap();
    let backup = invoke(&app, "create_backup", json!({})).unwrap();
    invoke(
        &app,
        "revoke_agent_credential",
        json!({ "input": { "projectId": project["id"], "id": leaked["credential"]["id"] } }),
    )
    .unwrap();
    let port = descriptor::read(data.path()).unwrap().port;

    invoke(
        &app,
        "restore_backup",
        json!({ "source": { "kind": "backup", "id": backup["id"] } }),
    )
    .unwrap();

    // Mismo servicio (mismo puerto), atendiendo con la base restaurada.
    assert_eq!(descriptor::read(data.path()).unwrap().port, port);
    let status = invoke(&app, "get_local_service_status", json!({})).unwrap();
    assert_eq!(status["running"], true);
    let kept = ServiceClient::new(data.path(), kept["token"].as_str().unwrap().to_owned());
    block_on(kept.get_json("/v1/session")).unwrap();
    // La credencial revocada después de la copia no revive.
    let leaked = ServiceClient::new(data.path(), leaked["token"].as_str().unwrap().to_owned());
    let error = block_on(leaked.get_json("/v1/session")).unwrap_err();
    assert!(
        matches!(&error, ClientError::Api { status: 401, .. }),
        "{error:?}"
    );
    // Salida con un reintento tardío en medio: el servicio ya parado no vuelve a publicarse
    // aunque el almacén siga abierto un instante (se cierra después).
    block_on(app.state::<ServiceSlot>().stop());
    block_on(start_local_service(app.handle()));
    assert!(!descriptor::path(data.path()).exists());
    block_on(app.state::<StorageSlot>().close());
}

#[test]
fn the_app_sees_agent_claims_and_a_person_releases_them() {
    let data = tempfile::tempdir().unwrap();
    let app = launch(data.path());
    let (project, created) = project_with_credential(&app, "Uno");
    let story = invoke(
        &app,
        "create_story",
        json!({ "input": { "requestId": "s-1", "projectId": project["id"], "title": "Login" } }),
    )
    .unwrap();
    let client = ServiceClient::new(data.path(), created["token"].as_str().unwrap().to_owned());

    // Un agente reclama por el servicio local; la app lo ve con su dueño y la asignación.
    let claim = block_on(client.post_json(
        "/v1/claims",
        &json!({ "requestId": "c-1", "storyId": story["id"] }),
    ))
    .unwrap();
    let active = invoke(
        &app,
        "list_active_claims",
        json!({ "projectId": project["id"] }),
    )
    .unwrap();
    assert_eq!(active.as_array().unwrap().len(), 1);
    assert_eq!(active[0]["claimId"], claim["claimId"]);
    assert_eq!(active[0]["owner"]["agentName"], "Robo Uno");
    let stories = invoke(
        &app,
        "list_stories",
        json!({ "projectId": project["id"], "includeArchived": false }),
    )
    .unwrap();
    assert_eq!(stories[0]["agentId"], claim["owner"]["agentId"]);

    // La persona lo libera: desaparece de la app, se quita la asignación y el agente ya no
    // puede renovarlo.
    invoke(
        &app,
        "release_claim",
        json!({ "input": { "projectId": project["id"], "claimId": claim["claimId"] } }),
    )
    .unwrap();
    let active = invoke(
        &app,
        "list_active_claims",
        json!({ "projectId": project["id"] }),
    )
    .unwrap();
    assert_eq!(active, json!([]));
    let stories = invoke(
        &app,
        "list_stories",
        json!({ "projectId": project["id"], "includeArchived": false }),
    )
    .unwrap();
    assert_eq!(stories[0]["agentId"], Value::Null);
    let error = block_on(client.post_json(
        &format!("/v1/claims/{}/renew", claim["claimId"].as_str().unwrap()),
        &json!({ "requestId": "r-1" }),
    ))
    .unwrap_err();
    assert!(
        matches!(&error, ClientError::Api { status: 409, code, .. } if code == "claim_ended"),
        "{error:?}"
    );
    // Reclamo de otro proyecto o inexistente → not_found.
    let error = invoke(
        &app,
        "release_claim",
        json!({ "input": { "projectId": project["id"], "claimId": "no-existe" } }),
    )
    .unwrap_err();
    assert_eq!(error["code"], "not_found");
    shut_down(&app);
}

#[test]
fn agent_reports_reach_the_app_as_events_and_reported_activity() {
    use std::sync::{Arc, Mutex};
    use tauri::Listener;

    let data = tempfile::tempdir().unwrap();
    let app = launch(data.path());
    let (project, created) = project_with_credential(&app, "Uno");
    let (other, _) = project_with_credential(&app, "Dos");
    let story = invoke(
        &app,
        "create_story",
        json!({ "input": { "requestId": "s-1", "projectId": project["id"], "title": "API" } }),
    )
    .unwrap();
    let events: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = events.clone();
    app.listen_any("agent-change", move |event| {
        sink.lock()
            .unwrap()
            .push(serde_json::from_str(event.payload()).unwrap());
    });
    let client = ServiceClient::new(data.path(), created["token"].as_str().unwrap().to_owned());

    let claim = block_on(client.post_json(
        "/v1/claims",
        &json!({ "requestId": "c-1", "storyId": story["id"] }),
    ))
    .unwrap();
    let claim_id = claim["claimId"].as_str().unwrap();
    block_on(client.post_json(
        &format!("/v1/claims/{claim_id}/blocker"),
        &json!({ "requestId": "b-1", "reason": "Falta la clave" }),
    ))
    .unwrap();
    let delivered = block_on(client.post_json(
        &format!("/v1/claims/{claim_id}/result"),
        &json!({ "requestId": "r-1", "summary": "Hecho", "references": [] }),
    ))
    .unwrap();
    assert_eq!(delivered["deliveryNumber"], 1);

    // Un aviso por operación confirmada, con el proyecto de la sesión y la historia.
    let seen = events.lock().unwrap().clone();
    assert_eq!(seen.len(), 3, "{seen:?}");
    for event in &seen {
        assert_eq!(event["projectId"], project["id"]);
        assert_eq!(event["storyIds"], json!([story["id"]]));
    }
    // Una operación fallida (reclamo ya cerrado) no avisa.
    assert!(
        block_on(client.post_json(
            &format!("/v1/claims/{claim_id}/blocker"),
            &json!({ "requestId": "b-2", "reason": "Otra" }),
        ))
        .is_err()
    );
    assert_eq!(events.lock().unwrap().len(), 3);

    // La app lee la actividad reportada, la más reciente primero.
    let activity = invoke(
        &app,
        "list_story_activity",
        json!({ "input": { "projectId": project["id"], "storyId": story["id"] } }),
    )
    .unwrap();
    let kinds: Vec<&str> = activity["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    // Entregar quita el bloqueo que reportó el propio agente.
    assert_eq!(
        kinds,
        [
            "delivered",
            "blocker_cleared",
            "blocker_reported",
            "claimed"
        ]
    );
    assert_eq!(activity["entries"][0]["detail"], "E1");
    assert_eq!(activity["entries"][0]["agent"]["agentName"], "Robo Uno");
    assert_eq!(activity["lastReportedAt"], activity["entries"][0]["at"]);
    // Con otro proyecto, la historia no existe.
    let error = invoke(
        &app,
        "list_story_activity",
        json!({ "input": { "projectId": other["id"], "storyId": story["id"] } }),
    )
    .unwrap_err();
    assert_eq!(error["code"], "not_found");
    shut_down(&app);
}
