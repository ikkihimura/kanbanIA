//! Sugerencias de `node` para los adaptadores npm (HU-026). Una app abierta desde Finder o el
//! menú Inicio no tiene el `PATH` de la terminal (nvm, fnm, asdf…), y los adaptadores son
//! scripts `#!/usr/bin/env node`. Esto solo **propone** rutas: mira si existen archivos
//! ejecutables en las ubicaciones habituales, sin ejecutar nada (ni `node --version` ni una
//! shell de login) y sin guardar nada; la persona elige y guarda una con la validación normal
//! (`update_run_settings`, campo `nodePath`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use ts_rs::TS;

/// Como mucho tantas sugerencias.
pub const MAX_NODE_CANDIDATES: usize = 20;
/// Como mucho tantas entradas leídas de la carpeta de versiones de un gestor, y tantas
/// versiones propuestas (las más altas).
const MAX_ENTRIES_PER_MANAGER: usize = 500;
const MAX_VERSIONS_PER_MANAGER: usize = 50;
/// La búsqueda no bloquea la app si un disco no responde.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(3);

/// Una instalación de `node` encontrada. Ruta local: solo para la persona, nunca para agentes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NodeCandidate {
    /// Ruta absoluta del ejecutable tal como se encontró (sin resolver enlaces).
    pub path: String,
    /// De dónde sale: «nvm», «Homebrew», «PATH de la app»…
    pub source: String,
    /// Versión según el nombre de su carpeta (nvm, fnm, asdf, mise); no se comprueba
    /// ejecutándolo. `null` = desconocida.
    pub version: Option<String>,
}

/// Dónde buscar. [`NodeSearchRoots::from_env`] lo toma del proceso; las pruebas lo fijan.
#[derive(Debug, Clone, Default)]
pub struct NodeSearchRoots {
    pub home: Option<PathBuf>,
    /// `PATH` con el que se abrió la app.
    pub path_var: Option<OsString>,
    /// Carpetas fijas del sistema (Homebrew, `/usr/local`; en Windows, `%ProgramFiles%`).
    pub system: Vec<(PathBuf, &'static str)>,
    /// Windows: `%APPDATA%` (nvm-windows, fnm).
    pub app_data: Option<PathBuf>,
}

impl NodeSearchRoots {
    /// Ubicaciones habituales de este sistema y carpeta personal del proceso.
    pub fn from_env() -> Self {
        #[cfg(windows)]
        let (home, system, app_data) = (
            std::env::var_os("USERPROFILE").map(PathBuf::from),
            std::env::var_os("ProgramFiles")
                .map(|dir| vec![(PathBuf::from(dir).join("nodejs"), "Instalador de Node.js")])
                .unwrap_or_default(),
            std::env::var_os("APPDATA").map(PathBuf::from),
        );
        #[cfg(not(windows))]
        let (home, system, app_data): (Option<PathBuf>, _, Option<PathBuf>) = (
            std::env::var_os("HOME").map(PathBuf::from),
            vec![
                (PathBuf::from("/opt/homebrew/bin"), "Homebrew"),
                (PathBuf::from("/usr/local/bin"), "/usr/local"),
            ],
            None,
        );
        Self {
            home: home.filter(|home| home.is_absolute()),
            path_var: std::env::var_os("PATH"),
            system,
            app_data: app_data.filter(|dir: &PathBuf| dir.is_absolute()),
        }
    }
}

#[cfg(windows)]
const NODE_FILE: &str = "node.exe";
#[cfg(not(windows))]
const NODE_FILE: &str = "node";

/// Busca `node` en las ubicaciones habituales (fuera del runtime y con límite de tiempo) y
/// devuelve como mucho [`MAX_NODE_CANDIDATES`], sin repetir el mismo archivo. Primero lo que
/// ya está en el `PATH` de la app; dentro de cada gestor, de la versión más alta a la más baja.
/// Si el disco no responde a tiempo, la lista vacía. No ejecuta nada.
pub async fn find_node_candidates(roots: NodeSearchRoots) -> Vec<NodeCandidate> {
    let task = tokio::task::spawn_blocking(move || search(&roots));
    match tokio::time::timeout(SEARCH_TIMEOUT, task).await {
        Ok(Ok(found)) => found,
        _ => Vec::new(),
    }
}

fn search(roots: &NodeSearchRoots) -> Vec<NodeCandidate> {
    let mut found = Found::default();
    if let Some(path_var) = &roots.path_var {
        for dir in std::env::split_paths(path_var).filter(|dir| dir.is_absolute()) {
            found.push(dir.join(NODE_FILE), "PATH de la app", None);
        }
    }
    for (dir, source) in &roots.system {
        found.push(dir.join(NODE_FILE), source, None);
    }
    if let Some(home) = &roots.home {
        #[cfg(not(windows))]
        {
            versions(
                &mut found,
                &home.join(".nvm/versions/node"),
                "bin/node",
                "nvm",
            );
            found.push(home.join(".volta/bin/node"), "Volta", None);
            for fnm in [
                home.join(".local/share/fnm/node-versions"),
                home.join("Library/Application Support/fnm/node-versions"),
            ] {
                versions(&mut found, &fnm, "installation/bin/node", "fnm");
            }
            versions(
                &mut found,
                &home.join(".asdf/installs/nodejs"),
                "bin/node",
                "asdf",
            );
            versions(
                &mut found,
                &home.join(".local/share/mise/installs/node"),
                "bin/node",
                "mise",
            );
        }
        #[cfg(windows)]
        found.push(
            home.join(".volta").join("bin").join(NODE_FILE),
            "Volta",
            None,
        );
    }
    if let Some(app_data) = &roots.app_data {
        versions(&mut found, &app_data.join("nvm"), NODE_FILE, "nvm-windows");
        versions(
            &mut found,
            &app_data.join("fnm").join("node-versions"),
            "installation/node.exe",
            "fnm",
        );
    }
    found.items
}

#[derive(Default)]
struct Found {
    items: Vec<NodeCandidate>,
    /// Rutas resueltas ya añadidas (un enlace de Homebrew y su destino son el mismo archivo).
    seen: Vec<PathBuf>,
}

impl Found {
    fn push(&mut self, path: PathBuf, source: &str, version: Option<String>) {
        if self.items.len() >= MAX_NODE_CANDIDATES || !is_executable_file(&path) {
            return;
        }
        let (Some(text), Ok(resolved)) = (path.to_str(), std::fs::canonicalize(&path)) else {
            return;
        };
        // Lo que no se podría guardar (`..`, caracteres invisibles…) no se propone.
        if !super::settings::is_valid_node_path_text(text) {
            return;
        }
        if let Some(index) = self.seen.iter().position(|seen| *seen == resolved) {
            // El mismo archivo por otro camino (enlace de Homebrew a nvm): se conserva la
            // primera ruta, con la versión si esta la sabe.
            if let Some(item) = self.items.get_mut(index)
                && item.version.is_none()
            {
                item.version = version;
            }
            return;
        }
        self.seen.push(resolved);
        self.items.push(NodeCandidate {
            path: text.to_owned(),
            source: source.to_owned(),
            version,
        });
    }
}

/// `<root>/<versión>/<inside>` de un gestor de versiones, de la más alta a la más baja.
fn versions(found: &mut Found, root: &Path, inside: &str, source: &str) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut names: Vec<String> = entries
        .take(MAX_ENTRIES_PER_MANAGER)
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| safe_version(name))
        .collect();
    // Primero ordenar y después recortar: `read_dir` no garantiza ningún orden.
    names.sort_by_key(|name| std::cmp::Reverse(version_key(name)));
    names.truncate(MAX_VERSIONS_PER_MANAGER);
    for name in names {
        let path = inside
            .split('/')
            .fold(root.join(&name), |path, part| path.join(part));
        found.push(path, source, Some(name));
    }
}

/// Un nombre de carpeta que se puede mostrar como versión: corto y sin caracteres raros.
fn safe_version(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// `v22.16.0` → `[22, 16, 0]` para ordenar (lo que no es número cuenta como 0).
fn version_key(name: &str) -> Vec<u64> {
    name.trim_start_matches(['v', 'V'])
        .split(['.', '-', '_'])
        .map(|part| part.parse().unwrap_or(0))
        .collect()
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
    fn versions_are_sorted_numerically_and_names_filtered() {
        let mut names = ["v8.0.0", "v22.16.0", "v22.9.1", "v20.0.0"];
        names.sort_by_key(|name| std::cmp::Reverse(version_key(name)));
        assert_eq!(names, ["v22.16.0", "v22.9.1", "v20.0.0", "v8.0.0"]);
        assert!(safe_version("v22.16.0"));
        assert!(safe_version("lts-iron"));
        for bad in [
            "",
            ".hidden",
            "a/b",
            "v22 x",
            "\u{202e}v22",
            &"9".repeat(33),
        ] {
            assert!(!safe_version(bad), "{bad:?}");
        }
    }
}
