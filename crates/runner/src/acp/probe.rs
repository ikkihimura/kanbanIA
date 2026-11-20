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
/// Límite de la comprobación de `node` (HU-026).
const NODE_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Por qué falló la comprobación. `Display` es un mensaje seguro (sin rutas ni texto del
/// agente).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    /// Ruta o carpeta no válidas.
    InvalidSpec,
    NotFound,
    /// El adaptador es un script `#!/usr/bin/env node` y no hay `node` en el `PATH` que
    /// recibiría (HU-026).
    NodeNotFound,
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
                "No se pudo ejecutar el adaptador: no existe, o le falta su intérprete."
            }
            Self::NodeNotFound => {
                "El adaptador necesita node y no lo encuentra: la app no tiene el PATH de tu \
                 terminal (pasa al abrirla desde Finder o el menú Inicio con Node de nvm). Indica \
                 la ruta de node (la que muestra `which node` en una terminal, o pulsa «Buscar \
                 node») y vuelve a comprobar."
            }
            Self::PermissionDenied => "No hay permiso para ejecutar el adaptador.",
            Self::SpawnFailed => "No se pudo iniciar el adaptador.",
            Self::TimedOut => "El adaptador no respondió a tiempo a «initialize».",
            Self::Disconnected => {
                "El adaptador terminó sin responder a «initialize» (¿es un adaptador ACP?). \
                 Consulta su salida de errores en una terminal."
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
/// termina su árbol. `args` solo se usa en pruebas (la app no pasa argumentos). `extra_env`:
/// las variables propias del agente, las mismas que en una ejecución (Codex: [`super::codex_env`]),
/// y el `PATH` con la carpeta de `node` si hay una configurada ([`super::node_path_env`]). Antes
/// de lanzar, un script `#!/usr/bin/env node` sin `node` en ese `PATH` → [`ProbeError::NodeNotFound`].
pub async fn probe_adapter(
    program: PathBuf,
    args: Vec<OsString>,
    cwd: PathBuf,
    extra_env: Vec<(OsString, OsString)>,
    limit: Duration,
) -> Result<AgentDeclaration, ProbeError> {
    let path_var = super::effective_path(&extra_env);
    let check = program.clone();
    // Si el disco no responde a tiempo, lo decide el lanzamiento.
    let missing = tokio::time::timeout(
        NODE_CHECK_TIMEOUT,
        tokio::task::spawn_blocking(move || super::missing_node(&check, path_var.as_deref())),
    )
    .await
    .is_ok_and(|checked| checked.unwrap_or(false));
    if missing {
        return Err(ProbeError::NodeNotFound);
    }
    let spec = ProcessSpec {
        program,
        args,
        cwd,
        extra_env,
        timeout: limit,
        grace: PROBE_GRACE,
        // Sin guardián (HU-024): solo `initialize`, sin sesión ni trabajo, ≤ 30 s; si la app
        // muere, el adaptador ve cerrada su entrada.
        guard: None,
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
