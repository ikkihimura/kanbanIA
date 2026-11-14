//! Contrato del servicio local (HU-012) con SQLite temporal real y peticiones HTTP reales.

// Pruebas del servidor: necesitan la feature `server` (por defecto).
#![cfg(feature = "server")]
// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::sync::Arc;

use kanbania_core::error::CoreError;
use kanbania_core::features::access::{
    CreateAgentCredentialInput, RevokeAgentCredentialInput, create_agent_credential,
    revoke_agent_credential,
};
use kanbania_core::features::agent_activity::AgentChange;
use kanbania_core::features::claims::{ReleaseClaimInput, release_claim};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::stories::{CreateStoryInput, StorySummary, create_story};
use kanbania_core::features::team::{
    Agent, AgentFields, AvatarSpec, CreateAgentInput, create_agent,
};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::storage::Store;
use kanbania_local_service::descriptor::{self, Descriptor, Secret};
use kanbania_local_service::{ClientError, LocalService, ServiceClient, StoreSource};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const UI: &str = "ui";

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Almacén fijo (en el host es el `StorageSlot`).
struct Fixed(Store);

impl StoreSource for Fixed {
    fn store(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Store, kanbania_core::error::ErrorPayload>>
                + Send
                + '_,
        >,
    > {
        let store = self.0.clone();
        Box::pin(async move { Ok(store) })
    }
}

/// Almacenamiento no disponible (p. ej. durante una restauración).
struct Unavailable;

impl StoreSource for Unavailable {
    fn store(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Store, kanbania_core::error::ErrorPayload>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Err(CoreError::StorageLocked.to_payload(None)) })
    }
}

struct World {
    data: tempfile::TempDir,
    store: Store,
    mine: Project,
    other: Project,
    agent: Agent,
    story: StorySummary,
    foreign_story: StorySummary,
    token: String,
    credential_id: String,
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
            priority: None,
        },
    )
    .await
    .unwrap()
}

async fn world() -> World {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let (mine, other) = (project(&store, "Mío").await, project(&store, "Ajeno").await);
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
    let story_mine = story(&store, &mine, "Tarea mía").await;
    let foreign_story = story(&store, &other, "Tarea ajena").await;
    let created = create_agent_credential(
        &store,
        &Actor::Human,
        &CreateAgentCredentialInput {
            project_id: mine.id.clone(),
            agent_id: agent.id.clone(),
        },
    )
    .await
    .unwrap();
    World {
        data,
        store,
        mine,
        other,
        agent,
        story: story_mine,
        foreign_story,
        token: created.token,
        credential_id: created.credential.id,
    }
}

async fn start(w: &World) -> LocalService {
    LocalService::start(w.data.path(), Arc::new(Fixed(w.store.clone())))
        .await
        .unwrap()
}

/// Petición HTTP/1.1 en crudo (para controlar cada cabecera). Devuelve estado y cuerpo JSON.
async fn raw(port: u16, path: &str, headers: &[(&str, &str)]) -> (u16, Value, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut request = format!("GET {path} HTTP/1.1\r\nConnection: close\r\n");
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        request.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for (k, v) in headers {
        request.push_str(&format!("{k}: {v}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let text = String::from_utf8_lossy(&response).into_owned();
    let status = text[9..12].parse().unwrap();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    let json = serde_json::from_str(body).unwrap_or(Value::Null);
    (status, json, text)
}

fn session_token(data_dir: &Path) -> String {
    descriptor::read(data_dir)
        .unwrap()
        .session_token
        .expose()
        .to_owned()
}

// ---------- CA-1/CA-2: autenticado, proyecto acotado ----------

#[tokio::test]
async fn an_authenticated_agent_reads_its_project_and_nothing_else() {
    let w = world().await;
    let service = start(&w).await;
    let client = ServiceClient::new(w.data.path(), w.token.clone());

    let session = client.get_json("/v1/session").await.unwrap();
    assert_eq!(session["agentId"], w.agent.id);
    assert_eq!(session["agentName"], "Robo");
    assert_eq!(session["projectId"], w.mine.id);
    assert_eq!(session["projectName"], "Mío");

    let story = client
        .get_json(&format!("/v1/stories/{}/context", w.story.id))
        .await
        .unwrap();
    assert_eq!(story["title"], "Tarea mía");
    let by_number = client
        .get_json("/v1/stories/by-number/1/context")
        .await
        .unwrap();
    assert_eq!(by_number["id"], w.story.id.as_str());
    let ready = client.get_json("/v1/stories/ready").await.unwrap();
    let titles: Vec<&str> = ready["stories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Tarea mía"], "solo las del proyecto propio");

    // Historia de otro proyecto: no encontrada, sin revelar que existe.
    let error = client
        .get_json(&format!("/v1/stories/{}/context", w.foreign_story.id))
        .await
        .unwrap_err();
    let ClientError::Api { status, code, .. } = error else {
        panic!("{error:?}");
    };
    assert_eq!((status, code.as_str()), (404, "not_found"));
    // Tampoco sirve pasar el proyecto en la petición: ninguna ruta lo acepta.
    let error = client
        .get_json(&format!(
            "/v1/stories/{}/context?projectId={}",
            w.foreign_story.id, w.other.id
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, ClientError::Api { status: 404, .. }));
    for query in [
        format!("projectId={}", w.other.id),
        "limit=0".into(),
        "limit=51".into(),
        "limit=abc".into(),
        "cursor=x".into(),
    ] {
        let error = client
            .get_json(&format!("/v1/stories/ready?{query}"))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, ClientError::Api { status: 400, code, .. } if code == "validation"),
            "{query}: {error:?}"
        );
    }
    for path in [
        "/v1/stories/by-number/0/context",
        "/v1/stories/by-number/x/context",
    ] {
        let error = client.get_json(path).await.unwrap_err();
        assert!(
            matches!(error, ClientError::Api { status: 404, .. }),
            "{path}"
        );
    }

    service.stop().await;
}

#[tokio::test]
async fn every_route_requires_both_the_session_token_and_a_valid_credential() {
    let w = world().await;
    let service = start(&w).await;
    let port = service.port();
    let session = session_token(w.data.path());
    let bearer = format!("Bearer {}", w.token);
    let wrong_session = "0".repeat(session.len());
    let unknown = format!("Bearer kbia_{}", "a".repeat(64));

    let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
        ("sin nada", vec![]),
        ("solo credencial", vec![("Authorization", &bearer)]),
        ("solo sesión", vec![("X-Kanbania-Session", &session)]),
        (
            "sesión incorrecta",
            vec![
                ("X-Kanbania-Session", &wrong_session),
                ("Authorization", &bearer),
            ],
        ),
        (
            "credencial desconocida",
            vec![
                ("X-Kanbania-Session", &session),
                ("Authorization", &unknown),
            ],
        ),
        (
            "esquema distinto",
            vec![
                ("X-Kanbania-Session", &session),
                ("Authorization", &w.token),
            ],
        ),
    ];
    for path in [
        "/v1/session",
        "/v1/stories/ready",
        &format!("/v1/stories/{}/context", w.story.id),
    ] {
        for (label, headers) in &cases {
            let (status, body, text) = raw(port, path, headers).await;
            assert_eq!(status, 401, "{label} {path}");
            assert_eq!(body["code"], "unauthorized", "{label}");
            assert!(!text.contains(&w.token) && !text.contains(&session));
        }
    }

    // Con ambas: entra.
    let ok = [
        ("X-Kanbania-Session", session.as_str()),
        ("Authorization", bearer.as_str()),
    ];
    assert_eq!(raw(port, "/v1/session", &ok).await.0, 200);

    // Revocada: deja de entrar en la siguiente petición.
    revoke_agent_credential(
        &w.store,
        &Actor::Human,
        &RevokeAgentCredentialInput {
            project_id: w.mine.id.clone(),
            id: w.credential_id.clone(),
        },
    )
    .await
    .unwrap();
    let (status, body, _) = raw(port, "/v1/session", &ok).await;
    assert_eq!((status, body["code"].as_str()), (401, Some("unauthorized")));

    service.stop().await;
}

#[tokio::test]
async fn foreign_hosts_browser_origins_and_large_payloads_are_rejected() {
    let w = world().await;
    let service = start(&w).await;
    let port = service.port();
    let session = session_token(w.data.path());
    let bearer = format!("Bearer {}", w.token);
    let auth = [
        ("X-Kanbania-Session", session.as_str()),
        ("Authorization", bearer.as_str()),
    ];

    // DNS rebinding: un nombre que resuelve a 127.0.0.1 pero no es el host esperado.
    for host in [
        "evil.example:80",
        &format!("evil.example:{port}"),
        "127.0.0.1",
    ] {
        let mut headers = auth.to_vec();
        headers.push(("Host", host));
        let (status, body, _) = raw(port, "/v1/session", &headers).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (403, Some("forbidden")),
            "{host}"
        );
    }
    let localhost = format!("localhost:{port}");
    let mut headers = auth.to_vec();
    headers.push(("Host", &localhost));
    assert_eq!(raw(port, "/v1/session", &headers).await.0, 200);

    // Navegadores: cualquier Origin, también en el saludo y aunque lleve credenciales.
    for path in ["/v1/session", "/v1/handshake"] {
        let mut headers = auth.to_vec();
        headers.push(("Origin", "http://localhost:5173"));
        headers.push(("X-Kanbania-Nonce", "ab".repeat(16).leak()));
        let (status, body, text) = raw(port, path, &headers).await;
        assert_eq!((status, body["code"].as_str()), (403, Some("forbidden")));
        assert!(
            !text.to_lowercase().contains("access-control-allow"),
            "sin CORS"
        );
    }

    // Cuerpo mayor que el límite: rechazado antes de autenticar o leerlo.
    let big = (kanbania_local_service::MAX_BODY_BYTES + 1).to_string();
    let mut headers = auth.to_vec();
    headers.push(("Content-Length", &big));
    let (status, body, _) = raw(port, "/v1/session", &headers).await;
    assert_eq!((status, body["code"].as_str()), (413, Some("validation")));

    // Rutas desconocidas (p. ej. un intento de SQL o shell genérico): 404 JSON.
    let retired = format!("/v1/stories/{}", w.story.id);
    for path in ["/v1/sql", "/v1/exec", "/v1/projects", "/", retired.as_str()] {
        let (status, body, _) = raw(port, path, &auth).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (404, Some("not_found")),
            "{path}"
        );
    }

    service.stop().await;
}

#[tokio::test]
async fn storage_errors_reach_the_client_as_structured_errors() {
    let w = world().await;
    let service = LocalService::start(w.data.path(), Arc::new(Unavailable))
        .await
        .unwrap();
    let client = ServiceClient::new(w.data.path(), w.token.clone());
    let error = client.get_json("/v1/session").await.unwrap_err();
    let ClientError::Api { status, code, .. } = error else {
        panic!("{error:?}");
    };
    assert_eq!((status, code.as_str()), (503, "storage_locked"));
    service.stop().await;
}

// ---------- CA-1: descriptor protegido ----------

#[tokio::test]
async fn the_descriptor_is_private_and_removed_on_stop() {
    let w = world().await;
    let service = start(&w).await;
    let path = descriptor::path(w.data.path());
    let read = descriptor::read(w.data.path()).unwrap();
    assert_eq!(read.port, service.port());
    assert_eq!(read.session_token.expose().len(), 64);
    // El Debug no muestra el token.
    assert!(!format!("{read:?}").contains(read.session_token.expose()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    service.stop().await;
    assert!(!path.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn a_descriptor_readable_by_others_is_not_trusted() {
    use std::os::unix::fs::PermissionsExt;
    let w = world().await;
    let service = start(&w).await;
    let path = descriptor::path(w.data.path());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let error = ServiceClient::new(w.data.path(), w.token.clone())
        .get_json("/v1/session")
        .await
        .unwrap_err();
    assert!(
        matches!(error, ClientError::InsecureDescriptor),
        "{error:?}"
    );
    service.stop().await;
}

// ---------- CA-3: app cerrada, sin otro propietario ----------

#[tokio::test]
async fn with_the_app_closed_the_client_fails_clearly_and_never_opens_the_data() {
    let empty = tempfile::tempdir().unwrap();
    let client = ServiceClient::new(empty.path(), format!("kbia_{}", "0".repeat(64)));
    let error = client.get_json("/v1/session").await.unwrap_err();
    assert!(matches!(error, ClientError::NotRunning));
    assert!(error.to_string().contains("no está abierto"));
    assert_eq!(
        std::fs::read_dir(empty.path()).unwrap().count(),
        0,
        "no crea nada"
    );

    // Descriptor huérfano tras un cierre brusco: el puerto ya no escucha.
    let w = world().await;
    let service = start(&w).await;
    let port = service.port();
    let orphan = descriptor::read(w.data.path()).unwrap();
    service.stop().await;
    descriptor::write(w.data.path(), &orphan).unwrap();
    let error = ServiceClient::new(w.data.path(), w.token.clone())
        .get_json("/v1/session")
        .await
        .unwrap_err();
    assert!(
        matches!(error, ClientError::NotRunning),
        "{error:?} (puerto {port})"
    );
}

#[tokio::test]
async fn the_client_does_not_hand_its_credential_to_an_impostor_on_the_port() {
    let w = world().await;
    // Otro proceso escucha donde apunta un descriptor (caducado) y responde al saludo.
    let impostor = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = impostor.local_addr().unwrap().port();
    descriptor::write(
        w.data.path(),
        &Descriptor::new(port, Secret::new("1".repeat(64))),
    )
    .unwrap();
    let seen = tokio::spawn(async move {
        let mut received = String::new();
        let (mut socket, _) = impostor.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        // Atiende peticiones hasta que el cliente cierre la conexión.
        loop {
            let n = match socket.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            received.push_str(&String::from_utf8_lossy(&buf[..n]));
            let body = r#"{"apiVersion":1,"proof":"deadbeef"}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            if socket.write_all(reply.as_bytes()).await.is_err() {
                break;
            }
        }
        received
    });

    let error = ServiceClient::new(w.data.path(), w.token.clone())
        .get_json("/v1/session")
        .await
        .unwrap_err();
    assert!(matches!(error, ClientError::Untrusted), "{error:?}");
    let received = seen.await.unwrap();
    assert!(received.contains("/v1/handshake"));
    assert!(!received.contains(&w.token), "la credencial no salió");
    assert!(!received.contains("1111111111"), "ni el token de sesión");
}

#[tokio::test]
async fn a_host_with_another_api_version_is_reported_clearly() {
    use kanbania_local_service::protocol::proof;
    let w = world().await;
    // Un host «futuro» legítimo (conoce el token) que habla la API v2.
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let token = Secret::new("2".repeat(64));
    descriptor::write(w.data.path(), &Descriptor::new(port, token.clone())).unwrap();
    let seen = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut received = String::new();
        let mut buf = vec![0u8; 8192];
        loop {
            let n = match socket.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
            received.push_str(&chunk);
            let nonce = chunk
                .lines()
                .find_map(|l| l.strip_prefix("x-kanbania-nonce: "))
                .unwrap_or_default()
                .trim()
                .to_owned();
            let body = format!(r#"{{"apiVersion":2,"proof":"{}"}}"#, proof(&token, &nonce));
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            if socket.write_all(reply.as_bytes()).await.is_err() {
                break;
            }
        }
        received
    });
    let error = ServiceClient::new(w.data.path(), w.token.clone())
        .get_json("/v1/session")
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            ClientError::Incompatible {
                server: 2,
                client: 1
            }
        ),
        "{error:?}"
    );
    assert!(error.to_string().contains("API v2"));
    assert!(
        !seen.await.unwrap().contains(&w.token),
        "no envía la credencial"
    );
}

// ---------- HU-014: reclamar y renovar por HTTP ----------

async fn second_agent_client(w: &World) -> ServiceClient {
    let other = create_agent(
        &w.store,
        UI,
        &CreateAgentInput {
            request_id: rid(),
            fields: AgentFields {
                name: "Robo dos".into(),
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
    let created = create_agent_credential(
        &w.store,
        &Actor::Human,
        &CreateAgentCredentialInput {
            project_id: w.mine.id.clone(),
            agent_id: other.id.clone(),
        },
    )
    .await
    .unwrap();
    ServiceClient::new(w.data.path(), created.token)
}

fn api_code(error: &ClientError) -> (u16, &str) {
    match error {
        ClientError::Api { status, code, .. } => (*status, code.as_str()),
        // No es un error de la API: la comparación de la prueba fallará mostrando esto.
        _ => (0, "no_es_error_de_api"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_clients_claiming_the_same_story_get_exactly_one_success() {
    let w = world().await;
    let service = start(&w).await;
    let first = ServiceClient::new(w.data.path(), w.token.clone());
    let second = second_agent_client(&w).await;

    for round in 0..5 {
        let s = story(&w.store, &w.mine, &format!("Carrera {round}")).await;
        let a = serde_json::json!({ "requestId": rid(), "storyId": s.id });
        let b = serde_json::json!({ "requestId": rid(), "storyId": s.id });
        let (ra, rb) = tokio::join!(
            first.post_json("/v1/claims", &a),
            second.post_json("/v1/claims", &b)
        );
        let results = [ra, rb];
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "ronda {round}: {results:?}"
        );
        let loser = results.iter().find_map(|r| r.as_ref().err()).unwrap();
        assert_eq!(api_code(loser), (409, "claimed"), "ronda {round}");
        // Libera (como la persona en la app) para que el tope por agente no intervenga.
        let winner = results.iter().find_map(|r| r.as_ref().ok()).unwrap();
        release_claim(
            &w.store,
            &Actor::Human,
            &ReleaseClaimInput {
                project_id: w.mine.id.clone(),
                claim_id: winner["claimId"].as_str().unwrap().to_owned(),
            },
        )
        .await
        .unwrap();
    }
    service.stop().await;
}

#[tokio::test]
async fn an_agent_claims_and_renews_only_within_its_session() {
    let w = world().await;
    let service = start(&w).await;
    let client = ServiceClient::new(w.data.path(), w.token.clone());

    let request_id = rid();
    let body = serde_json::json!({ "requestId": request_id, "storyId": w.story.id });
    let claim = client.post_json("/v1/claims", &body).await.unwrap();
    assert_eq!(claim["storyId"], w.story.id.as_str());
    assert_eq!(claim["owner"]["agentId"], w.agent.id.as_str());
    assert!(claim["claimId"].is_string() && claim["expiresAt"].is_string());
    // Reintento idéntico → mismo reclamo; mismo requestId con otra historia → conflicto.
    assert_eq!(client.post_json("/v1/claims", &body).await.unwrap(), claim);
    let other = story(&w.store, &w.mine, "Otra").await;
    let reused = serde_json::json!({ "requestId": request_id, "storyId": other.id });
    let error = client.post_json("/v1/claims", &reused).await.unwrap_err();
    assert_eq!(api_code(&error), (409, "idempotency_conflict"));

    // El proyecto y el agente salen de la sesión: campos así se rechazan, no se usan.
    for extra in [
        serde_json::json!({ "requestId": rid(), "storyId": other.id, "agentId": "x" }),
        serde_json::json!({ "requestId": rid(), "storyId": other.id, "projectId": w.other.id }),
        serde_json::json!({ "requestId": rid(), "number": "uno" }),
    ] {
        let error = client.post_json("/v1/claims", &extra).await.unwrap_err();
        assert_eq!(api_code(&error), (400, "validation"), "{extra}");
    }
    // Historia de otro proyecto → 404, también por número.
    let foreign = serde_json::json!({ "requestId": rid(), "storyId": w.foreign_story.id });
    let error = client.post_json("/v1/claims", &foreign).await.unwrap_err();
    assert_eq!(api_code(&error), (404, "not_found"));
    // Un número que solo existe en el otro proyecto: se resuelve en el de la sesión → 404.
    let mut foreign_only = story(&w.store, &w.other, "Solo ajena").await;
    while foreign_only.number <= other.number {
        foreign_only = story(&w.store, &w.other, "Solo ajena").await;
    }
    let by_number = serde_json::json!({ "requestId": rid(), "number": foreign_only.number });
    let error = client
        .post_json("/v1/claims", &by_number)
        .await
        .unwrap_err();
    assert_eq!(api_code(&error), (404, "not_found"));

    let claim_id = claim["claimId"].as_str().unwrap();
    let renewed = client
        .post_json(
            &format!("/v1/claims/{claim_id}/renew"),
            &serde_json::json!({ "requestId": rid() }),
        )
        .await
        .unwrap();
    assert_eq!(renewed["claimId"], claim_id);
    assert!(renewed["renewedAt"].is_string());
    let error = client
        .post_json(
            "/v1/claims/no-existe/renew",
            &serde_json::json!({ "requestId": rid() }),
        )
        .await
        .unwrap_err();
    assert_eq!(api_code(&error), (404, "not_found"));
    // Otro agente del mismo proyecto no renueva un reclamo ajeno.
    let second = second_agent_client(&w).await;
    let error = second
        .post_json(
            &format!("/v1/claims/{claim_id}/renew"),
            &serde_json::json!({ "requestId": rid() }),
        )
        .await
        .unwrap_err();
    assert_eq!(api_code(&error), (404, "not_found"));

    // Sin credencial válida no se reclama nada.
    let anonymous = ServiceClient::new(w.data.path(), format!("kbia_{}", "0".repeat(64)));
    let error = anonymous
        .post_json(
            "/v1/claims",
            &serde_json::json!({ "requestId": rid(), "storyId": other.id }),
        )
        .await
        .unwrap_err();
    assert_eq!(api_code(&error), (401, "unauthorized"));
    // Credencial revocada: ni reclamar ni renovar.
    revoke_agent_credential(
        &w.store,
        &Actor::Human,
        &RevokeAgentCredentialInput {
            project_id: w.mine.id.clone(),
            id: w.credential_id.clone(),
        },
    )
    .await
    .unwrap();
    let error = client
        .post_json(
            "/v1/claims",
            &serde_json::json!({ "requestId": rid(), "storyId": other.id }),
        )
        .await
        .unwrap_err();
    assert_eq!(api_code(&error), (401, "unauthorized"));
    let error = client
        .post_json(
            &format!("/v1/claims/{claim_id}/renew"),
            &serde_json::json!({ "requestId": rid() }),
        )
        .await
        .unwrap_err();
    assert_eq!(api_code(&error), (401, "unauthorized"));
    service.stop().await;
}

// ---------- HU-015: bloqueos, subtareas y entregas por HTTP ----------

/// Almacén fijo que además registra los avisos `agent_changed`.
struct Recording {
    store: Store,
    changes: std::sync::Mutex<Vec<AgentChange>>,
}

impl Recording {
    fn new(store: &Store) -> Arc<Self> {
        Arc::new(Self {
            store: store.clone(),
            changes: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Avisos recibidos desde la última llamada.
    fn take(&self) -> Vec<AgentChange> {
        std::mem::take(&mut *self.changes.lock().unwrap())
    }
}

impl StoreSource for Recording {
    fn store(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Store, kanbania_core::error::ErrorPayload>>
                + Send
                + '_,
        >,
    > {
        let store = self.store.clone();
        Box::pin(async move { Ok(store) })
    }

    fn agent_changed(&self, change: AgentChange) {
        self.changes.lock().unwrap().push(change);
    }
}

async fn claim_on(client: &ServiceClient, story_id: &str) -> String {
    let claim = client
        .post_json(
            "/v1/claims",
            &serde_json::json!({ "requestId": rid(), "storyId": story_id }),
        )
        .await
        .unwrap();
    claim["claimId"].as_str().unwrap().to_owned()
}

/// Cuerpos válidos de las tres rutas para un reclamo.
fn report_bodies(claim_id: &str) -> [(String, Value); 3] {
    [
        (
            format!("/v1/claims/{claim_id}/blocker"),
            serde_json::json!({ "requestId": rid(), "reason": "Espera" }),
        ),
        (
            format!("/v1/claims/{claim_id}/subtasks"),
            serde_json::json!({ "requestId": rid(), "title": "Hija" }),
        ),
        (
            format!("/v1/claims/{claim_id}/result"),
            serde_json::json!({ "requestId": rid(), "summary": "Hecho" }),
        ),
    ]
}

/// POST en crudo con cuerpo `chunked` (sin `Content-Length`) troceado.
async fn raw_chunked_post(port: u16, path: &str, headers: &[(&str, &str)], body: &[u8]) -> u16 {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\
         Content-Type: application/json\r\nTransfer-Encoding: chunked\r\n"
    );
    for (k, v) in headers {
        request.push_str(&format!("{k}: {v}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    for chunk in body.chunks(8 * 1024) {
        let mut piece = format!("{:x}\r\n", chunk.len()).into_bytes();
        piece.extend_from_slice(chunk);
        piece.extend_from_slice(b"\r\n");
        // El servidor puede cortar en cuanto supera el límite.
        if stream.write_all(&piece).await.is_err() {
            break;
        }
    }
    let _ = stream.write_all(b"0\r\n\r\n").await;
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response).await;
    let text = String::from_utf8_lossy(&response);
    text[9..12].parse().unwrap()
}

#[tokio::test]
async fn an_agent_reports_blockers_subtasks_and_results_and_the_host_is_told() {
    let w = world().await;
    let source = Recording::new(&w.store);
    let service = LocalService::start(w.data.path(), source.clone())
        .await
        .unwrap();
    let client = ServiceClient::new(w.data.path(), w.token.clone());
    let claim_id = claim_on(&client, &w.story.id).await;
    let changed = |ids: Vec<&str>| AgentChange {
        project_id: w.mine.id.clone(),
        story_ids: ids.into_iter().map(str::to_owned).collect(),
    };
    assert_eq!(source.take(), [changed(vec![&w.story.id])], "reclamar");

    // Bloqueo.
    let path = format!("/v1/claims/{claim_id}/blocker");
    let body = serde_json::json!({ "requestId": rid(), "reason": "Falta acceso" });
    let blocked = client.post_json(&path, &body).await.unwrap();
    assert_eq!(blocked["blocked"], true);
    assert_eq!(blocked["reason"], "Falta acceso");
    assert_eq!(blocked["storyNumber"], w.story.number);
    assert_eq!(source.take(), [changed(vec![&w.story.id])]);
    // Reintento idéntico → la misma respuesta; otro motivo con el mismo id → conflicto, sin aviso.
    assert_eq!(client.post_json(&path, &body).await.unwrap(), blocked);
    source.take();
    let reused = serde_json::json!({ "requestId": body["requestId"], "reason": "Otro" });
    let error = client.post_json(&path, &reused).await.unwrap_err();
    assert_eq!(api_code(&error), (409, "idempotency_conflict"));
    assert!(source.take().is_empty(), "un fallo no avisa");
    // Quitar el bloqueo propio: `reason` nulo u omitido.
    for body in [
        serde_json::json!({ "requestId": rid(), "reason": null }),
        serde_json::json!({ "requestId": rid() }),
    ] {
        let cleared = client.post_json(&path, &body).await.unwrap();
        assert_eq!(cleared["blocked"], false, "{body}");
    }
    source.take();

    // Subtarea: avisa de la historia reclamada y de la nueva.
    let path = format!("/v1/claims/{claim_id}/subtasks");
    let body = serde_json::json!({ "requestId": rid(), "title": "Hija", "description": "Texto" });
    let created = client.post_json(&path, &body).await.unwrap();
    assert_eq!(created["parentStoryId"], w.story.id.as_str());
    assert_eq!(created["title"], "Hija");
    let child = created["storyId"].as_str().unwrap();
    let notice = source.take();
    assert_eq!(notice.len(), 1);
    assert_eq!(notice[0].project_id, w.mine.id);
    let mut ids = notice[0].story_ids.clone();
    ids.sort();
    let mut expected = vec![w.story.id.clone(), child.to_owned()];
    expected.sort();
    assert_eq!(ids, expected);
    assert_eq!(client.post_json(&path, &body).await.unwrap(), created);
    source.take();
    let invalid = serde_json::json!({ "requestId": rid(), "title": "   " });
    let error = client.post_json(&path, &invalid).await.unwrap_err();
    assert_eq!(api_code(&error), (400, "validation"));
    assert!(source.take().is_empty());

    // Entrega: pendiente de revisión, cierra el reclamo.
    let path = format!("/v1/claims/{claim_id}/result");
    let body = serde_json::json!({
        "requestId": rid(),
        "summary": "Hecho",
        "references": [{ "kind": "url", "value": "https://example.com/pr/1" }],
    });
    let submitted = client.post_json(&path, &body).await.unwrap();
    assert_eq!(submitted["deliveryNumber"], 1);
    assert_eq!(submitted["storyId"], w.story.id.as_str());
    assert!(submitted["deliveryId"].is_string() && submitted["inReview"].is_boolean());
    assert_eq!(source.take(), [changed(vec![&w.story.id])]);
    // Reintento tras cerrar el reclamo: la entrega original, no una segunda.
    assert_eq!(client.post_json(&path, &body).await.unwrap(), submitted);
    source.take();
    // El reclamo ya no sirve: ninguna de las rutas escribe ni avisa.
    for (path, body) in report_bodies(&claim_id) {
        let error = client.post_json(&path, &body).await.unwrap_err();
        assert_eq!(api_code(&error), (409, "claim_ended"), "{path}");
    }
    assert!(source.take().is_empty());

    service.stop().await;
}

#[tokio::test]
async fn reports_only_work_on_the_agents_own_claim_in_its_project() {
    let w = world().await;
    let source = Recording::new(&w.store);
    let service = LocalService::start(w.data.path(), source.clone())
        .await
        .unwrap();
    let client = ServiceClient::new(w.data.path(), w.token.clone());

    // Reclamo del mismo agente en otro proyecto (otra credencial).
    let foreign = create_agent_credential(
        &w.store,
        &Actor::Human,
        &CreateAgentCredentialInput {
            project_id: w.other.id.clone(),
            agent_id: w.agent.id.clone(),
        },
    )
    .await
    .unwrap();
    let foreign_client = ServiceClient::new(w.data.path(), foreign.token);
    let foreign_claim = claim_on(&foreign_client, &w.foreign_story.id).await;
    // Reclamo de otro agente del mismo proyecto.
    let second = second_agent_client(&w).await;
    let theirs = story(&w.store, &w.mine, "Suya").await;
    let their_claim = claim_on(&second, &theirs.id).await;
    source.take();

    for claim_id in [&foreign_claim, &their_claim, &"0".repeat(36)] {
        for (path, body) in report_bodies(claim_id) {
            let error = client.post_json(&path, &body).await.unwrap_err();
            assert_eq!(api_code(&error), (404, "not_found"), "{path}");
        }
    }
    // Id demasiado largo: también «no encontrado», sin llegar al núcleo.
    for (path, body) in report_bodies(&"a".repeat(65)) {
        let error = client.post_json(&path, &body).await.unwrap_err();
        assert_eq!(api_code(&error), (404, "not_found"), "{path}");
    }
    assert!(source.take().is_empty(), "nada cambió");
    // Ni el proyecto ajeno ni la historia del otro agente cambiaron.
    for (project, story) in [(&w.other.id, &w.foreign_story.id), (&w.mine.id, &theirs.id)] {
        let review = kanbania_core::features::review::get_story_review(&w.store, project, story)
            .await
            .unwrap();
        assert!(review.blocker.is_none() && review.deliveries.is_empty());
    }

    // El agente y el proyecto salen de la sesión: campos así (o desconocidos) → 400.
    let mine = claim_on(&client, &w.story.id).await;
    source.take();
    let bodies = [
        (
            "blocker",
            serde_json::json!({ "requestId": rid(), "reason": "x", "agentId": w.agent.id }),
        ),
        (
            "blocker",
            serde_json::json!({ "requestId": rid(), "reason": "x", "claimId": their_claim }),
        ),
        (
            "subtasks",
            serde_json::json!({ "requestId": rid(), "title": "x", "projectId": w.other.id }),
        ),
        (
            "subtasks",
            serde_json::json!({ "requestId": rid(), "title": "x", "columnId": "c" }),
        ),
        (
            "result",
            serde_json::json!({ "requestId": rid(), "summary": "x", "agentId": w.agent.id }),
        ),
        (
            "result",
            serde_json::json!({ "requestId": rid(), "summary": "x", "accepted": true }),
        ),
        (
            "result",
            serde_json::json!({ "requestId": rid(), "summary": "x", "references": [
                { "kind": "url", "value": "https://a.example", "projectId": w.other.id }
            ] }),
        ),
        (
            "result",
            serde_json::json!({ "requestId": rid(), "summary": "x", "references": [
                { "kind": "ftp", "value": "ftp://a.example" }
            ] }),
        ),
        ("result", serde_json::json!({ "requestId": rid() })),
        ("subtasks", serde_json::json!({ "title": "x" })),
    ];
    for (action, body) in bodies {
        let error = client
            .post_json(&format!("/v1/claims/{mine}/{action}"), &body)
            .await
            .unwrap_err();
        assert_eq!(api_code(&error), (400, "validation"), "{action} {body}");
    }
    assert!(source.take().is_empty());

    service.stop().await;
}

#[tokio::test]
async fn reports_need_a_live_credential_and_bounded_bodies() {
    let w = world().await;
    let source = Recording::new(&w.store);
    let service = LocalService::start(w.data.path(), source.clone())
        .await
        .unwrap();
    let port = service.port();
    let client = ServiceClient::new(w.data.path(), w.token.clone());
    let claim_id = claim_on(&client, &w.story.id).await;
    source.take();

    // Cuerpo con `Content-Length` mayor que el límite → 413 antes de leerlo.
    let big = "x".repeat(kanbania_local_service::MAX_BODY_BYTES + 1);
    for (path, mut body) in report_bodies(&claim_id) {
        let field = if path.ends_with("blocker") {
            "reason"
        } else if path.ends_with("subtasks") {
            "description"
        } else {
            "summary"
        };
        body[field] = big.clone().into();
        let error = client.post_json(&path, &body).await.unwrap_err();
        assert_eq!(api_code(&error), (413, "validation"), "{path}");
    }
    // `chunked` sin longitud declarada: lo corta el extractor (`DefaultBodyLimit`).
    let session = session_token(w.data.path());
    let bearer = format!("Bearer {}", w.token);
    let auth = [
        ("X-Kanbania-Session", session.as_str()),
        ("Authorization", bearer.as_str()),
    ];
    let chunked =
        serde_json::to_vec(&serde_json::json!({ "requestId": rid(), "summary": big })).unwrap();
    let status = raw_chunked_post(
        port,
        &format!("/v1/claims/{claim_id}/result"),
        &auth,
        &chunked,
    )
    .await;
    assert_eq!(status, 413);
    // Un cuerpo `chunked` pequeño sí entra (el límite no rechaza `chunked` en sí).
    let small =
        serde_json::to_vec(&serde_json::json!({ "requestId": rid(), "reason": "Espera" })).unwrap();
    let status = raw_chunked_post(
        port,
        &format!("/v1/claims/{claim_id}/blocker"),
        &auth,
        &small,
    )
    .await;
    assert_eq!(status, 200);
    source.take();

    // Credencial revocada → 401 en las tres rutas; nada se escribe ni se avisa.
    revoke_agent_credential(
        &w.store,
        &Actor::Human,
        &RevokeAgentCredentialInput {
            project_id: w.mine.id.clone(),
            id: w.credential_id.clone(),
        },
    )
    .await
    .unwrap();
    for (path, body) in report_bodies(&claim_id) {
        let error = client.post_json(&path, &body).await.unwrap_err();
        assert_eq!(api_code(&error), (401, "unauthorized"), "{path}");
    }
    assert!(source.take().is_empty());
    let review =
        kanbania_core::features::review::get_story_review(&w.store, &w.mine.id, &w.story.id)
            .await
            .unwrap();
    assert!(review.deliveries.is_empty());

    service.stop().await;
}
