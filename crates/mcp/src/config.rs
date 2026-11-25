//! Configuración por variables de entorno (la pone el cliente MCP en su `env`).
//!
//! - `KANBANIA_AGENT_TOKEN` (obligatoria): credencial creada en «Acceso de agentes». Nunca
//!   como argumento: los argumentos de un proceso los ven otros usuarios con `ps`.
//! - `KANBANIA_DATA_DIR` (opcional): directorio de datos de la app, si no es el de por defecto.

use std::fmt;
use std::path::PathBuf;

pub const TOKEN_ENV: &str = "KANBANIA_AGENT_TOKEN";
pub const DATA_DIR_ENV: &str = "KANBANIA_DATA_DIR";
/// Identificador de la app (mismo que `tauri.conf.json`): nombre de su directorio de datos.
const APP_IDENTIFIER: &str = "dev.ikkidev.kanbania";

pub struct Config {
    pub data_dir: PathBuf,
    pub token: String,
}

/// Errores de arranque. Sus mensajes nunca incluyen el token.
#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    MissingToken,
    MalformedToken,
    NoDataDir,
    RelativeDataDir,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingToken => write!(
                f,
                "Falta {TOKEN_ENV}. Crea una credencial en Kanban IA (configuración del proyecto \
                 → «Acceso de agentes») y ponla en el \"env\" de la configuración MCP."
            ),
            Self::MalformedToken => write!(
                f,
                "{TOKEN_ENV} no tiene el formato de una credencial de Kanban IA (kbia_…). \
                 Cópiala de nuevo o crea otra."
            ),
            Self::RelativeDataDir => write!(
                f,
                "{DATA_DIR_ENV} debe ser una ruta absoluta (el cliente MCP puede lanzar este \
                 proceso desde cualquier carpeta)."
            ),
            Self::NoDataDir => write!(
                f,
                "No se pudo determinar el directorio de datos de Kanban IA. Indícalo con \
                 {DATA_DIR_ENV}."
            ),
        }
    }
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(|name| std::env::var_os(name).and_then(|v| v.into_string().ok()))
    }

    /// Separado de `from_env` para probarlo sin tocar el entorno del proceso.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let token = var(TOKEN_ENV)
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .ok_or(ConfigError::MissingToken)?;
        if !well_formed(&token) {
            return Err(ConfigError::MalformedToken);
        }
        let data_dir = match var(DATA_DIR_ENV).filter(|d| !d.is_empty()) {
            Some(dir) if PathBuf::from(&dir).is_absolute() => PathBuf::from(dir),
            Some(_) => return Err(ConfigError::RelativeDataDir),
            None => default_data_dir(&var).ok_or(ConfigError::NoDataDir)?,
        };
        Ok(Self { data_dir, token })
    }
}

/// Misma forma que valida el núcleo (`kbia_` + 64 hex). Comprobarlo aquí da un error claro al
/// arrancar en vez de un «no autorizado» en cada llamada.
fn well_formed(token: &str) -> bool {
    token.len() == 69
        && token.starts_with("kbia_")
        && token[5..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Directorio de datos por defecto, como lo resuelve Tauri (`app_data_dir`).
fn default_data_dir(var: &impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        var("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library/Application Support")
                .join(APP_IDENTIFIER)
        })
    } else if cfg!(windows) {
        var("APPDATA").map(|dir| PathBuf::from(dir).join(APP_IDENTIFIER))
    } else {
        var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".local/share")))
            .map(|dir| dir.join(APP_IDENTIFIER))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "kbia_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn token_is_required_and_must_look_like_a_credential() {
        let missing = Config::from_vars(vars(&[("HOME", "/h")])).err();
        assert_eq!(missing, Some(ConfigError::MissingToken));
        for bad in ["x", "Bearer kbia_", &TOKEN.to_uppercase(), &TOKEN[1..]] {
            let error = Config::from_vars(vars(&[(TOKEN_ENV, bad), ("HOME", "/h")])).err();
            assert_eq!(error, Some(ConfigError::MalformedToken), "{bad}");
            // El mensaje no repite lo recibido.
            assert!(!ConfigError::MalformedToken.to_string().contains(bad));
        }
    }

    #[test]
    fn data_dir_defaults_to_the_app_one_and_can_be_overridden() {
        let config = Config::from_vars(vars(&[
            (TOKEN_ENV, TOKEN),
            ("HOME", "/Users/yo"),
            ("APPDATA", "C:\\Users\\yo\\AppData\\Roaming"),
            ("XDG_DATA_HOME", "/xdg"),
        ]))
        .ok()
        .unwrap();
        let expected = if cfg!(target_os = "macos") {
            PathBuf::from("/Users/yo/Library/Application Support/dev.ikkidev.kanbania")
        } else if cfg!(windows) {
            PathBuf::from("C:\\Users\\yo\\AppData\\Roaming").join("dev.ikkidev.kanbania")
        } else {
            PathBuf::from("/xdg/dev.ikkidev.kanbania")
        };
        assert_eq!(config.data_dir, expected);

        let absolute = if cfg!(windows) {
            "C:\\perfil"
        } else {
            "/perfil"
        };
        let config = Config::from_vars(vars(&[(TOKEN_ENV, TOKEN), (DATA_DIR_ENV, absolute)]))
            .ok()
            .unwrap();
        assert_eq!(config.data_dir, PathBuf::from(absolute));
        if cfg!(unix) {
            let relative = Config::from_vars(vars(&[(TOKEN_ENV, TOKEN), (DATA_DIR_ENV, "perfil")]));
            assert_eq!(relative.err(), Some(ConfigError::RelativeDataDir));
        }
    }

    #[test]
    fn app_identifier_matches_the_desktop_app() {
        let conf: serde_json::Value = serde_json::from_str(include_str!(
            "../../../apps/desktop/src-tauri/tauri.conf.json"
        ))
        .unwrap();
        assert_eq!(
            conf["identifier"], APP_IDENTIFIER,
            "directorio de datos por defecto"
        );
    }
}
