//! Disco para la limpieza de ejecuciones (HU-025): medir carpetas, borrar la salida de un
//! intento y encontrar carpetas de worktrees. Nunca sigue enlaces simbólicos ni sale de las
//! carpetas de la app. E/S bloqueante: llámalo desde `spawn_blocking`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Límites de una medición: más archivos o más tiempo → resultado incompleto (un mínimo).
pub const MAX_MEASURED_FILES: u64 = 200_000;
pub const MAX_MEASURE_TIME: Duration = Duration::from_secs(3);
/// Como mucho tantas carpetas de worktrees revisadas por plan.
pub const MAX_WORKTREE_DIRS: usize = 1_000;

/// Lo que ocupa una carpeta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub bytes: u64,
    pub files: u64,
    pub complete: bool,
}

/// Mide `path` sin seguir enlaces (un enlace cuenta como un archivo pequeño). `None` si no
/// existe. Se detiene en [`MAX_MEASURED_FILES`] o [`MAX_MEASURE_TIME`] (`complete = false`).
pub fn measure(path: &Path) -> Option<Usage> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    let started = Instant::now();
    let mut usage = Usage {
        complete: true,
        ..Usage::default()
    };
    if !metadata.is_dir() {
        usage.bytes = metadata.len();
        usage.files = 1;
        return Some(usage);
    }
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            usage.complete = false;
            continue;
        };
        for entry in entries {
            if usage.files >= MAX_MEASURED_FILES || started.elapsed() >= MAX_MEASURE_TIME {
                usage.complete = false;
                return Some(usage);
            }
            let Ok(entry) = entry else {
                usage.complete = false;
                continue;
            };
            let Ok(metadata) = entry.metadata() else {
                usage.complete = false;
                continue;
            };
            // `DirEntry::metadata` no sigue enlaces.
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                usage.files += 1;
                usage.bytes = usage.bytes.saturating_add(metadata.len());
            }
        }
    }
    Some(usage)
}

/// Un segmento que genera el núcleo (UUID): `[A-Za-z0-9_-]`, ≤ 64.
fn simple_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 64
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Borra la salida de un intento: exactamente `<runs_root>/<run_id>`, una carpeta real (no un
/// enlace). `Ok(false)` si no existía. `remove_dir_all` no sigue los enlaces de dentro.
pub fn remove_run_output(runs_root: &Path, run_id: &str) -> std::io::Result<bool> {
    if !simple_segment(run_id) || !runs_root.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "identificador de intento o carpeta no válidos",
        ));
    }
    let dir = runs_root.join(run_id);
    match std::fs::symlink_metadata(&dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
        Ok(metadata) if !metadata.is_dir() => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "la salida del intento no es una carpeta",
        )),
        Ok(_) => std::fs::remove_dir_all(&dir).map(|()| true),
    }
}

/// Carpetas `<root>/<proyecto>/<intento>` (ambos segmentos simples, carpetas reales; los
/// enlaces y la carpeta de hooks se ignoran), como mucho [`MAX_WORKTREE_DIRS`]; `true` si había
/// más. Sin la raíz, ninguna.
pub fn worktree_dirs(root: &Path) -> (Vec<PathBuf>, bool) {
    let mut found = Vec::new();
    let real_dirs = |dir: &Path| -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter(|entry| entry.file_name().to_str().is_some_and(simple_segment))
            .map(|entry| entry.path())
            .collect();
        dirs.sort();
        dirs
    };
    for project in real_dirs(root) {
        for run in real_dirs(&project) {
            if found.len() >= MAX_WORKTREE_DIRS {
                return (found, true);
            }
            found.push(run);
        }
    }
    (found, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_without_following_links_and_removes_only_the_run_folder() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("runs");
        let run = runs.join("01-intento");
        std::fs::create_dir_all(run.join("sub")).unwrap();
        std::fs::write(run.join("output-1.jsonl"), [0u8; 1000]).unwrap();
        std::fs::write(run.join("sub").join("x"), [0u8; 24]).unwrap();
        let outside = dir.path().join("fuera");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("grande"), [0u8; 5000]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, run.join("enlace")).unwrap();

        let usage = measure(&run).unwrap();
        assert!(usage.complete);
        assert!(usage.bytes >= 1024 && usage.bytes < 2000, "{usage:?}");
        assert_eq!(measure(&dir.path().join("no existe")), None);

        assert!(remove_run_output(&runs, "../fuera").is_err());
        assert!(remove_run_output(Path::new("runs"), "01-intento").is_err());
        assert!(remove_run_output(&runs, "01-intento").unwrap());
        assert!(!run.exists());
        assert!(outside.join("grande").exists(), "no sigue el enlace");
        assert!(!remove_run_output(&runs, "01-intento").unwrap());
    }

    #[test]
    fn lists_only_project_and_run_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("worktrees");
        std::fs::create_dir_all(root.join("p1").join("i1")).unwrap();
        std::fs::create_dir_all(root.join("p1").join("i2")).unwrap();
        std::fs::create_dir_all(root.join(".no-hooks")).unwrap();
        std::fs::create_dir_all(root.join("p 2").join("i3")).unwrap();
        std::fs::write(root.join("p1").join("archivo"), "").unwrap();
        let (dirs, truncated) = worktree_dirs(&root);
        assert!(!truncated);
        assert_eq!(
            dirs,
            [root.join("p1").join("i1"), root.join("p1").join("i2")]
        );
        assert_eq!(worktree_dirs(&dir.path().join("nada")), (Vec::new(), false));
    }
}
