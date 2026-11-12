//! Servidor HTTP del servicio local: solo `127.0.0.1`, puerto dinámico, sin CORS.
//!
//! Cada petición pasa por:
//! 1. `guard`: `Host` exacto (contra DNS rebinding), sin cabecera `Origin` (no se atiende a
//!    navegadores) y tamaño de cuerpo acotado.
//! 2. `authenticate` (todas las rutas salvo el saludo): token de sesión del descriptor, que
//!    prueba que el cliente es el usuario local, y credencial del agente, de la que salen la
//!    identidad y el proyecto. Ninguna ruta acepta un proyecto o un agente en la petición.
//!
//! Las rutas solo llaman a casos de uso del núcleo: no hay SQL ni shell genéricos.

use std::future::Future;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::rejection::QueryRejection;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use kanbania_core::error::{CoreError, ErrorCode, ErrorPayload};
use kanbania_core::features::access::{AgentSession, authenticate_agent, describe_session};
use kanbania_core::features::agent_work::{
    ReadyQuery, StoryKey, get_story_context, list_ready_stories,
};
use kanbania_core::platform::storage::Store;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::descriptor::{self, Descriptor, Secret};
use crate::protocol::{API_VERSION, MAX_BODY_BYTES, NONCE_HEADER, SESSION_HEADER, proof};

const MAX_ID_LEN: usize = 64;

type StoreFuture<'a> = Pin<Box<dyn Future<Output = Result<Store, ErrorPayload>> + Send + 'a>>;

/// De dónde sale el almacén en cada petición. El host lo implementa con su `StorageSlot`:
/// así el servicio usa el único propietario de datos y respeta cierres y restauraciones.
pub trait StoreSource: Send + Sync + 'static {
    fn store(&self) -> StoreFuture<'_>;
}

struct Shared {
    source: Arc<dyn StoreSource>,
    session_token: Secret,
    port: u16,
}

/// Petición autenticada: sesión del agente y almacén que la atiende.
#[derive(Clone)]
struct Authed {
    session: AgentSession,
    store: Store,
}

/// Servicio en marcha. Al pararlo, borra su descriptor (si sigue siendo el suyo).
pub struct LocalService {
    port: u16,
    session_token: Secret,
    data_dir: PathBuf,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl LocalService {
    /// Escucha en `127.0.0.1` con un puerto libre y publica el descriptor en `data_dir`.
    /// Solo debe llamarlo quien posee el almacén (el bloqueo del directorio de datos).
    pub async fn start(data_dir: &Path, source: Arc<dyn StoreSource>) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        let session_token = Secret::random()?;
        let router = router(Arc::new(Shared {
            source,
            session_token: session_token.clone(),
            port,
        }));
        let (tx, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let server = axum::serve(listener, router).with_graceful_shutdown(async {
                let _ = rx.await;
            });
            if let Err(error) = server.await {
                eprintln!("[local-service] el servidor terminó con error: {error}");
            }
        });
        let service = Self {
            port,
            session_token,
            data_dir: data_dir.to_path_buf(),
            shutdown: Some(tx),
            task,
        };
        if let Err(error) = descriptor::write(
            data_dir,
            &Descriptor::new(port, service.session_token.clone()),
        ) {
            service.stop().await;
            return Err(error);
        }
        Ok(service)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Deja de aceptar conexiones, espera un plazo corto a las peticiones en curso y borra el
    /// descriptor.
    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if tokio::time::timeout(Duration::from_secs(2), &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
        }
        descriptor::remove_if_ours(&self.data_dir, &self.session_token);
    }
}

fn router(shared: Arc<Shared>) -> Router {
    let authenticated = Router::new()
        .route("/v1/session", get(session))
        .route("/v1/stories/ready", get(ready_stories))
        .route("/v1/stories/{id}/context", get(context_by_id))
        .route(
            "/v1/stories/by-number/{number}/context",
            get(context_by_number),
        )
        .route_layer(from_fn_with_state(shared.clone(), authenticate));
    Router::new()
        .route("/v1/handshake", get(handshake))
        .merge(authenticated)
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(from_fn_with_state(shared.clone(), guard))
        .with_state(shared)
}

// ---------- Capas ----------

async fn guard(State(shared): State<Arc<Shared>>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let allowed = [
        format!("127.0.0.1:{}", shared.port),
        format!("localhost:{}", shared.port),
    ];
    if !host.is_some_and(|h| allowed.iter().any(|a| a == h)) {
        return forbidden("Host no permitido.");
    }
    if headers.contains_key(header::ORIGIN) {
        return forbidden(
            "Origen no permitido: el servicio local no atiende peticiones de navegadores.",
        );
    }
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|len| len > MAX_BODY_BYTES as u64) {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            CoreError::Validation {
                field: "body",
                reason: format!(
                    "La petición supera el tamaño máximo ({} KB).",
                    MAX_BODY_BYTES / 1024
                ),
            }
            .to_payload(None),
        );
    }
    next.run(request).await
}

async fn authenticate(
    State(shared): State<Arc<Shared>>,
    mut request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    let session_ok = headers
        .get(SESSION_HEADER)
        .is_some_and(|v| shared.session_token.matches(v.as_bytes()));
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned);
    let (true, Some(token)) = (session_ok, token) else {
        return core_error(&CoreError::Unauthorized);
    };
    let store = match shared.source.store().await {
        Ok(store) => store,
        Err(payload) => return payload_response(payload),
    };
    match authenticate_agent(&store, &token).await {
        Ok(session) => {
            request.extensions_mut().insert(Authed { session, store });
            next.run(request).await
        }
        Err(error) => core_error(&error),
    }
}

// ---------- Rutas ----------

/// Saludo sin credenciales: el servidor demuestra conocer el token de sesión respondiendo al
/// reto del cliente. Así un cliente con un descriptor caducado no entrega la credencial del
/// agente a otro proceso que ocupe ese puerto.
async fn handshake(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    let nonce = headers.get(NONCE_HEADER).and_then(|v| v.to_str().ok());
    let Some(nonce) =
        nonce.filter(|n| (32..=128).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return error_response(
            StatusCode::BAD_REQUEST,
            CoreError::Validation {
                field: "nonce",
                reason: "Falta el reto del saludo o no es válido.".into(),
            }
            .to_payload(None),
        );
    };
    Json(serde_json::json!({
        "apiVersion": API_VERSION,
        "proof": proof(&shared.session_token, nonce),
    }))
    .into_response()
}

/// Quién es esta sesión y a qué proyecto da acceso.
async fn session(Extension(authed): Extension<Authed>) -> Response {
    match describe_session(&authed.store, &authed.session).await {
        Ok(info) => Json(info).into_response(),
        Err(error) => core_error(&error),
    }
}

/// Trabajo listo del proyecto de la sesión (`?cursor=&limit=`). Parámetros desconocidos —p. ej.
/// un `projectId`— se rechazan: el proyecto sale siempre de la sesión.
async fn ready_stories(
    Extension(authed): Extension<Authed>,
    query: Result<Query<ReadyQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return error_response(
            StatusCode::BAD_REQUEST,
            CoreError::Validation {
                field: "query",
                reason: "Parámetros no válidos: solo se admiten cursor y limit (1-50).".into(),
            }
            .to_payload(None),
        );
    };
    match list_ready_stories(&authed.store, &authed.session, &query).await {
        Ok(page) => Json(page).into_response(),
        Err(error) => core_error(&error),
    }
}

/// Contexto de una historia del proyecto de la sesión. Otro proyecto → «no encontrada».
async fn context_by_id(
    Extension(authed): Extension<Authed>,
    UrlPath(id): UrlPath<String>,
) -> Response {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return core_error(&CoreError::NotFound { entity: "Historia" });
    }
    story_context(&authed, StoryKey::Id(id)).await
}

async fn context_by_number(
    Extension(authed): Extension<Authed>,
    UrlPath(number): UrlPath<String>,
) -> Response {
    match number.parse::<i64>() {
        Ok(n) if n >= 1 => story_context(&authed, StoryKey::Number(n)).await,
        _ => core_error(&CoreError::NotFound { entity: "Historia" }),
    }
}

async fn story_context(authed: &Authed, key: StoryKey) -> Response {
    match get_story_context(&authed.store, &authed.session, &key).await {
        Ok(context) => Json(context).into_response(),
        Err(error) => core_error(&error),
    }
}

async fn not_found() -> Response {
    core_error(&CoreError::NotFound { entity: "Ruta" })
}

// ---------- Errores ----------

fn forbidden(reason: &str) -> Response {
    core_error(&CoreError::Forbidden {
        reason: reason.into(),
    })
}

fn core_error(error: &CoreError) -> Response {
    if matches!(
        error,
        CoreError::Database(_) | CoreError::StorageIo(_) | CoreError::Serialization(_)
    ) {
        // Diagnóstico local; el cliente solo recibe el mensaje seguro.
        eprintln!("[local-service] {error}");
    }
    payload_response(error.to_payload(None))
}

fn payload_response(payload: ErrorPayload) -> Response {
    error_response(status_for(payload.code), payload)
}

fn error_response(status: StatusCode, payload: ErrorPayload) -> Response {
    (status, Json(payload)).into_response()
}

fn status_for(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::Validation | ErrorCode::Cycle => StatusCode::BAD_REQUEST,
        ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
        ErrorCode::Forbidden => StatusCode::FORBIDDEN,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::AlreadyExists
        | ErrorCode::Conflict
        | ErrorCode::IdempotencyConflict
        | ErrorCode::WipLimitExceeded
        | ErrorCode::AcceptanceRequired => StatusCode::CONFLICT,
        ErrorCode::StorageLocked
        | ErrorCode::StorageNewerSchema
        | ErrorCode::StorageMigrationFailed
        | ErrorCode::StorageUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
