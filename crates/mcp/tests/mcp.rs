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
    project_id: String,
    other_project_id: String,
    agent_id: String,
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
        project_id: p.id.clone(),
        other_project_id: other.id.clone(),
        agent_id: agent.id.clone(),
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
        // En Windows, sin `SystemRoot` Winsock no se inicializa y no hay conexión posible.
        if let Some(root) = std::env::var_os("SystemRoot") {
            cmd.env("SystemRoot", root);
        }
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
    assert_eq!(
        names,
        [
            "claim_story",
            "create_subtask",
            "get_story_context",
            "list_ready_stories",
            "renew_claim",
            "report_blocker",
            "submit_result"
        ]
    );
    for tool in &tools {
        let schema = serde_json::to_value(tool).unwrap();
        assert!(schema["outputSchema"].is_object(), "{}", tool.name);
        assert!(
            schema["inputSchema"]["properties"].is_object(),
            "{}",
            tool.name
        );
    }
    // Las cifras de las descripciones son las del núcleo (el MCP no lo enlaza: van escritas).
    let described = |name: &str| {
        tools
            .iter()
            .find(|t| t.name == name)
            .and_then(|t| t.description.clone())
            .unwrap()
            .to_string()
    };
    let minutes = format!(
        "{} minutos",
        kanbania_core::features::claims::CLAIM_DURATION_MINUTES
    );
    assert!(described("claim_story").contains(&minutes));
    assert!(described("renew_claim").contains(&minutes));
    assert!(info["instructions"].as_str().unwrap().contains(&minutes));
    let limit = format!(
        "ya tienes {} reclamadas",
        kanbania_core::features::claims::MAX_OPEN_CLAIMS
    );
    assert!(described("claim_story").contains(&limit));
    let subtasks = format!(
        "Como mucho {} subtareas",
        kanbania_core::features::agent_reports::MAX_SUBTASKS_PER_STORY
    );
    assert!(described("create_subtask").contains(&subtasks));
    let pending = format!(
        "y {} subtareas creadas por agentes",
        kanbania_core::features::agent_reports::MAX_PENDING_AGENT_SUBTASKS
    );
    assert!(described("create_subtask").contains(&pending));
    assert!(described("create_subtask").contains("Un solo nivel"));
    // Causas de [claim_ended], también la credencial revocada y la entrega.
    for tool in [
        "renew_claim",
        "report_blocker",
        "create_subtask",
        "submit_result",
    ] {
        let text = described(tool);
        for cause in [
            "venció",
            "entregaste",
            "liberó o reasignó",
            "archivó o aceptó",
            "revocó",
        ] {
            assert!(text.contains(cause), "{tool}: {cause}");
        }
    }
    assert!(described("report_blocker").contains("solo una persona puede quitarlo"));
    assert!(described("submit_result").contains("64 KB"));
    assert!(described("list_ready_stories").contains("entrega pendiente"));
    assert!(described("get_story_context").contains("personas u otros agentes"));
    // La descripción escribe la cifra con separador de miles: ligada a la constante.
    assert_eq!(
        kanbania_core::features::agent_reports::MAX_SUBTASK_DESCRIPTION_CHARS,
        10_000
    );
    assert!(described("create_subtask").contains("hasta 10 000"));
    // Lo que el agente debe saber antes de entregar: revisión humana, nunca autoaceptación.
    let submit = described("submit_result");
    for phrase in [
        "pendiente de revisión humana",
        "nunca aceptas",
        "cierra tu reclamo",
        "reportada por ti",
        "reclamo vigente",
    ] {
        assert!(submit.contains(phrase), "{phrase}: {submit}");
    }
    for tool in ["report_blocker", "create_subtask"] {
        assert!(described(tool).contains("reclamo vigente"), "{tool}");
    }
    let instructions = info["instructions"].as_str().unwrap();
    assert!(instructions.contains("pendiente de revisión humana"));
    assert!(instructions.contains("nunca aceptas"));
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

    // Reclamar y renovar (HU-014).
    let request_id = rid();
    let (error, claim) = call(
        &client,
        "claim_story",
        json!({ "requestId": request_id, "number": w.mine.number }),
    )
    .await;
    assert!(!error, "{claim}");
    assert_eq!(claim["storyId"], w.mine.id.as_str());
    assert_eq!(claim["owner"]["agentName"], "Robo");
    let (_, context) = call(
        &client,
        "get_story_context",
        json!({ "storyId": w.mine.id }),
    )
    .await;
    assert_eq!(context["yourClaim"]["claimId"], claim["claimId"]);
    let (error, renewed) = call(
        &client,
        "renew_claim",
        json!({ "claimId": claim["claimId"], "requestId": rid() }),
    )
    .await;
    assert!(!error, "{renewed}");
    assert!(renewed["renewedAt"].is_string());
    // Mismo requestId para otra historia: conflicto, explicado al agente.
    let (error, text) = call(
        &client,
        "claim_story",
        json!({ "requestId": request_id, "storyId": w.foreign.id }),
    )
    .await;
    assert!(error);
    assert!(
        text.as_str().unwrap().starts_with("[idempotency_conflict]"),
        "{text}"
    );
    for args in [
        json!({ "storyId": w.mine.id }),
        json!({ "requestId": "con espacio", "storyId": w.mine.id }),
        json!({ "requestId": rid(), "storyId": "../session" }),
        json!({ "requestId": rid(), "storyId": w.mine.id, "agentId": "otro" }),
    ] {
        let (error, _) = call(&client, "claim_story", args.clone()).await;
        assert!(error, "{args}");
    }
    let (error, _) = call(
        &client,
        "renew_claim",
        json!({ "claimId": "../../v1/session", "requestId": rid() }),
    )
    .await;
    assert!(error);

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
    assert_eq!(client.list_all_tools().await.unwrap().len(), 7);
    let (error, text) = call(&client, "list_ready_stories", json!({})).await;
    assert!(error);
    assert!(text.as_str().unwrap().starts_with("[app_closed]"), "{text}");
    // Un reclamo con la app cerrada falla sin efecto; el agente lo reintenta con el mismo id.
    let claim_args = json!({ "requestId": rid(), "storyId": w.mine.id });
    let (error, text) = call(&client, "claim_story", claim_args.clone()).await;
    assert!(error);
    assert!(text.as_str().unwrap().starts_with("[app_closed]"), "{text}");

    // Se abre la app: la siguiente llamada funciona sin reiniciar el proceso MCP.
    let first = LocalService::start(w.data.path(), Arc::new(Fixed(w.store.clone())))
        .await
        .unwrap();
    let (error, page) = call(&client, "list_ready_stories", json!({})).await;
    assert!(!error, "{page}");
    let (error, claim) = call(&client, "claim_story", claim_args.clone()).await;
    assert!(!error, "{claim}");

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
    // Reintento tras el reinicio con el mismo requestId: el mismo reclamo, no uno nuevo.
    let (error, again) = call(&client, "claim_story", claim_args).await;
    assert!(!error, "{again}");
    assert_eq!(again["claimId"], claim["claimId"]);
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

/// Texto de un error de herramienta (falla si la llamada no fue un error).
async fn tool_error(client: &RunningService<RoleClient, ()>, tool: &str, args: Value) -> String {
    let (error, payload) = call(client, tool, args.clone()).await;
    assert!(error, "{tool} {args} debía fallar: {payload}");
    payload.as_str().unwrap().to_owned()
}

/// HU-015: consulta → reclamo → bloqueo → subtarea → entrega con un cliente MCP real.
#[tokio::test]
async fn an_agent_reports_and_delivers_through_a_real_mcp_client() {
    use kanbania_core::features::access::authenticate_agent;
    use kanbania_core::features::claims::{ClaimStoryInput, claim_story};
    use kanbania_core::features::review::{DeliverySource, DeliveryStatus, get_story_review};

    let w = world().await;
    let service = LocalService::start(w.data.path(), Arc::new(Fixed(w.store.clone())))
        .await
        .unwrap();
    let logs = tempfile::tempdir().unwrap();
    let client = connect(w.data.path(), &w.token, &logs.path().join("e.log")).await;

    // Consulta y reclamo.
    let (error, page) = call(&client, "list_ready_stories", json!({})).await;
    assert!(!error, "{page}");
    assert!(
        page["stories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == w.mine.id.as_str())
    );
    let (error, claim) = call(
        &client,
        "claim_story",
        json!({ "requestId": rid(), "storyId": w.mine.id }),
    )
    .await;
    assert!(!error, "{claim}");
    let claim_id = claim["claimId"].as_str().unwrap().to_owned();

    // Bloqueo: reintento idéntico → misma respuesta; mismo requestId con otro motivo → conflicto.
    let block_args =
        json!({ "claimId": claim_id, "requestId": rid(), "reason": "Falta acceso al repo" });
    let (error, blocked) = call(&client, "report_blocker", block_args.clone()).await;
    assert!(!error, "{blocked}");
    assert_eq!(blocked["blocked"], true);
    assert_eq!(blocked["storyId"], w.mine.id.as_str());
    let (error, again) = call(&client, "report_blocker", block_args.clone()).await;
    assert!(!error, "{again}");
    assert_eq!(again, blocked);
    let mut other_reason = block_args.clone();
    other_reason["reason"] = "Otro motivo".into();
    let text = tool_error(&client, "report_blocker", other_reason).await;
    assert!(text.starts_with("[idempotency_conflict]"), "{text}");
    let (_, context) = call(
        &client,
        "get_story_context",
        json!({ "storyId": w.mine.id }),
    )
    .await;
    assert_eq!(context["blockerReason"], "Falta acceso al repo");
    let review = get_story_review(&w.store, &w.project_id, &w.mine.id)
        .await
        .unwrap();
    let blocker = serde_json::to_value(review.blocker.unwrap()).unwrap();
    assert_eq!(blocker["source"], "reported", "afirmación del agente");
    // Quitar su propio bloqueo (reason omitido).
    let (error, cleared) = call(
        &client,
        "report_blocker",
        json!({ "claimId": claim_id, "requestId": rid() }),
    )
    .await;
    assert!(!error, "{cleared}");
    assert_eq!(cleared["blocked"], false);
    assert!(cleared["reason"].is_null());

    // Subtarea: un reintento no duplica; otro título con el mismo requestId → conflicto.
    let subtask_args = json!({
        "claimId": claim_id,
        "requestId": rid(),
        "title": "Preparar migración",
        "description": "Detalles **en Markdown**",
    });
    let (error, subtask) = call(&client, "create_subtask", subtask_args.clone()).await;
    assert!(!error, "{subtask}");
    assert_eq!(subtask["parentStoryId"], w.mine.id.as_str());
    assert_eq!(subtask["parentNumber"], w.mine.number);
    assert_eq!(subtask["title"], "Preparar migración");
    let (error, again) = call(&client, "create_subtask", subtask_args.clone()).await;
    assert!(!error, "{again}");
    assert_eq!(again["storyId"], subtask["storyId"]);
    let mut other_title = subtask_args.clone();
    other_title["title"] = "Otra cosa".into();
    let text = tool_error(&client, "create_subtask", other_title).await;
    assert!(text.starts_with("[idempotency_conflict]"), "{text}");
    let (_, context) = call(
        &client,
        "get_story_context",
        json!({ "storyId": w.mine.id }),
    )
    .await;
    let children = context["subtasks"].as_array().unwrap();
    assert_eq!(children.len(), 1, "sin duplicados: {children:?}");
    assert_eq!(children[0]["id"], subtask["storyId"]);
    // La subtarea nace sin agente: sale lista para cualquiera.
    let (_, child) = call(
        &client,
        "get_story_context",
        json!({ "storyId": subtask["storyId"] }),
    )
    .await;
    assert_eq!(child["assignment"], "none");
    assert_eq!(child["parent"]["id"], w.mine.id.as_str());
    assert_eq!(child["column"]["category"], "backlog");
    // Un solo nivel: la subtarea de un agente se puede reclamar, pero no admite subtareas de
    // agentes.
    let (error, child_claim) = call(
        &client,
        "claim_story",
        json!({ "requestId": rid(), "storyId": subtask["storyId"] }),
    )
    .await;
    assert!(!error, "{child_claim}");
    let text = tool_error(
        &client,
        "create_subtask",
        json!({ "claimId": child_claim["claimId"], "requestId": rid(), "title": "Nieta" }),
    )
    .await;
    assert!(text.starts_with("[validation]"), "{text}");
    let (_, child) = call(
        &client,
        "get_story_context",
        json!({ "storyId": subtask["storyId"] }),
    )
    .await;
    assert!(child["subtasks"].as_array().unwrap().is_empty());

    // Argumentos inválidos: no llegan al servicio.
    for bad in ["../session", "..", "a/b", "../../v1/session", ""] {
        for (tool, extra) in [
            ("report_blocker", json!({ "reason": "x" })),
            ("create_subtask", json!({ "title": "x" })),
            ("submit_result", json!({ "summary": "x" })),
            ("renew_claim", json!({})),
        ] {
            let mut args = extra.clone();
            args["claimId"] = bad.into();
            args["requestId"] = rid().into();
            let text = tool_error(&client, tool, args).await;
            assert!(
                text.starts_with("claimId no válido"),
                "{tool} {bad}: {text}"
            );
        }
    }
    for args in [
        json!({ "claimId": claim_id, "requestId": "con espacio", "summary": "x" }),
        json!({ "claimId": claim_id, "requestId": "a/../b", "summary": "x" }),
    ] {
        let text = tool_error(&client, "submit_result", args).await;
        assert!(text.starts_with("requestId no válido"), "{text}");
    }
    // Campos desconocidos (elegir agente o proyecto, o colarlos en una referencia): rechazados.
    for (tool, args) in [
        (
            "report_blocker",
            json!({ "claimId": claim_id, "requestId": rid(), "agentId": w.agent_id }),
        ),
        (
            "create_subtask",
            json!({ "claimId": claim_id, "requestId": rid(), "title": "x",
                    "projectId": w.other_project_id }),
        ),
        (
            "submit_result",
            json!({ "claimId": claim_id, "requestId": rid(), "summary": "x",
                    "references": [{ "kind": "url", "value": "https://a.example",
                                     "projectId": w.other_project_id }] }),
        ),
        (
            "submit_result",
            json!({ "claimId": claim_id, "requestId": rid(), "summary": "x",
                    "references": [{ "kind": "ftp", "value": "ftp://a.example" }] }),
        ),
    ] {
        let (error, _) = call(&client, tool, args.clone()).await;
        assert!(error, "{tool} {args}");
    }
    // Validación del núcleo, explicada al agente.
    let text = tool_error(
        &client,
        "submit_result",
        json!({ "claimId": claim_id, "requestId": rid(), "summary": "x",
                "references": [{ "kind": "path", "value": "relativa/archivo" }] }),
    )
    .await;
    assert!(text.starts_with("[validation]"), "{text}");
    for value in ["/tmp/../etc/passwd", "/tmp/./x", "/tmp/a\u{E0041}b"] {
        let text = tool_error(
            &client,
            "submit_result",
            json!({ "claimId": claim_id, "requestId": rid(), "summary": "x",
                    "references": [{ "kind": "path", "value": value }] }),
        )
        .await;
        assert!(text.starts_with("[validation]"), "{value}: {text}");
    }
    // Más de 64 KB en total: el MCP lo rechaza sin enviarlo.
    let text = tool_error(
        &client,
        "submit_result",
        json!({ "claimId": claim_id, "requestId": rid(),
                "summary": "x".repeat(kanbania_local_service::MAX_BODY_BYTES) }),
    )
    .await;
    assert!(
        text.contains("64 KB") && text.contains("No se envió nada"),
        "{text}"
    );

    // Reclamo de otro proyecto (del mismo agente): no se encuentra.
    let foreign_token = create_agent_credential(
        &w.store,
        &Actor::Human,
        &CreateAgentCredentialInput {
            project_id: w.other_project_id.clone(),
            agent_id: w.agent_id.clone(),
        },
    )
    .await
    .unwrap()
    .token;
    let foreign_session = authenticate_agent(&w.store, &foreign_token).await.unwrap();
    let foreign_claim = claim_story(
        &w.store,
        &foreign_session,
        &ClaimStoryInput {
            request_id: rid(),
            story_id: Some(w.foreign.id.clone()),
            number: None,
        },
    )
    .await
    .unwrap();
    for (tool, extra) in [
        ("report_blocker", json!({ "reason": "x" })),
        ("create_subtask", json!({ "title": "x" })),
        ("submit_result", json!({ "summary": "x" })),
    ] {
        let mut args = extra;
        args["claimId"] = foreign_claim.claim_id.clone().into();
        args["requestId"] = rid().into();
        let text = tool_error(&client, tool, args).await;
        assert!(text.starts_with("[not_found]"), "{tool}: {text}");
    }
    let foreign_review = get_story_review(&w.store, &w.other_project_id, &w.foreign.id)
        .await
        .unwrap();
    assert!(foreign_review.deliveries.is_empty() && foreign_review.blocker.is_none());

    // Bloqueo propio puesto al entregar: la entrega lo quita.
    let (error, _) = call(
        &client,
        "report_blocker",
        json!({ "claimId": claim_id, "requestId": rid(), "reason": "Espera revisión externa" }),
    )
    .await;
    assert!(!error);

    // Entrega: queda pendiente de revisión, sin aceptar; un reintento no la duplica.
    let submit_args = json!({
        "claimId": claim_id,
        "requestId": rid(),
        "summary": "Hecho: migración y pruebas.",
        "references": [
            { "kind": "url", "value": "https://example.com/pr/1" },
            { "kind": "path", "value": "/tmp/kanbania-informe.txt" },
        ],
    });
    let (error, delivered) = call(&client, "submit_result", submit_args.clone()).await;
    assert!(!error, "{delivered}");
    assert_eq!(delivered["deliveryNumber"], 1);
    assert_eq!(delivered["storyId"], w.mine.id.as_str());
    assert_eq!(delivered["inReview"], true);
    let (error, again) = call(&client, "submit_result", submit_args.clone()).await;
    assert!(!error, "{again}");
    assert_eq!(again, delivered);
    let mut other_summary = submit_args.clone();
    other_summary["summary"] = "Otro resumen".into();
    let text = tool_error(&client, "submit_result", other_summary).await;
    assert!(text.starts_with("[idempotency_conflict]"), "{text}");

    let review = get_story_review(&w.store, &w.project_id, &w.mine.id)
        .await
        .unwrap();
    assert!(review.accepted_at.is_none(), "un agente nunca acepta");
    assert_eq!(review.deliveries.len(), 1, "sin duplicados");
    let delivery = &review.deliveries[0];
    assert_eq!(delivery.source, DeliverySource::Reported);
    assert_eq!(delivery.status, DeliveryStatus::Pending);
    assert_eq!(delivery.agent_id.as_deref(), Some(w.agent_id.as_str()));
    assert_eq!(delivery.references.len(), 2);
    let (_, context) = call(
        &client,
        "get_story_context",
        json!({ "storyId": w.mine.id }),
    )
    .await;
    assert_eq!(context["column"]["category"], "review");
    assert!(context["acceptedAt"].is_null());
    assert!(context["yourClaim"].is_null(), "entregar cierra el reclamo");
    assert!(
        context["blockerReason"].is_null(),
        "entregar quita el bloqueo propio"
    );
    assert!(review.blocker.is_none());
    // Con la entrega pendiente, la historia no vuelve a la lista ni se puede reclamar.
    let (_, page) = call(&client, "list_ready_stories", json!({})).await;
    assert!(
        page["stories"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["id"] != w.mine.id.as_str()),
        "{page}"
    );
    assert_eq!(context["ready"], false);
    let text = tool_error(
        &client,
        "claim_story",
        json!({ "requestId": rid(), "storyId": w.mine.id }),
    )
    .await;
    assert!(text.starts_with("[not_ready]"), "{text}");

    // Tras entregar, el reclamo ya no sirve para nada.
    for (tool, extra) in [
        ("renew_claim", json!({})),
        ("report_blocker", json!({ "reason": "x" })),
        ("create_subtask", json!({ "title": "x" })),
        ("submit_result", json!({ "summary": "Segunda entrega" })),
    ] {
        let mut args = extra;
        args["claimId"] = claim_id.clone().into();
        args["requestId"] = rid().into();
        let text = tool_error(&client, tool, args).await;
        assert!(text.starts_with("[claim_ended]"), "{tool}: {text}");
    }
    let review = get_story_review(&w.store, &w.project_id, &w.mine.id)
        .await
        .unwrap();
    assert_eq!(review.deliveries.len(), 1);

    service.stop().await;
    client.cancel().await.unwrap();
}
