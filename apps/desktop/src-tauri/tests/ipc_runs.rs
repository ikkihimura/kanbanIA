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

    // Sin supervisor en marcha, iniciar no registra nada y lo dice.
    let refused = invoke(
        &app,
        "start_run",
        json!({ "input": {
            "requestId": "st0", "projectId": pid, "runId": first["id"], "executor": { "kind": "testProcess", "scenario": "success" },
        } }),
    )
    .unwrap_err();
    assert_eq!(refused["code"], "validation");
    assert_eq!(refused["requestId"], "st0");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("vuelve a abrirlo")
    );

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

/// Git para preparar el repositorio de la prueba (no el del runner).
fn git(repo: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=Prueba",
            "-c",
            "user.email=prueba@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// Espera a que el intento llegue a un estado terminal y devuelve el detalle.
fn wait_finished(app: &App<MockRuntime>, pid: &Value, run_id: &Value) -> Value {
    let started = std::time::Instant::now();
    loop {
        let detail = invoke(app, "get_run", json!({ "projectId": pid, "runId": run_id })).unwrap();
        let state = detail["run"]["state"].as_str().unwrap().to_owned();
        if ["completed", "failed", "cancelled", "interrupted"].contains(&state.as_str()) {
            return detail;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "sigue en {state}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// HU-018 por IPC con el supervisor real del host y la propia app como proceso de prueba
/// (`--kanbania-test-process`): iniciar, salida, cambios del worktree, eliminar (limpio y
/// descartando cambios), configuración, aviso `run-change` y proyecto ajeno.
#[test]
fn runs_execute_through_the_host_supervisor() {
    use std::sync::{Arc, Mutex};
    use tauri::Listener;

    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("datos con espacios");
    std::fs::create_dir(&data).unwrap();
    let repo = dir.path().join("repo con espacios");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("a.txt"), "original\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "inicial"]);
    std::fs::write(repo.join("a.txt"), "cambio local sin commit\n").unwrap();

    let app = build_app(&data);
    app.manage(kanbania_desktop_lib::TestProcessProgram(Some(
        env!("CARGO_BIN_EXE_kanbania-desktop").into(),
    )));
    tauri::async_runtime::block_on(kanbania_desktop_lib::start_run_supervisor(app.handle()));
    let events: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = events.clone();
    app.listen_any(kanbania_desktop_lib::RUN_CHANGE_EVENT, move |event| {
        sink.lock()
            .unwrap()
            .push(serde_json::from_str(event.payload()).unwrap());
    });

    let pid = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p", "name": "P" } }),
    )
    .unwrap()["id"]
        .clone();
    invoke(
        &app,
        "link_repository",
        json!({ "input": { "requestId": "l", "projectId": pid, "path": repo.to_str().unwrap() } }),
    )
    .unwrap();
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

    let settings = invoke(&app, "get_run_settings", json!({})).unwrap();
    assert_eq!(settings["maxConcurrency"], 2);
    assert_eq!(settings["timeoutMinutes"], 30);
    let updated = invoke(
        &app,
        "update_run_settings",
        json!({ "input": {
            "expectedRevision": settings["revision"], "maxConcurrency": 3, "timeoutMinutes": 10, "approvalNotifications": false,
        } }),
    )
    .unwrap();
    assert_eq!(updated["maxConcurrency"], 3);
    let out_of_range = invoke(
        &app,
        "update_run_settings",
        json!({ "input": {
            "expectedRevision": updated["revision"], "maxConcurrency": 9, "timeoutMinutes": 10, "approvalNotifications": false,
        } }),
    )
    .unwrap_err();
    assert_eq!(out_of_range["code"], "validation");

    let mut run_ids = Vec::new();
    for (n, scenario) in [(1, "success"), (2, "write")] {
        let story = invoke(
            &app,
            "create_story",
            json!({ "input": { "requestId": format!("s{n}"), "projectId": pid, "title": format!("H{n}") } }),
        )
        .unwrap();
        invoke(
            &app,
            "set_story_agent",
            json!({ "input": { "projectId": pid, "storyId": story["id"], "agentId": agent["id"] } }),
        )
        .unwrap();
        let run = invoke(
            &app,
            "create_run",
            json!({ "input": { "requestId": format!("r{n}"), "projectId": pid, "storyId": story["id"] } }),
        )
        .unwrap();
        let started = invoke(
            &app,
            "start_run",
            json!({ "input": {
                "requestId": format!("st{n}"), "projectId": pid, "runId": run["id"],
                "executor": { "kind": "testProcess", "scenario": scenario },
            } }),
        )
        .unwrap();
        assert!(started["startRequestedAt"].is_string());
        run_ids.push(run["id"].clone());
    }

    let success = wait_finished(&app, &pid, &run_ids[0]);
    assert_eq!(
        success["run"]["state"], "completed",
        "{}",
        success["run"]["endReason"]
    );
    let written = wait_finished(&app, &pid, &run_ids[1]);
    assert_eq!(
        written["run"]["state"], "completed",
        "{}",
        written["run"]["endReason"]
    );

    let output = invoke(
        &app,
        "get_run_output",
        json!({ "projectId": pid, "runId": run_ids[0] }),
    )
    .unwrap();
    let lines = output["lines"].as_array().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l["stream"] == "stdout" && l["text"] == "paso 1 de 3")
    );
    assert!(lines.iter().any(|l| l["stream"] == "stderr"));
    assert!(lines[0]["stream"] == "system");
    assert!(lines[0]["atMs"].is_number());
    assert_eq!(output["earlierDropped"], false);

    // Limpio → se elimina sin forzar.
    let clean = invoke(
        &app,
        "get_worktree_changes",
        json!({ "projectId": pid, "runId": run_ids[0] }),
    )
    .unwrap();
    assert_eq!(
        clean,
        json!({ "missing": false, "changes": 0, "ignored": 0, "complete": true, "commitsAtRisk": false })
    );
    let removed = invoke(
        &app,
        "remove_run_worktree",
        json!({ "input": { "projectId": pid, "runId": run_ids[0], "force": false } }),
    )
    .unwrap();
    assert_eq!(removed["removal"], "clean");
    assert!(!Path::new(removed["path"].as_str().unwrap()).exists());

    // Con cambios → hace falta forzar explícitamente; la rama se conserva.
    let dirty = invoke(
        &app,
        "get_worktree_changes",
        json!({ "projectId": pid, "runId": run_ids[1] }),
    )
    .unwrap();
    assert_eq!(dirty["changes"], 1);
    let refused = invoke(
        &app,
        "remove_run_worktree",
        json!({ "input": { "projectId": pid, "runId": run_ids[1], "force": false } }),
    )
    .unwrap_err();
    assert_eq!(refused["code"], "validation");
    assert_eq!(refused["field"], "force");
    let message = refused["message"].as_str().unwrap();
    assert!(message.contains("1 cambio"), "{message}");
    assert!(!message.contains(dir.path().to_str().unwrap()));
    let worktree_path = written["worktree"]["path"].as_str().unwrap().to_owned();
    assert!(
        Path::new(&worktree_path)
            .join("kanbania-prueba.txt")
            .exists()
    );
    let discarded = invoke(
        &app,
        "remove_run_worktree",
        json!({ "input": { "projectId": pid, "runId": run_ids[1], "force": true } }),
    )
    .unwrap();
    assert_eq!(discarded["removal"], "discarded");
    assert!(!Path::new(&worktree_path).exists());
    let branch = discarded["branch"].as_str().unwrap();
    git(
        &repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    );
    // El repositorio original sigue con su cambio sin commit.
    assert_eq!(
        std::fs::read_to_string(repo.join("a.txt")).unwrap(),
        "cambio local sin commit\n"
    );

    // Avisos con los intentos y sus historias.
    let seen = events.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|e| e["runIds"] == json!([run_ids[0]])
                && e["storyIds"].as_array().unwrap().len() == 1),
        "{seen:?}"
    );

    // Otro proyecto: no encontrado.
    let other = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p2", "name": "Otro" } }),
    )
    .unwrap()["id"]
        .clone();
    for (cmd, args) in [
        (
            "get_run_output",
            json!({ "projectId": other, "runId": run_ids[0] }),
        ),
        (
            "get_worktree_changes",
            json!({ "projectId": other, "runId": run_ids[1] }),
        ),
        (
            "remove_run_worktree",
            json!({ "input": { "projectId": other, "runId": run_ids[1], "force": true } }),
        ),
        (
            "start_run",
            json!({ "input": {
                "requestId": "st9", "projectId": other, "runId": run_ids[0], "executor": { "kind": "testProcess", "scenario": "success" },
            } }),
        ),
    ] {
        let error = invoke(&app, cmd, args).unwrap_err();
        assert_eq!(error["code"], "not_found", "{cmd}: {error}");
    }
    // Un id con otra forma no llega a componer rutas.
    let error = invoke(
        &app,
        "get_run_output",
        json!({ "projectId": pid, "runId": "../../etc" }),
    )
    .unwrap_err();
    assert_eq!(error["code"], "not_found");

    tauri::async_runtime::block_on(
        app.state::<kanbania_desktop_lib::RunSupervisorSlot>()
            .stop(),
    );
    tauri::async_runtime::block_on(app.state::<StorageSlot>().close());
}

/// Restaurar una copia: el supervisor se para (interrumpiendo con su propio motivo) y no vuelve a
/// arrancar por ninguna vía hasta `resume`.
#[test]
fn restoring_suspends_the_supervisor_until_resumed() {
    use kanbania_desktop_lib::RunSupervisorSlot;
    use tauri::async_runtime::block_on;

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("a.txt"), "a\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "inicial"]);
    let data = dir.path().join("datos");
    std::fs::create_dir(&data).unwrap();
    let app = build_app(&data);
    app.manage(kanbania_desktop_lib::TestProcessProgram(Some(
        env!("CARGO_BIN_EXE_kanbania-desktop").into(),
    )));
    block_on(kanbania_desktop_lib::start_run_supervisor(app.handle()));
    let slot = app.state::<RunSupervisorSlot>();
    assert!(block_on(slot.is_running()));

    let pid = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p", "name": "P" } }),
    )
    .unwrap()["id"]
        .clone();
    invoke(
        &app,
        "link_repository",
        json!({ "input": { "requestId": "l", "projectId": pid, "path": repo.to_str().unwrap() } }),
    )
    .unwrap();
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
        json!({ "input": { "requestId": "s", "projectId": pid, "title": "Lenta" } }),
    )
    .unwrap();
    invoke(
        &app,
        "set_story_agent",
        json!({ "input": { "projectId": pid, "storyId": story["id"], "agentId": agent["id"] } }),
    )
    .unwrap();
    let run = invoke(
        &app,
        "create_run",
        json!({ "input": { "requestId": "r", "projectId": pid, "storyId": story["id"] } }),
    )
    .unwrap();
    invoke(
        &app,
        "start_run",
        json!({ "input": { "requestId": "st", "projectId": pid, "runId": run["id"], "executor": { "kind": "testProcess", "scenario": "slow" } } }),
    )
    .unwrap();
    let started = std::time::Instant::now();
    loop {
        let detail = invoke(
            &app,
            "get_run",
            json!({ "projectId": pid, "runId": run["id"] }),
        )
        .unwrap();
        if detail["run"]["state"] == "running" {
            break;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(30));
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Una carpeta que no es una copia válida se rechaza antes de detener nada.
    let not_a_backup = dir.path().join("no es una copia");
    std::fs::create_dir(&not_a_backup).unwrap();
    let refused = invoke(
        &app,
        "restore_backup",
        json!({ "source": { "kind": "folder", "path": not_a_backup.to_str().unwrap() } }),
    )
    .unwrap_err();
    assert_eq!(refused["code"], "validation");
    assert!(block_on(slot.is_running()));
    let detail = invoke(
        &app,
        "get_run",
        json!({ "projectId": pid, "runId": run["id"] }),
    )
    .unwrap();
    assert_eq!(detail["run"]["state"], "running");

    block_on(slot.pause_for_restore());
    assert!(!block_on(slot.is_running()));
    let detail = invoke(
        &app,
        "get_run",
        json!({ "projectId": pid, "runId": run["id"] }),
    )
    .unwrap();
    assert_eq!(detail["run"]["state"], "interrupted");
    assert_eq!(
        detail["run"]["endReason"],
        "Se restauró una copia de los datos: no se puede verificar el proceso."
    );
    // Un reintento o cualquier otra vía no lo arranca mientras dura la restauración.
    block_on(kanbania_desktop_lib::start_run_supervisor(app.handle()));
    assert!(!block_on(slot.is_running()));
    block_on(slot.resume(app.handle()));
    assert!(block_on(slot.is_running()));

    block_on(slot.stop());
    block_on(app.state::<StorageSlot>().close());
}

/// Si el supervisor no llega a arrancar (aquí, sin ejecutable), los intentos de la sesión
/// anterior pasan igual a `interrupted` y sus peticiones de inicio se anulan; iniciar se rechaza.
#[test]
fn the_previous_session_is_recovered_even_if_the_supervisor_cannot_start() {
    use kanbania_core::features::runs::{self, SupervisorToken};
    use kanbania_core::platform::actor::Actor;
    use kanbania_desktop_lib::RunSupervisorSlot;
    use tauri::async_runtime::block_on;

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("a.txt"), "a\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "inicial"]);
    let data = dir.path().join("datos");
    std::fs::create_dir(&data).unwrap();
    let app = build_app(&data);
    let pid = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p", "name": "P" } }),
    )
    .unwrap()["id"]
        .clone();
    invoke(
        &app,
        "link_repository",
        json!({ "input": { "requestId": "l", "projectId": pid, "path": repo.to_str().unwrap() } }),
    )
    .unwrap();
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
    let mut runs_by_title = Vec::new();
    for (n, title) in ["Activa", "En cola"].into_iter().enumerate() {
        let story = invoke(
            &app,
            "create_story",
            json!({ "input": { "requestId": format!("s{n}"), "projectId": pid, "title": title } }),
        )
        .unwrap();
        invoke(
            &app,
            "set_story_agent",
            json!({ "input": { "projectId": pid, "storyId": story["id"], "agentId": agent["id"] } }),
        )
        .unwrap();
        let run = invoke(
            &app,
            "create_run",
            json!({ "input": { "requestId": format!("r{n}"), "projectId": pid, "storyId": story["id"] } }),
        )
        .unwrap();
        runs_by_title.push(run["id"].as_str().unwrap().to_owned());
    }
    // «Sesión anterior»: uno despachado (`preparing`) y otro con petición de inicio, escritos
    // con el núcleo directamente (sin supervisor en esta app todavía).
    let store = block_on(app.state::<StorageSlot>().store()).unwrap();
    let project_id = pid.as_str().unwrap().to_owned();
    for run_id in &runs_by_title {
        block_on(runs::start_run(
            &store,
            &Actor::Human,
            &runs::StartRunInput {
                request_id: format!("st-{}", &run_id[run_id.len() - 8..]),
                project_id: project_id.clone(),
                run_id: run_id.clone(),
                repository_id: None,
                executor: runs::RunExecutor::TestProcess {
                    scenario: runs::TestScenario::Success,
                },
            },
        ))
        .unwrap();
    }
    {
        let token = SupervisorToken::claim(&store).unwrap();
        block_on(runs::begin_dispatch(&store, &token, &runs_by_title[0])).unwrap();
    }
    drop(store);

    app.manage(kanbania_desktop_lib::TestProcessProgram(None));
    block_on(kanbania_desktop_lib::start_run_supervisor(app.handle()));
    assert!(!block_on(app.state::<RunSupervisorSlot>().is_running()));
    let active = invoke(
        &app,
        "get_run",
        json!({ "projectId": pid, "runId": runs_by_title[0] }),
    )
    .unwrap();
    assert_eq!(active["run"]["state"], "interrupted");
    let waiting = invoke(
        &app,
        "get_run",
        json!({ "projectId": pid, "runId": runs_by_title[1] }),
    )
    .unwrap();
    assert_eq!(waiting["run"]["state"], "queued");
    assert_eq!(waiting["run"]["startRequestedAt"], Value::Null);
    // Soltó el token: otro arranque podría reclamarlo.
    let store = block_on(app.state::<StorageSlot>().store()).unwrap();
    drop(SupervisorToken::claim(&store).unwrap());
    // Sin supervisor, iniciar se rechaza sin registrar nada.
    let refused = invoke(
        &app,
        "start_run",
        json!({ "input": {
            "requestId": "st-x", "projectId": pid, "runId": runs_by_title[1], "executor": { "kind": "testProcess", "scenario": "success" },
        } }),
    )
    .unwrap_err();
    assert_eq!(refused["code"], "validation");
    block_on(app.state::<StorageSlot>().close());
}

/// Adaptador ACP mínimo en shell (Unix): responde a `initialize` con la versión indicada y
/// después duerme (la comprobación debe terminar su árbol). Incluye `_meta` con un correo que
/// nunca debe llegar a la UI.
#[cfg(unix)]
fn fake_adapter(dir: &Path, name: &str, version: u32) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nread line\nid=$(printf '%s' \"$line\" | sed 's/.*\"id\":\\([0-9]*\\).*/\\1/')\n\
             printf '{{\"jsonrpc\":\"2.0\",\"id\":%s,\"result\":{{\"protocolVersion\":{version},\
             \"agentInfo\":{{\"name\":\"adaptador-prueba\",\"title\":\"Adaptador de prueba\",\"version\":\"1.2.3\"}},\
             \"authMethods\":[],\"agentCapabilities\":{{\"loadSession\":true,\"promptCapabilities\":{{\"image\":true}},\
             \"_meta\":{{\"cuenta\":\"persona@example.invalid\"}}}}}}}}\\n' \"$id\"\nsleep 30\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn acp_adapter_check_through_ipc() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());

    // Sin ruta y sin adaptador configurado.
    let none = invoke(&app, "check_acp_adapter", json!({})).unwrap_err();
    assert_eq!(none["code"], "validation");
    assert!(
        none["message"]
            .as_str()
            .unwrap()
            .contains("No hay ningún adaptador configurado")
    );
    // Misma validación que la configuración: relativa o inexistente → `validation`, sin la
    // ruta en el mensaje.
    let relative = invoke(
        &app,
        "check_acp_adapter",
        json!({ "path": "bin/claude-agent-acp" }),
    )
    .unwrap_err();
    assert_eq!(relative["code"], "validation");
    let missing_path = data.path().join("no existe");
    let missing = invoke(
        &app,
        "check_acp_adapter",
        json!({ "path": missing_path.to_str().unwrap() }),
    )
    .unwrap_err();
    assert_eq!(missing["code"], "validation");
    assert!(!missing.to_string().contains("no existe\""));

    #[cfg(unix)]
    {
        let base = data.path().join("carpeta con espacios");
        std::fs::create_dir(&base).unwrap();
        let adapter = fake_adapter(&base, "adaptador de prueba", 1);
        let started = std::time::Instant::now();
        let check = invoke(
            &app,
            "check_acp_adapter",
            json!({ "path": adapter.to_str().unwrap() }),
        )
        .unwrap();
        // Solo `initialize`: no espera a que el adaptador termine (duerme 30 s).
        assert!(started.elapsed() < std::time::Duration::from_secs(20));
        assert_eq!(
            check,
            json!({
                "protocolVersion": 1,
                "agentName": "adaptador-prueba",
                "agentTitle": "Adaptador de prueba",
                "agentVersion": "1.2.3",
                "capabilities": ["loadSession", "image"],
            })
        );
        assert!(!check.to_string().contains("example.invalid"));

        // Sin `path`, comprueba la ruta configurada.
        let settings = invoke(&app, "get_run_settings", json!({})).unwrap();
        invoke(
            &app,
            "update_run_settings",
            json!({ "input": {
                "expectedRevision": settings["revision"],
                "maxConcurrency": settings["maxConcurrency"],
                "timeoutMinutes": settings["timeoutMinutes"],
                "claudeAcpPath": adapter.to_str().unwrap(),
                "approvalNotifications": false,
            } }),
        )
        .unwrap();
        let configured = invoke(&app, "check_acp_adapter", json!({})).unwrap();
        assert_eq!(configured["agentName"], "adaptador-prueba");

        // Otra versión de ACP → `validation` con un mensaje fijo, sin la ruta.
        let v2 = fake_adapter(&base, "adaptador v2", 2);
        let error = invoke(
            &app,
            "check_acp_adapter",
            json!({ "path": v2.to_str().unwrap() }),
        )
        .unwrap_err();
        assert_eq!(error["code"], "validation");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("versión de ACP compatible")
        );
        assert!(!error.to_string().contains("carpeta con espacios"));
    }
}

/// Aprobaciones (HU-020) por IPC: listar las que esperan y decidir. El recorrido completo con un
/// agente lo prueban `crates/core/tests/run_approvals.rs` y `crates/runner/tests/acp.rs`.
#[test]
fn approvals_through_ipc() {
    let data = tempfile::tempdir().unwrap();
    let app = build_app(data.path());
    let pid = invoke(
        &app,
        "create_project",
        json!({ "input": { "requestId": "p", "name": "P" } }),
    )
    .unwrap()["id"]
        .clone();
    let waiting = invoke(&app, "list_waiting_approvals", json!({ "projectId": pid })).unwrap();
    assert_eq!(waiting, json!([]));
    let missing = invoke(
        &app,
        "decide_approval",
        json!({ "input": {
            "requestId": "d1", "projectId": pid,
            "approvalId": "01a1e1aa-0000-7000-8000-000000000000", "decision": "allow_once",
        } }),
    )
    .unwrap_err();
    assert_eq!(missing["code"], "not_found");
    assert_eq!(missing["requestId"], "d1");
    // Una decisión desconocida no se deserializa (nunca «permitir siempre»).
    let always = invoke(
        &app,
        "decide_approval",
        json!({ "input": {
            "requestId": "d2", "projectId": pid,
            "approvalId": "01a1e1aa-0000-7000-8000-000000000000", "decision": "allow_always",
        } }),
    );
    assert!(always.is_err());
    let unknown_project = invoke(
        &app,
        "list_waiting_approvals",
        json!({ "projectId": "no-existe" }),
    )
    .unwrap_err();
    assert_eq!(unknown_project["code"], "not_found");
}
