//! Descriptor del servicio: `service.json` en el directorio de datos, legible solo por el
//! usuario (0600 en Unix). Dice a los clientes locales en qué puerto escucha el host y con qué
//! token de sesión (nuevo en cada arranque) deben presentarse. Nunca se registra en logs.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DESCRIPTOR_FILE: &str = "service.json";
const FORMAT: &str = "kanbania-service";
const VERSION: u32 = 1;
/// Un descriptor real ocupa unos cientos de bytes.
const MAX_BYTES: u64 = 4096;

/// Secreto que no aparece en `Debug` (ni, por tanto, en logs o mensajes de error).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// 32 bytes aleatorios del sistema en hexadecimal.
    pub fn random() -> std::io::Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(Self(hex(&bytes)))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Comparación en tiempo constante (para tokens de igual longitud).
    pub fn matches(&self, candidate: &[u8]) -> bool {
        let own = self.0.as_bytes();
        own.len() == candidate.len()
            && own
                .iter()
                .zip(candidate)
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<oculto>)")
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Descriptor {
    pub format: String,
    pub version: u32,
    pub port: u16,
    pub session_token: Secret,
    pub pid: u32,
}

impl Descriptor {
    pub fn new(port: u16, session_token: Secret) -> Self {
        Self {
            format: FORMAT.into(),
            version: VERSION,
            port,
            session_token,
            pid: std::process::id(),
        }
    }
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(DESCRIPTOR_FILE)
}

#[derive(Debug)]
pub enum ReadError {
    /// No existe: la app no está abierta (o nunca arrancó el servicio con este perfil).
    Missing,
    /// En Unix, legible o escribible por otros usuarios: no se confía en él.
    Insecure,
    /// Formato desconocido, versión futura o JSON roto.
    Invalid,
    Io(std::io::Error),
}

/// Escribe el descriptor de forma atómica y con permisos solo del usuario.
pub fn write(data_dir: &Path, descriptor: &Descriptor) -> std::io::Result<()> {
    let target = path(data_dir);
    let staging = data_dir.join(format!("{DESCRIPTOR_FILE}.tmp"));
    match std::fs::remove_file(&staging) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&staging)?;
    file.write_all(&serde_json::to_vec_pretty(descriptor)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&staging, &target)
}

pub fn read(data_dir: &Path) -> Result<Descriptor, ReadError> {
    let target = path(data_dir);
    let metadata = match std::fs::symlink_metadata(&target) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ReadError::Missing),
        Err(e) => return Err(ReadError::Io(e)),
    };
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(ReadError::Invalid);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // Solo el propietario, y el mismo que el del directorio de datos (que es del usuario).
        let owner_only = metadata.permissions().mode() & 0o077 == 0;
        let same_user = std::fs::metadata(data_dir).is_ok_and(|dir| dir.uid() == metadata.uid());
        if !owner_only || !same_user {
            return Err(ReadError::Insecure);
        }
    }
    let bytes = std::fs::read(&target).map_err(ReadError::Io)?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).map_err(|_| ReadError::Invalid)?;
    if descriptor.format != FORMAT || descriptor.version != VERSION {
        return Err(ReadError::Invalid);
    }
    Ok(descriptor)
}

/// Borra el descriptor solo si sigue siendo el nuestro (otro arranque pudo reemplazarlo).
pub fn remove_if_ours(data_dir: &Path, session_token: &Secret) {
    if let Ok(current) = read(data_dir)
        && current.session_token == *session_token
    {
        let _ = std::fs::remove_file(path(data_dir));
    }
}
