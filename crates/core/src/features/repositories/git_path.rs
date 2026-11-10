//! Política de rutas de repositorio (solo lectura del sistema de archivos):
//!
//! - La ruta debe ser absoluta y existir; se canonicaliza resolviendo enlaces simbólicos y se
//!   guarda la ruta real (`dunce` evita prefijos `\\?\` en Windows).
//! - Debe ser la raíz de un working tree Git: `.git/` con `HEAD`, o un archivo `.git` con
//!   `gitdir:` (worktrees y submódulos). Repositorios bare no se admiten.
//! - Nunca se escribe en el repositorio.
//! - Los duplicados se comparan por identidad de archivo (`same-file`), inmune a mayúsculas,
//!   normalización Unicode y enlaces simbólicos.

use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::CoreError;

const MAX_PATH_BYTES: usize = 4096;
const GITDIR_PREFIX: &[u8] = b"gitdir:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RepositoryStatus {
    Available,
    Missing,
    NotARepository,
    Inaccessible,
}

fn invalid(reason: &str) -> CoreError {
    CoreError::Validation {
        field: "path",
        reason: reason.to_owned(),
    }
}

/// Valida la entrada del usuario y devuelve la ruta canónica de la raíz del repositorio.
pub(super) fn validate_repository_path(input: &str) -> Result<String, CoreError> {
    if input.trim().is_empty() {
        return Err(invalid("Indica la carpeta del repositorio."));
    }
    if input.len() > MAX_PATH_BYTES || input.contains('\0') {
        return Err(invalid("La ruta no es válida."));
    }
    let raw = Path::new(input);
    if !raw.is_absolute() {
        return Err(invalid("La ruta debe ser absoluta."));
    }

    let canonical = dunce::canonicalize(raw).map_err(|error| match error.kind() {
        ErrorKind::NotFound => invalid("La carpeta no existe."),
        ErrorKind::PermissionDenied => invalid("No hay permiso para acceder a la carpeta."),
        _ => invalid("No se pudo leer la carpeta."),
    })?;

    match repository_status(&canonical) {
        RepositoryStatus::Available => {}
        RepositoryStatus::Missing => return Err(invalid("La carpeta no existe.")),
        RepositoryStatus::Inaccessible => {
            return Err(invalid("No hay permiso para acceder a la carpeta."));
        }
        RepositoryStatus::NotARepository => {
            return Err(invalid(
                "La carpeta no es la raíz de un repositorio Git (no contiene .git).",
            ));
        }
    }

    let canonical = canonical
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("La ruta contiene caracteres no representables."))?;
    // Al resolver enlaces la ruta puede crecer: respetar el límite de la base de datos.
    if canonical.len() > MAX_PATH_BYTES {
        return Err(invalid("La ruta real del repositorio es demasiado larga."));
    }
    Ok(canonical)
}

/// Estado actual de una ruta guardada. Solo lee metadatos y, como mucho, 64 bytes de `.git`.
pub fn repository_status(path: &Path) -> RepositoryStatus {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => return status_from_error(&error),
    };
    if !metadata.is_dir() {
        return RepositoryStatus::NotARepository;
    }
    match has_git_marker(path) {
        Ok(true) => RepositoryStatus::Available,
        Ok(false) => RepositoryStatus::NotARepository,
        Err(error) => status_from_error(&error),
    }
}

fn status_from_error(error: &std::io::Error) -> RepositoryStatus {
    match error.kind() {
        ErrorKind::NotFound | ErrorKind::NotADirectory => RepositoryStatus::Missing,
        _ => RepositoryStatus::Inaccessible,
    }
}

fn has_git_marker(root: &Path) -> std::io::Result<bool> {
    let git = root.join(".git");
    let metadata = match std::fs::metadata(&git) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if metadata.is_dir() {
        return Ok(git.join("HEAD").is_file());
    }
    if metadata.is_file() {
        let mut head = [0u8; 64];
        let read = std::fs::File::open(&git)?.read(&mut head)?;
        return Ok(head[..read].starts_with(GITDIR_PREFIX));
    }
    Ok(false)
}

/// `true` si ambas rutas apuntan al mismo directorio físico. Una ruta inaccesible no coincide.
pub(super) fn same_directory(a: &Path, b: &Path) -> bool {
    same_file::is_same_file(a, b).unwrap_or(false)
}

pub(super) fn display_name(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_repo(parent: &Path, name: &str) -> PathBuf {
        let root = parent.join(name);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        root
    }

    #[test]
    fn accepts_repository_root_with_spaces_and_unicode() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git_repo(dir.path(), "Mi repo ñandú 🚀");

        let path = validate_repository_path(repo.to_str().unwrap()).unwrap();

        assert!(path.ends_with("Mi repo ñandú 🚀"));
        assert_eq!(display_name(&path), "Mi repo ñandú 🚀");
    }

    #[test]
    fn accepts_worktree_style_git_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("worktree");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".git"), "gitdir: /elsewhere/.git/worktrees/x\n").unwrap();

        assert!(validate_repository_path(root.to_str().unwrap()).is_ok());
    }

    #[test]
    fn rejects_invalid_inputs_with_specific_reasons() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let file = dir.path().join("file.txt");
        std::fs::write(&file, "x").unwrap();
        let repo = git_repo(dir.path(), "repo");
        let subdir = repo.join("src");
        std::fs::create_dir_all(&subdir).unwrap();
        let bogus_git_file = dir.path().join("bogus");
        std::fs::create_dir_all(&bogus_git_file).unwrap();
        std::fs::write(bogus_git_file.join(".git"), "not a gitdir").unwrap();

        let missing = dir.path().join("nope");
        let cases = [
            ("", "Indica"),
            ("relative/path", "absoluta"),
            (missing.to_str().unwrap(), "no existe"),
            (plain.to_str().unwrap(), "no es la raíz"),
            (file.to_str().unwrap(), "no es la raíz"),
            (subdir.to_str().unwrap(), "no es la raíz"),
            (bogus_git_file.to_str().unwrap(), "no es la raíz"),
        ];
        for (input, expected) in cases {
            match validate_repository_path(input) {
                Err(CoreError::Validation { field, reason }) => {
                    assert_eq!(field, "path");
                    assert!(reason.contains(expected), "{input:?}: {reason}");
                }
                other => panic!("{input:?} debió fallar, obtuvo {other:?}"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_resolved_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git_repo(dir.path(), "real");
        let link = dir.path().join("enlace");
        std::os::unix::fs::symlink(&repo, &link).unwrap();

        let stored = validate_repository_path(link.to_str().unwrap()).unwrap();

        assert_eq!(Path::new(&stored), dunce::canonicalize(&repo).unwrap());
        assert!(same_directory(Path::new(&stored), &link));
    }

    #[test]
    fn status_reflects_current_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git_repo(dir.path(), "repo");
        assert_eq!(repository_status(&repo), RepositoryStatus::Available);

        std::fs::remove_dir_all(repo.join(".git")).unwrap();
        assert_eq!(repository_status(&repo), RepositoryStatus::NotARepository);

        std::fs::remove_dir_all(&repo).unwrap();
        assert_eq!(repository_status(&repo), RepositoryStatus::Missing);
    }
}
