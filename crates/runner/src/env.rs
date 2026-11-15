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

/// Variables permitidas con el valor que tienen en este proceso (las ausentes se omiten).
pub(crate) fn allowed_env() -> Vec<(OsString, OsString)> {
    ALLOWED
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect()
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
