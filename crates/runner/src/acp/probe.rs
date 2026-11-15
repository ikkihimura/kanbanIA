//! «Comprobar adaptador» (HU-019): lanza el adaptador con el mismo entorno que una ejecución,
//! hace solo `initialize` y termina su árbol. No crea sesión ni envía instrucciones.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use super::client::{AcpClient, AgentDeclaration};
use super::{AcpFailure, AcpLimits, AcpStop};
use crate::process::{ProcessSpec, RunningProcess, SpawnErrorKind, cancel_pair};

/// Gracia al terminar el adaptador comprobado.
const PROBE_GRACE: Duration = Duration::from_secs(2);

/// Por qué falló la comprobación. `Display` es un mensaje seguro (sin rutas ni texto del
/// agente).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    /// Ruta o carpeta no válidas.
    InvalidSpec,
    NotFound,
    PermissionDenied,
    SpawnFailed,
    TimedOut,
    Disconnected,
    Protocol,
    VersionUnsupported,
    RequestFailed,
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidSpec => "La ruta del adaptador no es válida.",
            Self::NotFound => {
                "No se pudo ejecutar el adaptador: no existe, o le falta su intérprete (p. ej. \
                 `node` en el PATH con el que se abrió la app)."
            }
            Self::PermissionDenied => "No hay permiso para ejecutar el adaptador.",
            Self::SpawnFailed => "No se pudo iniciar el adaptador.",
            Self::TimedOut => "El adaptador no respondió a tiempo a «initialize».",
            Self::Disconnected => {
                "El adaptador terminó sin responder a «initialize» (¿es un adaptador ACP? ¿encuentra \
                 `node`?)."
            }
            Self::Protocol => "El adaptador no respondió con ACP válido.",
            Self::VersionUnsupported => {
                "El adaptador no habla una versión de ACP compatible (se requiere la 1)."
            }
            Self::RequestFailed => "El adaptador devolvió un error a «initialize».",
        })
    }
}

impl std::error::Error for ProbeError {}

/// Lanza `program` (absoluto, sin shell) en `cwd`, hace `initialize` con el límite `limit` y
/// termina su árbol. `args` solo se usa en pruebas (la app no pasa argumentos).
pub async fn probe_adapter(
    program: PathBuf,
    args: Vec<OsString>,
    cwd: PathBuf,
    limit: Duration,
) -> Result<AgentDeclaration, ProbeError> {
    let spec = ProcessSpec {
        program,
        args,
        cwd,
        extra_env: Vec::new(),
        timeout: limit,
        grace: PROBE_GRACE,
    };
    let (process, io) =
        RunningProcess::spawn_protocol_unlogged(spec).map_err(|error| match error.kind {
            SpawnErrorKind::InvalidSpec => ProbeError::InvalidSpec,
            SpawnErrorKind::NotFound => ProbeError::NotFound,
            SpawnErrorKind::PermissionDenied => ProbeError::PermissionDenied,
            SpawnErrorKind::Other => ProbeError::SpawnFailed,
        })?;
    let limits = AcpLimits {
        request_timeout: limit,
        ..AcpLimits::default()
    };
    let mut client = AcpClient::new(io, limits, None);
    // Nadie cancela la comprobación: el límite de tiempo basta.
    let (_keep, mut token) = cancel_pair();
    let result = client.initialize(&mut token, process.deadline()).await;
    client.close_input();
    drop(client);
    let _ = process.close(Duration::ZERO).await;
    result.map_err(|stop| match stop {
        AcpStop::TimedOut => ProbeError::TimedOut,
        AcpStop::Cancelled => ProbeError::RequestFailed,
        AcpStop::Failed(AcpFailure::Disconnected) => ProbeError::Disconnected,
        AcpStop::Failed(AcpFailure::VersionUnsupported) => ProbeError::VersionUnsupported,
        AcpStop::Failed(AcpFailure::Protocol) => ProbeError::Protocol,
        AcpStop::Failed(_) => ProbeError::RequestFailed,
    })
}
