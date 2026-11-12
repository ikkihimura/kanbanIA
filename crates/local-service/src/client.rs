//! Cliente del servicio local (lo usará el adaptador MCP en HU-013).
//!
//! Nunca abre SQLite: si la app no está abierta, falla con [`ClientError::NotRunning`] en vez
//! de convertirse en otro propietario de los datos. Antes de enviar la credencial del agente,
//! comprueba con un reto que el servidor conoce el token de sesión del descriptor.

use std::fmt;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Bytes;
use hyper::{Request, header};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::TcpStream;

use crate::descriptor::{self, ReadError, Secret};
use crate::protocol::{API_VERSION, NONCE_HEADER, SESSION_HEADER, proof};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub enum ClientError {
    /// La app no está abierta con este directorio de datos (o se cerró durante la petición).
    NotRunning,
    /// El descriptor tiene permisos de otros usuarios: no se usa.
    InsecureDescriptor,
    /// Quien responde en el puerto no demostró conocer el token: no se le envía nada más.
    Untrusted,
    /// La app abierta habla otra versión de la API.
    Incompatible { server: u64, client: u32 },
    /// Respuesta o descriptor ilegibles.
    Protocol(String),
    /// Error del servicio (código y mensaje seguros del núcleo).
    Api {
        status: u16,
        code: String,
        message: String,
        /// Reintentar (o que la persona actúe) puede resolverlo.
        recoverable: bool,
    },
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning => f.write_str(
                "Kanban IA no está abierto con este directorio de datos. Ábrelo y vuelve a \
                 intentarlo.",
            ),
            Self::InsecureDescriptor => f.write_str(
                "El descriptor del servicio local tiene permisos de otros usuarios y no se usará. \
                 Cierra Kanban IA y vuelve a abrirlo.",
            ),
            Self::Untrusted => f.write_str(
                "El proceso que responde en el puerto del servicio no es Kanban IA. No se envió \
                 la credencial.",
            ),
            Self::Incompatible { server, client } => write!(
                f,
                "La versión de Kanban IA abierta usa la API v{server} y esta integración la \
                 v{client}. Actualiza la que sea más antigua."
            ),
            Self::Protocol(detail) => write!(f, "Respuesta no válida del servicio local: {detail}"),
            Self::Api { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for ClientError {}

pub struct ServiceClient {
    data_dir: PathBuf,
    agent_token: Secret,
}

impl fmt::Debug for ServiceClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceClient")
            .field("data_dir", &self.data_dir)
            .field("agent_token", &self.agent_token)
            .finish()
    }
}

impl ServiceClient {
    pub fn new(data_dir: &Path, agent_token: String) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            agent_token: Secret::new(agent_token),
        }
    }

    /// `GET` autenticado a una ruta de la API (`/v1/...`). Lee el descriptor en cada llamada:
    /// si la app se reinició, usa el puerto y el token nuevos.
    pub async fn get_json(&self, path: &str) -> Result<Value, ClientError> {
        tokio::time::timeout(REQUEST_TIMEOUT, self.get_inner(path))
            .await
            .map_err(|_| ClientError::Protocol("tiempo de espera agotado".into()))?
    }

    async fn get_inner(&self, path: &str) -> Result<Value, ClientError> {
        if !path.starts_with("/v1/") {
            return Err(ClientError::Protocol("ruta fuera de la API".into()));
        }
        let descriptor = match descriptor::read(&self.data_dir) {
            Ok(d) => d,
            Err(ReadError::Missing) => return Err(ClientError::NotRunning),
            Err(ReadError::Insecure) => return Err(ClientError::InsecureDescriptor),
            Err(ReadError::Invalid) => {
                return Err(ClientError::Protocol("descriptor no válido".into()));
            }
            Err(ReadError::Io(e)) => return Err(ClientError::Protocol(e.to_string())),
        };
        let host = format!("127.0.0.1:{}", descriptor.port);
        let stream = match tokio::time::timeout(
            CONNECT_TIMEOUT,
            TcpStream::connect((Ipv4Addr::LOCALHOST, descriptor.port)),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            // Descriptor huérfano (la app se cerró de golpe) o puerto que no responde.
            _ => return Err(ClientError::NotRunning),
        };
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|_| ClientError::NotRunning)?;
        tokio::spawn(async move {
            let _ = connection.await;
        });

        // 1. Reto: el servidor debe demostrar que conoce el token de sesión.
        let nonce = Secret::random()
            .map_err(|e| ClientError::Protocol(e.to_string()))?
            .expose()
            .to_owned();
        let request = Request::get("/v1/handshake")
            .header(header::HOST, &host)
            .header(NONCE_HEADER, &nonce)
            .body(Empty::<Bytes>::new())
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        let (status, body) = send(&mut sender, request).await?;
        let expected = proof(&descriptor.session_token, &nonce);
        if status != 200 || body.get("proof").and_then(Value::as_str) != Some(expected.as_str()) {
            return Err(ClientError::Untrusted);
        }
        let server_version = body.get("apiVersion").and_then(Value::as_u64);
        if server_version != Some(u64::from(API_VERSION)) {
            return Err(ClientError::Incompatible {
                server: server_version.unwrap_or(0),
                client: API_VERSION,
            });
        }

        // 2. Petición con las credenciales.
        let request = Request::get(path)
            .header(header::HOST, &host)
            .header(SESSION_HEADER, descriptor.session_token.expose())
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", self.agent_token.expose()),
            )
            .body(Empty::<Bytes>::new())
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        let (status, body) = send(&mut sender, request).await?;
        if (200..300).contains(&status) {
            return Ok(body);
        }
        let text = |key: &str| {
            body.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Err(ClientError::Api {
            status,
            code: text("code"),
            message: text("message"),
            recoverable: body
                .get("recoverable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

async fn send(
    sender: &mut hyper::client::conn::http1::SendRequest<Empty<Bytes>>,
    request: Request<Empty<Bytes>>,
) -> Result<(u16, Value), ClientError> {
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| ClientError::NotRunning)?;
    let status = response.status().as_u16();
    let bytes = Limited::new(response.into_body(), MAX_RESPONSE_BYTES)
        .collect()
        .await
        .map_err(|_| ClientError::Protocol("respuesta demasiado grande o cortada".into()))?
        .to_bytes();
    let value = serde_json::from_slice(&bytes)
        .map_err(|_| ClientError::Protocol(format!("respuesta {status} sin JSON")))?;
    Ok((status, value))
}
