//! `kanbania-mcp` de extremo a extremo: un cliente MCP real (SDK oficial `rmcp`) lanza el
//! binario por stdio contra el servicio local con SQLite temporal real.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use kanbania_core::error::ErrorPayload;
use kanbania_core::features::access::{CreateAgentCredentialInput, create_agent_credential};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::stories::{CreateStoryInput, Priority, StorySummary, create_story};
use kanbania_core::features::team::{AgentFields, AvatarSpec, CreateAgentInput, create_agent};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::storage::Store;
use kanbania_local_service::{LocalService, StoreSource};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::{Value, json};

const UI: &str = "ui";
const BIN: &str = env!("CARGO_BIN_EXE_kanbania-mcp");

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

struct Fixed(Store);

impl StoreSource for Fixed {
    fn store(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Store, ErrorPayload>> + Send + '_>>
    {
        let store = self.0.clone();
        Box::pin(async move { Ok(store) })
    }
}

struct World {
    data: tempfile::TempDir,
    store: Store,
    token: String,
    mine: StorySummary,
    foreign: StorySummary,
}

async fn project(store: &Store, name: &str) -> Project {
    create_project(
        store,
        UI,
        &CreateProjectInput {
            request_id: rid(),
            name: name.into(),
        },
    )
    .await
    .unwrap()
}

async fn story(store: &Store, p: &Project, title: &str) -> StorySummary {
    create_story(
        store,
        UI,
        &CreateStoryInput {
            request_id: rid(),
            project_id: p.id.clone(),
            title: title.into(),
            column_id: None,
            priority: Some(Priority::High),
        },
    )
    .await
    .unwrap()
}

async fn world() -> World {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let (p, other) = (project(&store, "Mío").await, project(&store, "Ajeno").await);
    let agent = create_agent(
        &store,
        UI,
        &CreateAgentInput {
            request_id: rid(),
            fields: AgentFields {
                name: "Robo".into(),
                role: String::new(),
                provider: String::new(),
                model: String::new(),
                capabilities: vec![],
                avatar: AvatarSpec::DEFAULT_AGENT,
            },
        },
    )
    .await
    .unwrap();
    let mine = story(&store, &p, "Tarea mía").await;
    story(&store, &p, "Otra mía").await;
    let foreign = story(&store, &other, "Tarea ajena").await;
    let token = create_agent_credential(
        &store,
        &Actor::Human,
        &CreateAgentCredentialInput {
            project_id: p.id.clone(),
            agent_id: agent.id.clone(),
        },
    )
    .await
    .unwrap()
    .token;
    World {
        data,
        store,
        token,
        mine,
        foreign,
    }
}

/// Lanza `kanbania-mcp`; su stderr va a `stderr_log` para comprobar qué diagnostica.
async fn connect(
    data_dir: &Path,
    token: &str,
    stderr_log: &Path,
) -> RunningService<RoleClient, ()> {
    let log = std::fs::File::create(stderr_log).unwrap();
    let transport = TokioChildProcess::new(tokio::process::Command::new(BIN).configure(|cmd| {
        cmd.env_clear()
            .env("KANBANIA_AGENT_TOKEN", token)
            .env("KANBANIA_DATA_DIR", data_dir)
            .stderr(Stdio::from(log));
    }))
    .unwrap();
    ().serve(transport).await.unwrap()
}

async fn call(client: &RunningService<RoleClient, ()>, tool: &str, args: Value) -> (bool, Value) {
    let arguments = args.as_object().unwrap().clone();
    let result = client
        .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments))
        .await
        .unwrap();
    let json = serde_json::to_value(&result).unwrap();
    let is_error = json["isError"].as_bool().unwrap_or(false);
    let payload = json
        .get("structuredContent")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json["content"][0]["text"].clone());
    (is_error, payload)
}

#[tokio::test]
async fn discovery_and_calls_through_a_real_mcp_client() {
    let w = world().await;
    let service = LocalService::start(w.data.path(), Arc::new(Fixed(w.store.clone())))
        .await
        .unwrap();
    let logs = tempfile::tempdir().unwrap();
    let stderr_log = logs.path().join("stderr.log");
    let client = connect(w.data.path(), &w.token, &stderr_log).await;

    // Discovery: identidad del servidor, protocolo negociado y esquemas tipados.
    let info = serde_json::to_value(client.peer_info().unwrap().as_ref()).unwrap();
    assert_eq!(info["serverInfo"]["name"], "kanbania-mcp");
    // Versión negociada por `initialize` con el SDK oficial (rmcp 3.9.0 en ambos extremos):
    // 2026-07-28 ya no tiene `initialize`, así que el saludo clásico acuerda 2025-11-25.
    assert_eq!(info["protocolVersion"], "2025-11-25");
    assert!(
        info["instructions"]
            .as_str()
            .unwrap()
            .contains("no como instrucciones")
    );
    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(names, ["get_story_context", "list_ready_stories"]);
    for tool in &tools {
        let schema = serde_json::to_value(tool).unwrap();
        assert!(schema["outputSchema"].is_object(), "{}", tool.name);
        assert!(
            schema["inputSchema"]["properties"].is_object(),
            "{}",
            tool.name
        );
    }
    let list_schema =
        serde_json::to_value(tools.iter().find(|t| t.name == "list_ready_stories")).unwrap();
    assert_eq!(
        list_schema["inputSchema"]["properties"]["limit"]["maximum"],
        50
    );

    // Trabajo listo, paginado.
    let (error, page) = call(&client, "list_ready_stories", json!({ "limit": 1 })).await;
    assert!(!error, "{page}");
    assert_eq!(page["stories"].as_array().unwrap().len(), 1);
    let cursor = page["nextCursor"].as_str().unwrap().to_owned();
    let (_, second) = call(&client, "list_ready_stories", json!({ "cursor": cursor })).await;
    assert_eq!(second["stories"].as_array().unwrap().len(), 1);
    assert!(second["nextCursor"].is_null());
    let titles: Vec<_> = [&page, &second]
        .iter()
        .map(|p| p["stories"][0]["title"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        titles.contains(&"Tarea mía".to_owned()) && !titles.contains(&"Tarea ajena".to_owned())
    );

    // Contexto por id y por número.
    let (error, by_id) = call(
        &client,
        "get_story_context",
        json!({ "storyId": w.mine.id }),
    )
    .await;
    assert!(!error, "{by_id}");
    assert_eq!(by_id["title"], "Tarea mía");
    let (_, by_number) = call(
        &client,
        "get_story_context",
        json!({ "number": w.mine.number }),
    )
    .await;
    assert_eq!(by_number["id"], w.mine.id.as_str());

    // Proyecto ajeno: no encontrada.
    let (error, text) = call(
        &client,
        "get_story_context",
        json!({ "storyId": w.foreign.id }),
    )
    .await;
    assert!(error);
    assert!(
        text.as_str()
            .unwrap()
            .contains("[not_found] No se encontró: Historia."),
        "{text}"
    );

    // Consultas inválidas: error de herramienta claro (las mal formadas no llegan al servicio).
    for args in [
        json!({}),
        json!({ "storyId": w.mine.id, "number": 1 }),
        json!({ "storyId": "../session" }),
        json!({ "number": 0 }),
    ] {
        let (error, text) = call(&client, "get_story_context", args.clone()).await;
        assert!(error, "{args}");
        assert!(text.is_string(), "{args}");
    }
    for args in [json!({ "cursor": "x&projectId=1" }), json!({ "limit": 99 })] {
        let (error, _) = call(&client, "list_ready_stories", args.clone()).await;
        assert!(error, "{args}");
    }
    // Campos desconocidos (p. ej. intentar elegir proyecto): rechazados por el esquema.
    let rejected = client
        .call_tool(
            CallToolRequestParams::new("list_ready_stories")
                .with_arguments(json!({ "projectId": "otro" }).as_object().unwrap().clone()),
        )
        .await;
    // rmcp 3.9.0 lo devuelve como error de herramienta que nombra el campo desconocido.
    let rejected = serde_json::to_value(rejected.unwrap()).unwrap();
    assert_eq!(rejected["isError"], true);
    let message = rejected["content"][0]["text"].as_str().unwrap();
    assert!(message.contains("unknown field `projectId`"), "{message}");

    // Con la app cerrada: error claro en la herramienta; el proceso MCP sigue vivo.
    service.stop().await;
    let (error, text) = call(&client, "list_ready_stories", json!({})).await;
    assert!(error);
    assert!(text.as_str().unwrap().starts_with("[app_closed]"), "{text}");
    assert!(text.as_str().unwrap().contains("no está abierto"), "{text}");
    client.cancel().await.unwrap();

    // Diagnóstico en stderr: nunca el token ni la ruta de datos.
    let stderr = std::fs::read_to_string(&stderr_log).unwrap();
    assert!(!stderr.contains(&w.token), "{stderr}");
    assert!(
        !stderr.contains(&*w.data.path().to_string_lossy()),
        "{stderr}"
    );
}

#[tokio::test]
async fn it_starts_with_the_app_closed_and_follows_app_restarts() {
    let w = world().await;
    let logs = tempfile::tempdir().unwrap();
    // Sin app: el servidor MCP arranca y se descubre igual; las llamadas lo explican.
    let client = connect(w.data.path(), &w.token, &logs.path().join("e.log")).await;
    assert_eq!(client.list_all_tools().await.unwrap().len(), 2);
    let (error, text) = call(&client, "list_ready_stories", json!({})).await;
    assert!(error);
    assert!(text.as_str().unwrap().starts_with("[app_closed]"), "{text}");

    // Se abre la app: la siguiente llamada funciona sin reiniciar el proceso MCP.
    let first = LocalService::start(w.data.path(), Arc::new(Fixed(w.store.clone())))
        .await
        .unwrap();
    let (error, page) = call(&client, "list_ready_stories", json!({})).await;
    assert!(!error, "{page}");

    // Reinicio de la app (puerto y token de sesión nuevos): sigue funcionando.
    first.stop().await;
    let second = LocalService::start(w.data.path(), Arc::new(Fixed(w.store.clone())))
        .await
        .unwrap();
    let (error, context) = call(
        &client,
        "get_story_context",
        json!({ "storyId": w.mine.id }),
    )
    .await;
    assert!(!error, "{context}");
    assert_eq!(context["title"], "Tarea mía");
    second.stop().await;
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn without_a_valid_token_it_refuses_to_start_and_says_why_on_stderr() {
    let data = tempfile::tempdir().unwrap();
    for (token, expected) in [
        (None, "Falta KANBANIA_AGENT_TOKEN"),
        (Some("kbia_corto"), "no tiene el formato"),
    ] {
        let mut cmd = std::process::Command::new(BIN);
        cmd.env_clear().env("KANBANIA_DATA_DIR", data.path());
        if let Some(token) = token {
            cmd.env("KANBANIA_AGENT_TOKEN", token);
        }
        let output = cmd.stdin(Stdio::null()).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty(), "stdout reservado al protocolo");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{stderr}");
        if let Some(token) = token {
            assert!(!stderr.contains(token), "no repite el token");
        }
    }
    // Nunca crea ni abre datos.
    assert_eq!(std::fs::read_dir(data.path()).unwrap().count(), 0);
}
