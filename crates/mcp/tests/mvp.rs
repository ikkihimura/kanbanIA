//! HU-016 (CA-1, CA-2): ciclo completo del MVP con un cliente MCP real y reinicio de la app.
//!
//! proyecto → historia con criterios → agente → asignación → credencial → consulta MCP →
//! reclamo → entrega → aceptación humana → reinicio (servicio y almacén cerrados y reabiertos
//! sobre el mismo directorio de datos) → persistencia comprobada desde el núcleo y desde el
//! mismo proceso MCP. Base SQLite temporal real; nada simulado.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use kanbania_core::error::ErrorPayload;
use kanbania_core::features::access::{CreateAgentCredentialInput, create_agent_credential};
use kanbania_core::features::agent_activity::{
    ActivityKind, StoryActivityInput, list_story_activity,
};
use kanbania_core::features::claims::list_active_claims;
use kanbania_core::features::columns::{ColumnCategory, list_columns};
use kanbania_core::features::projects::{CreateProjectInput, create_project};
use kanbania_core::features::relations::{
    SetStoryAgentInput, get_story_relations, set_story_agent,
};
use kanbania_core::features::review::{
    AcceptStoryInput, DeliverySource, DeliveryStatus, accept_story, get_story_review,
};
use kanbania_core::features::stories::{
    CreateStoryInput, CriterionDraft, Priority, UpdateStoryInput, create_story, get_story,
    update_story,
};
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

/// El host real pide el almacén al `StorageSlot`; aquí basta uno fijo.
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

/// Resultado de una herramienta que debía funcionar.
async fn ok(client: &RunningService<RoleClient, ()>, tool: &str, args: Value) -> Value {
    let (error, payload) = call(client, tool, args.clone()).await;
    assert!(!error, "{tool} {args}: {payload}");
    payload
}

/// Texto de un error de herramienta (falla si la llamada no fue un error).
async fn tool_error(client: &RunningService<RoleClient, ()>, tool: &str, args: Value) -> String {
    let (error, payload) = call(client, tool, args.clone()).await;
    assert!(error, "{tool} {args} debía fallar: {payload}");
    payload.as_str().unwrap().to_owned()
}

fn listed(page: &Value, story_id: &str) -> Option<Value> {
    page["stories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == story_id)
        .cloned()
}

#[tokio::test]
async fn mvp_cycle_survives_an_app_restart() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();

    // --- Planificación desde la app (persona) ---
    let project = create_project(
        &store,
        UI,
        &CreateProjectInput {
            request_id: rid(),
            name: "Ciclo MVP".into(),
        },
    )
    .await
    .unwrap();
    // Como en la guía local: la historia se planifica en «Listo».
    let ready = list_columns(&store, &project.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.category == ColumnCategory::Ready)
        .unwrap();
    let created = create_story(
        &store,
        UI,
        &CreateStoryInput {
            request_id: rid(),
            project_id: project.id.clone(),
            title: "Exportar recetas a PDF".into(),
            column_id: Some(ready.id.clone()),
            priority: Some(Priority::High),
        },
    )
    .await
    .unwrap();
    let story = update_story(
        &store,
        &UpdateStoryInput {
            project_id: project.id.clone(),
            id: created.id.clone(),
            expected_revision: created.revision,
            title: created.title.clone(),
            description: "Botón «Exportar» en la ficha de la receta.".into(),
            priority: Priority::High,
            criteria: vec![
                CriterionDraft {
                    id: None,
                    text: "El PDF incluye ingredientes y pasos.".into(),
                },
                CriterionDraft {
                    id: None,
                    text: "El nombre del archivo es el título de la receta.".into(),
                },
            ],
        },
    )
    .await
    .unwrap();
    assert_eq!(story.criteria.len(), 2);
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
    set_story_agent(
        &store,
        &SetStoryAgentInput {
            project_id: project.id.clone(),
            story_id: story.id.clone(),
            agent_id: Some(agent.id.clone()),
        },
    )
    .await
    .unwrap();
    let token = create_agent_credential(
        &store,
        &Actor::Human,
        &CreateAgentCredentialInput {
            project_id: project.id.clone(),
            agent_id: agent.id.clone(),
        },
    )
    .await
    .unwrap()
    .token;

    // --- La app abre el servicio local; el agente se conecta por MCP ---
    let service = LocalService::start(data.path(), Arc::new(Fixed(store.clone())))
        .await
        .unwrap();
    let logs = tempfile::tempdir().unwrap();
    let stderr_log = logs.path().join("stderr.log");
    let client = connect(data.path(), &token, &stderr_log).await;

    // Consulta: la historia asignada aparece como lista y suya.
    let page = ok(&client, "list_ready_stories", json!({})).await;
    let entry = listed(&page, &story.id).unwrap_or_else(|| panic!("no aparece: {page}"));
    assert_eq!(entry["assignedToYou"], true);
    assert_eq!(entry["number"], story.number);
    assert_eq!(entry["criteriaCount"], 2);
    let context = ok(&client, "get_story_context", json!({ "storyId": story.id })).await;
    assert_eq!(context["assignment"], "you");
    assert_eq!(context["ready"], true);
    assert_eq!(context["criteria"].as_array().unwrap().len(), 2);

    // Reclamo y entrega.
    let claim = ok(
        &client,
        "claim_story",
        json!({ "requestId": rid(), "number": story.number }),
    )
    .await;
    assert_eq!(claim["storyId"], story.id.as_str());
    let claim_id = claim["claimId"].as_str().unwrap().to_owned();
    let delivered = ok(
        &client,
        "submit_result",
        json!({
            "claimId": claim_id,
            "requestId": rid(),
            "summary": "Exportación a PDF hecha, con pruebas.",
            "references": [{ "kind": "url", "value": "https://example.com/pr/7" }],
        }),
    )
    .await;
    assert_eq!(delivered["deliveryNumber"], 1);
    assert_eq!(delivered["inReview"], true);
    let review = get_story_review(&store, &project.id, &story.id)
        .await
        .unwrap();
    assert!(review.accepted_at.is_none(), "un agente nunca acepta");
    assert_eq!(review.deliveries[0].status, DeliveryStatus::Pending);

    // --- Aceptación humana desde la app ---
    let current = get_story(&store, &project.id, &story.id).await.unwrap();
    let accepted = accept_story(
        &store,
        &Actor::Human,
        &AcceptStoryInput {
            project_id: project.id.clone(),
            story_id: story.id.clone(),
            expected_revision: current.revision,
            satisfied_criteria: current.criteria.iter().map(|c| c.id.clone()).collect(),
            column_id: None,
            to_index: None,
        },
    )
    .await
    .unwrap();
    assert!(accepted.accepted_at.is_some());
    let done = list_columns(&store, &project.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.category == ColumnCategory::Done)
        .unwrap();
    assert_eq!(accepted.column_id, done.id);

    // --- Cierre de la app: servicio parado y almacén cerrado ---
    service.stop().await;
    // Conexión perdida: mensaje accionable (abrir la app), el proceso MCP sigue vivo.
    let text = tool_error(&client, "list_ready_stories", json!({})).await;
    assert!(text.starts_with("[app_closed]"), "{text}");
    assert!(text.contains("Ábrelo y vuelve a intentarlo"), "{text}");
    let text = tool_error(&client, "get_story_context", json!({ "storyId": story.id })).await;
    assert!(text.starts_with("[app_closed]"), "{text}");
    store.close().await;
    drop(store);

    // --- Reapertura sobre el mismo directorio de datos ---
    let store = Store::open(data.path()).await.unwrap();
    let service = LocalService::start(data.path(), Arc::new(Fixed(store.clone())))
        .await
        .unwrap();

    // Persistencia desde el núcleo.
    let story_now = get_story(&store, &project.id, &story.id).await.unwrap();
    assert!(story_now.accepted_at.is_some());
    assert_eq!(story_now.column_id, done.id);
    assert_eq!(story_now.criteria.len(), 2);
    let relations = get_story_relations(&store, &project.id, &story.id)
        .await
        .unwrap();
    assert_eq!(relations.agent.map(|a| a.id), Some(agent.id.clone()));
    let review = get_story_review(&store, &project.id, &story.id)
        .await
        .unwrap();
    assert!(review.accepted_at.is_some());
    assert!(review.blocker.is_none());
    assert_eq!(review.deliveries.len(), 1);
    let delivery = &review.deliveries[0];
    assert_eq!(delivery.source, DeliverySource::Reported);
    assert_eq!(delivery.status, DeliveryStatus::Accepted);
    assert_eq!(delivery.agent_id.as_deref(), Some(agent.id.as_str()));
    assert_eq!(delivery.references.len(), 1);
    assert_eq!(delivery.references[0].value, "https://example.com/pr/7");
    assert_eq!(
        review.history.len(),
        1,
        "evento de aceptación con criterios"
    );
    assert_eq!(review.history[0].criteria.len(), 2);
    let activity = list_story_activity(
        &store,
        &StoryActivityInput {
            project_id: project.id.clone(),
            story_id: story.id.clone(),
        },
    )
    .await
    .unwrap();
    let kinds: Vec<ActivityKind> = activity.entries.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&ActivityKind::Claimed), "{kinds:?}");
    assert!(kinds.contains(&ActivityKind::Delivered), "{kinds:?}");
    assert!(activity.last_reported_at.is_some());
    assert!(
        list_active_claims(&store, &project.id)
            .await
            .unwrap()
            .is_empty()
    );
    // El motivo `delivered` del cierre lo fija `crates/core/tests/agent_reports.rs` (el núcleo
    // no expone los reclamos cerrados y este crate no enlaza sqlx); aquí, que sigue cerrado.

    // Persistencia desde el mismo proceso MCP, que sigue a la app reabierta.
    let page = ok(&client, "list_ready_stories", json!({})).await;
    assert!(listed(&page, &story.id).is_none(), "{page}");
    let context = ok(&client, "get_story_context", json!({ "storyId": story.id })).await;
    assert!(context["acceptedAt"].is_string());
    assert_eq!(context["column"]["category"], "done");
    assert_eq!(context["ready"], false);
    assert!(context["yourClaim"].is_null());
    let text = tool_error(
        &client,
        "claim_story",
        json!({ "requestId": rid(), "storyId": story.id }),
    )
    .await;
    assert!(text.starts_with("[not_ready]"), "{text}");
    let text = tool_error(
        &client,
        "submit_result",
        json!({ "claimId": claim_id, "requestId": rid(), "summary": "Otra" }),
    )
    .await;
    assert!(text.starts_with("[claim_ended]"), "{text}");
    // Ninguna herramienta acepta: aceptar es solo de personas.
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 7);
    assert!(
        tools
            .iter()
            .all(|t| !t.name.contains("accept") && !t.name.contains("move")),
        "{tools:?}"
    );

    service.stop().await;
    client.cancel().await.unwrap();
    let stderr = std::fs::read_to_string(&stderr_log).unwrap();
    assert!(!stderr.contains(&token), "{stderr}");
}
