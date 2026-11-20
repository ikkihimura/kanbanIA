//! `node` para los adaptadores npm (HU-026). Los adaptadores (`claude-agent-acp`, `codex-acp`)
//! son scripts `#!/usr/bin/env node`: con Node de nvm, una app abierta desde Finder o el menú
//! Inicio no lo tiene en el `PATH` y el adaptador sale con 127 sin hablar ACP.
//!
//! - [`node_path_env`]: con una ruta de `node` configurada, el `PATH` del adaptador es su
//!   carpeta seguida del `PATH` de la app. Es la única variable que cambia: el entorno permitido
//!   no crece (sigue siendo la lista de `env.rs`).
//! - [`missing_node`]: antes de lanzar, lee la primera línea del adaptador y, si pide `node` a
//!   través de `env`, lo busca en ese `PATH` igual que haría `env` (sin ejecutar nada). Así
//!   «Comprobar adaptador» y el intento dicen «no encuentro node» en vez de un fallo genérico.

use std::ffi::{OsStr, OsString};
use std::path::Path;

/// Bytes que se leen del adaptador para encontrar su primera línea.
const SHEBANG_BYTES: usize = 256;

/// Variable `PATH` para el adaptador con la carpeta de `node` delante del `PATH` de la app (sin
/// sus entradas relativas, como en `allowed_env`), o `None` sin ruta de node, con una ruta no
/// absoluta o si no se puede componer (carpeta con el separador del `PATH`; el núcleo ya la
/// rechaza al guardar).
pub fn node_path_env(node: Option<&Path>) -> Option<(OsString, OsString)> {
    let dir = node.filter(|node| node.is_absolute())?.parent()?;
    let inherited = crate::env::inherited_path();
    let rest = inherited.iter().flat_map(std::env::split_paths);
    let joined = std::env::join_paths(std::iter::once(dir.to_path_buf()).chain(rest)).ok()?;
    Some(("PATH".into(), joined))
}

/// `true` si la ruta de node configurada no sirve: no es un archivo ejecutable (p. ej. se
/// desinstaló esa versión con nvm) o no se puede poner en el `PATH`. E/S bloqueante y breve.
pub fn configured_node_unusable(node: &Path) -> bool {
    !is_executable_file(node) || node_path_env(Some(node)).is_none()
}

/// El `PATH` que recibirá el proceso: el de `extra_env` si lo cambia, si no el de la app.
pub fn effective_path(extra_env: &[(OsString, OsString)]) -> Option<OsString> {
    extra_env
        .iter()
        .rev()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| value.clone())
        .or_else(crate::env::inherited_path)
}

/// Ruta de búsqueda de `env` cuando no hay `PATH` (`_PATH_DEFPATH`).
const DEFAULT_PATH: &str = "/usr/bin:/bin";

/// `true` si `adapter` es un script `#!/usr/bin/env node` (también `env -S node …`) y no hay
/// un `node` ejecutable en ninguna carpeta absoluta de `path_var` (sin `PATH`, la ruta por
/// defecto de `env`, `/usr/bin:/bin`). Cualquier otra cosa (binario,
/// otro intérprete, archivo ilegible) → `false`: lo decide el lanzamiento, como antes. E/S
/// bloqueante y breve (lee ≤ 256 bytes). En Windows, siempre `false` (no hay `#!`).
pub fn missing_node(adapter: &Path, path_var: Option<&OsStr>) -> bool {
    if cfg!(windows) || !wants_node(adapter) {
        return false;
    }
    let path_var = path_var.unwrap_or(OsStr::new(DEFAULT_PATH));
    !std::env::split_paths(path_var)
        // `env` buscaría una entrada relativa (o vacía) desde el `cwd`, que es el worktree: no
        // se cuenta como `node` disponible.
        .filter(|dir| dir.is_absolute())
        .any(|dir| is_executable_file(&dir.join("node")))
}

fn wants_node(adapter: &Path) -> bool {
    use std::io::Read;
    // Solo un archivo normal: abrir un FIFO bloquearía.
    if !std::fs::metadata(adapter).is_ok_and(|metadata| metadata.is_file()) {
        return false;
    }
    let Ok(file) = std::fs::File::open(adapter) else {
        return false;
    };
    let mut head = Vec::with_capacity(SHEBANG_BYTES);
    if file
        .take(SHEBANG_BYTES as u64)
        .read_to_end(&mut head)
        .is_err()
    {
        return false;
    }
    env_interpreter(&head).is_some_and(|name| name == "node")
}

/// Programa que pide una primera línea `#!<…>/env [-S] [VAR=valor…] <programa> …`.
fn env_interpreter(head: &[u8]) -> Option<&str> {
    let rest = head.strip_prefix(b"#!")?;
    let line = rest.split(|&b| b == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?;
    let mut words = line.split_ascii_whitespace();
    let env = words.next()?;
    if env != "env" && !env.ends_with("/env") {
        return None;
    }
    words.find(|word| !word.starts_with('-') && !word.contains('='))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interpreter_is_read_from_an_env_shebang() {
        assert_eq!(
            env_interpreter(b"#!/usr/bin/env node\nimport x"),
            Some("node")
        );
        assert_eq!(env_interpreter(b"#! /usr/bin/env node"), Some("node"));
        assert_eq!(
            env_interpreter(b"#!/usr/bin/env -S node --no-warnings\n"),
            Some("node")
        );
        assert_eq!(env_interpreter(b"#!/usr/bin/env A=1 node\n"), Some("node"));
        assert_eq!(env_interpreter(b"#!/usr/bin/env bun\n"), Some("bun"));
        assert_eq!(env_interpreter(b"#!/bin/sh\nnode x"), None);
        assert_eq!(env_interpreter(b"#!/opt/node/bin/node\n"), None);
        assert_eq!(env_interpreter(b"\x7fELF"), None);
        assert_eq!(env_interpreter(b""), None);
    }

    #[cfg(unix)]
    #[test]
    fn node_is_looked_up_like_env_does() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("adaptador");
        std::fs::write(&script, "#!/usr/bin/env node\n").unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let empty = dir.path().join("vacía");
        std::fs::create_dir(&empty).unwrap();
        let path = |dirs: &[&Path]| std::env::join_paths(dirs).unwrap();

        assert!(missing_node(&script, Some(&path(&[&empty]))));
        // Un `node` sin permiso de ejecución no cuenta.
        std::fs::write(bin.join("node"), "").unwrap();
        assert!(missing_node(&script, Some(&path(&[&empty, &bin]))));
        std::fs::set_permissions(bin.join("node"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!missing_node(&script, Some(&path(&[&empty, &bin]))));
        // Las entradas relativas no cuentan (serían relativas al worktree).
        assert!(missing_node(&script, Some(OsStr::new("bin"))));
        // Un binario u otro intérprete: lo decide el lanzamiento.
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        assert!(!missing_node(&script, None));
        assert!(!missing_node(&dir.path().join("no existe"), None));
        // Ruta de node configurada que ya no sirve.
        assert!(!configured_node_unusable(&bin.join("node")));
        assert!(configured_node_unusable(&empty.join("node")));
        assert!(configured_node_unusable(Path::new("node")));
    }

    #[test]
    fn the_node_folder_goes_first_and_nothing_else_changes() {
        assert_eq!(node_path_env(None), None);
        assert_eq!(node_path_env(Some(Path::new("node"))), None);
        let node = if cfg!(windows) {
            Path::new(r"C:\nodejs\node.exe")
        } else {
            Path::new("/opt/node/bin/node")
        };
        let (name, value) = node_path_env(Some(node)).unwrap();
        assert_eq!(name, "PATH");
        let dirs: Vec<_> = std::env::split_paths(&value).collect();
        assert_eq!(dirs[0], node.parent().unwrap());
        let inherited: Vec<_> = crate::env::inherited_path()
            .iter()
            .flat_map(std::env::split_paths)
            .collect();
        assert!(dirs.iter().all(|dir| dir.is_absolute()));
        assert_eq!(dirs[1..], inherited[..]);
        assert_eq!(
            effective_path(&[(name, value.clone())]).as_deref(),
            Some(value.as_os_str())
        );
        assert_eq!(effective_path(&[]), crate::env::inherited_path());
    }
}
