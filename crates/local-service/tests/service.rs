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
        .get_json(&format!("/v1/stories/{}", w.story.id))
        .await
        .unwrap();
    assert_eq!(story["title"], "Tarea mía");

    // Historia de otro proyecto: no encontrada, sin revelar que existe.
    let error = client
        .get_json(&format!("/v1/stories/{}", w.foreign_story.id))
        .await
        .unwrap_err();
    let ClientError::Api { status, code, .. } = error else {
        panic!("{error:?}");
    };
    assert_eq!((status, code.as_str()), (404, "not_found"));
    // Tampoco sirve pasar el proyecto en la petición: ninguna ruta lo acepta.
    let error = client
        .get_json(&format!(
            "/v1/stories/{}?projectId={}",
            w.foreign_story.id, w.other.id
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, ClientError::Api { status: 404, .. }));

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
    for path in ["/v1/session", &format!("/v1/stories/{}", w.story.id)] {
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
    for path in ["/v1/sql", "/v1/exec", "/v1/projects", "/"] {
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
