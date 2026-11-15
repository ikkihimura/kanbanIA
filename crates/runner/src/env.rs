//! Entorno de los procesos hijos: se vacía y solo se copian del padre unas pocas variables
//! necesarias para encontrar programas, la configuración del usuario y la carpeta temporal.
//! Nunca pasan `KANBANIA_*` (token del agente, directorio de datos…) ni otras credenciales.

use std::ffi::{OsStr, OsString};

#[cfg(unix)]
const ALLOWED: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "TMPDIR"];
// En Windows los nombres no distinguen mayúsculas (`var_os` tampoco). `ComSpec` no se copia:
// nada se lanza a través de `cmd.exe`.
#[cfg(windows)]
const ALLOWED: &[&str] = &[
    "PATH",
    "SystemRoot",
    "USERPROFILE",
    "HOME",
    "TEMP",
    "TMP",
    "LANG",
    "LC_ALL",
];

/// Variables que necesita además un adaptador ACP (HU-019): Claude Code usa `USER`/`LOGNAME`
/// para el llavero de macOS y `SHELL` para sus herramientas; en Windows, las carpetas del
/// perfil. Nunca `KANBANIA_*`, `ANTHROPIC_*` ni `CLAUDE_*`: la sesión iniciada de Claude Code
/// basta.
#[cfg(unix)]
const ADAPTER_EXTRA: &[&str] = &["USER", "LOGNAME", "SHELL"];
#[cfg(windows)]
const ADAPTER_EXTRA: &[&str] = &["USERPROFILE", "APPDATA", "LOCALAPPDATA"];

/// Prefijos que nunca pasan a un adaptador, aunque estén en una lista.
const ADAPTER_FORBIDDEN_PREFIXES: &[&str] = &["KANBANIA_", "ANTHROPIC_", "CLAUDE_"];

fn present(names: &[&str]) -> Vec<(OsString, OsString)> {
    names
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect()
}

/// Variables permitidas con el valor que tienen en este proceso (las ausentes se omiten).
pub(crate) fn allowed_env() -> Vec<(OsString, OsString)> {
    present(ALLOWED)
}

/// Variables extra de un adaptador ACP (se suman a [`allowed_env`]).
pub(crate) fn adapter_extra_env() -> Vec<(OsString, OsString)> {
    present(ADAPTER_EXTRA)
        .into_iter()
        .filter(|(name, _)| !is_adapter_forbidden(name))
        .collect()
}

/// `true` si el nombre no debe llegar nunca a un adaptador ACP (`KANBANIA_*`, `ANTHROPIC_*`,
/// `CLAUDE_*`, sin distinguir mayúsculas).
pub fn is_adapter_forbidden(name: &OsStr) -> bool {
    name.to_str().is_none_or(|name| {
        ADAPTER_FORBIDDEN_PREFIXES.iter().any(|prefix| {
            name.get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        })
    })
}

/// Nombres de las variables que puede recibir un adaptador (para pruebas y documentación).
pub fn adapter_env_names() -> Vec<&'static str> {
    ALLOWED.iter().chain(ADAPTER_EXTRA).copied().collect()
}

/// `true` si el nombre está reservado a Kanban IA (`KANBANIA_*`, sin distinguir mayúsculas):
/// nunca se entrega a un proceso hijo, aunque lo pida quien lo lanza.
pub fn is_reserved_env_name(name: &OsStr) -> bool {
    // Un nombre que no es Unicode tampoco se acepta.
    name.to_str().is_none_or(|name| {
        name.get(..9)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("KANBANIA_"))
    })
}

/// Nombre aceptable para una variable extra: no vacío, sin `=` ni NUL y no reservado.
pub(crate) fn valid_extra_name(name: &OsStr) -> bool {
    match name.to_str() {
        Some(text) => {
            !text.is_empty() && !text.contains(['=', '\0']) && !is_reserved_env_name(name)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_env_never_includes_credentials() {
        let names = adapter_env_names();
        #[cfg(unix)]
        for expected in ["PATH", "HOME", "USER", "LOGNAME", "SHELL"] {
            assert!(names.contains(&expected), "{expected}");
        }
        for name in names {
            assert!(!is_adapter_forbidden(OsStr::new(name)), "{name}");
            assert!(!is_reserved_env_name(OsStr::new(name)), "{name}");
        }
        for forbidden in [
            "KANBANIA_AGENT_TOKEN",
            "anthropic_api_key",
            "CLAUDE_CODE_OAUTH_TOKEN",
        ] {
            assert!(is_adapter_forbidden(OsStr::new(forbidden)), "{forbidden}");
        }
        assert!(!is_adapter_forbidden(OsStr::new("PATH")));
    }
}
