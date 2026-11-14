//! Commit y rama actuales de un repositorio vinculado, leyendo archivos de `.git` (nunca se
//! ejecuta `git` ni se escribe nada):
//!
//! - `.git` carpeta, o archivo `gitdir: <ruta>` (worktrees y submódulos; con `commondir` para
//!   las referencias compartidas);
//! - `HEAD` → `ref: refs/heads/<rama>` resuelto como referencia suelta o en `packed-refs`, o un
//!   identificador de commit (HEAD separado, sin rama).
//!
//! Lo que no se puede leer o reconocer (repositorio sin commits, formato `reftable`, archivos
//! inaccesibles, demasiado grandes…) queda como `None`: «no disponible», nunca inventado.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::platform::text::has_invisible_formatting;

const MAX_SMALL_FILE: u64 = 4 * 1024;
const MAX_PACKED_REFS: u64 = 8 * 1024 * 1024;
const MAX_REF_LEN: usize = 255;
const MAX_BRANCH_CHARS: usize = 100;
/// Límite para todas las lecturas de un intento: un disco que no responde no bloquea la app.
const HEADS_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct GitHead {
    pub commit: Option<String>,
    pub branch: Option<String>,
}

/// Lee los HEAD de varios repositorios (id → ruta) fuera del runtime asíncrono y con límite de
/// tiempo. Si se agota, ningún repositorio tiene HEAD («no disponible»).
pub(super) async fn read_heads(repositories: Vec<(String, String)>) -> HashMap<String, GitHead> {
    if repositories.is_empty() {
        return HashMap::new();
    }
    let task = tokio::task::spawn_blocking(move || {
        repositories
            .into_iter()
            .map(|(id, path)| (id, read_head(Path::new(&path))))
            .collect::<HashMap<_, _>>()
    });
    match tokio::time::timeout(HEADS_TIMEOUT, task).await {
        Ok(Ok(heads)) => heads,
        _ => HashMap::new(),
    }
}

pub(super) fn read_head(root: &Path) -> GitHead {
    let Some(git_dir) = git_dir(root) else {
        return GitHead::default();
    };
    let Some(head) = read_limited(&git_dir.join("HEAD"), MAX_SMALL_FILE) else {
        return GitHead::default();
    };
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref:") {
        let reference = reference.trim();
        if !valid_ref(reference) {
            return GitHead::default();
        }
        GitHead {
            commit: common_dir(&git_dir).and_then(|common| resolve_ref(&common, reference)),
            branch: reference.strip_prefix("refs/heads/").map(shorten_branch),
        }
    } else if is_object_id(head) {
        GitHead {
            commit: Some(head.to_ascii_lowercase()),
            branch: None,
        }
    } else {
        GitHead::default()
    }
}

/// Carpeta de Git del working tree: `.git/` o el destino de un archivo `.git` con `gitdir:`.
fn git_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    let metadata = std::fs::metadata(&dot_git).ok()?;
    if metadata.is_dir() {
        return git_folder(&dot_git);
    }
    if !metadata.is_file() {
        return None;
    }
    let content = read_limited(&dot_git, MAX_SMALL_FILE)?;
    let target = content.lines().next()?.strip_prefix("gitdir:")?.trim();
    if target.is_empty() {
        return None;
    }
    git_folder(&root.join(target))
}

/// Carpeta con las referencias compartidas (la principal en un worktree). Si `commondir`
/// apunta a algo que no es una carpeta de Git, no hay referencias que leer.
fn common_dir(git_dir: &Path) -> Option<PathBuf> {
    match read_limited(&git_dir.join("commondir"), MAX_SMALL_FILE) {
        Some(content) if !content.trim().is_empty() => git_folder(&git_dir.join(content.trim())),
        _ => Some(git_dir.to_path_buf()),
    }
}

/// Ruta real (sin enlaces ni `..`) de una carpeta de Git: un directorio con un archivo `HEAD`.
/// `join` con una ruta absoluta la sustituye, así que vale para destinos relativos y absolutos.
fn git_folder(candidate: &Path) -> Option<PathBuf> {
    let real = dunce::canonicalize(candidate).ok()?;
    let is_git = std::fs::metadata(&real).ok()?.is_dir()
        && std::fs::metadata(real.join("HEAD")).ok()?.is_file();
    is_git.then_some(real)
}

fn resolve_ref(common: &Path, reference: &str) -> Option<String> {
    let loose = reference
        .split('/')
        .fold(common.to_path_buf(), |path, part| path.join(part));
    if let Some(content) = read_limited(&loose, MAX_SMALL_FILE) {
        let content = content.trim();
        // Una referencia simbólica anidada no se sigue: «no disponible».
        return is_object_id(content).then(|| content.to_ascii_lowercase());
    }
    let packed = read_limited(&common.join("packed-refs"), MAX_PACKED_REFS)?;
    packed
        .lines()
        .filter(|line| !line.starts_with('#') && !line.starts_with('^'))
        .filter_map(|line| line.split_once(' '))
        .find(|(id, name)| *name == reference && is_object_id(id))
        .map(|(id, _)| id.to_ascii_lowercase())
}

/// Referencias aceptadas: más estrictas que `git check-ref-format`. Bajo `refs/`, partes no
/// vacías que no empiezan por `.` (así nunca se sale de la carpeta de Git; `.invalid` de
/// reftable tampoco pasa), sin `..`, `@{` ni final `.lock`, y solo ASCII imprimible sin
/// espacios ni `` ` < > | # \ : ~ ^ ? * [ ``. Lo demás deja la rama y el commit en `None`.
fn valid_ref(reference: &str) -> bool {
    reference.len() <= MAX_REF_LEN
        && reference.starts_with("refs/")
        && !has_invisible_formatting(reference)
        && !reference.contains("..")
        && !reference.contains("@{")
        && reference
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"))
        && reference.bytes().all(|b| {
            b.is_ascii_graphic()
                && !matches!(
                    b,
                    b'`' | b'<'
                        | b'>'
                        | b'|'
                        | b'#'
                        | b'\\'
                        | b':'
                        | b'~'
                        | b'^'
                        | b'?'
                        | b'*'
                        | b'['
                )
        })
}

/// La rama se guarda como mucho con `MAX_BRANCH_CHARS` caracteres (ASCII por `valid_ref`);
/// si se recorta, termina en «…».
fn shorten_branch(branch: &str) -> String {
    if branch.len() <= MAX_BRANCH_CHARS {
        branch.to_owned()
    } else {
        format!("{}…", &branch[..MAX_BRANCH_CHARS])
    }
}

/// SHA-1 (40) o SHA-256 (64) en hexadecimal.
fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Lee un archivo normal de texto de como mucho `limit` bytes; si es mayor, no es un archivo
/// normal o no se puede leer, `None`.
///
/// Se comprueba con `metadata` *antes* de abrir: abrir un FIFO para leer bloquea hasta que
/// alguien escribe. Queda una carrera (sustituirlo entre la comprobación y la apertura) que
/// no se cierra sin `O_NONBLOCK`, y el núcleo no enlaza `libc`; la cubre el límite de tiempo
/// de `read_heads` (el hilo bloqueado se abandona, la app sigue).
fn read_limited(path: &Path, limit: u64) -> Option<String> {
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut content = String::new();
    file.take(limit + 1).read_to_string(&mut content).ok()?;
    (content.len() as u64 <= limit).then_some(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn rejects_refs_that_could_leave_the_git_folder() {
        for bad in [
            "refs/../../etc/passwd",
            "refs/heads/.invalid",
            "heads/main",
            "refs//main",
            "refs/heads/a\\b",
            "/refs/heads/main",
            "refs/heads/a b",
            "refs/heads/`rm`",
            "refs/heads/<b>",
            "refs/heads/a|b",
            "refs/heads/#1",
            "refs/heads/x\u{200B}y",
            "refs/heads/ñandú",
            "refs/heads/a..b",
            "refs/heads/a@{1}",
            "refs/heads/x.lock",
        ] {
            assert!(!valid_ref(bad), "{bad:?}");
        }
        assert!(valid_ref("refs/heads/feature/issue-12_v2.0"));
        let long = format!("refs/heads/{}", "b".repeat(150));
        assert!(valid_ref(&long));
        let branch = shorten_branch(long.strip_prefix("refs/heads/").unwrap());
        assert_eq!(branch.chars().count(), MAX_BRANCH_CHARS + 1);
        assert!(branch.ends_with('…'));
    }

    #[test]
    fn object_ids_are_sha1_or_sha256_hex() {
        assert!(is_object_id(SHA));
        assert!(is_object_id(&"a".repeat(64)));
        assert!(!is_object_id(&SHA[1..]));
        assert!(!is_object_id(&"g".repeat(40)));
    }

    #[test]
    fn nested_symbolic_and_oversized_refs_are_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        std::fs::create_dir_all(git.join("refs/heads")).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("refs/heads/main"), "ref: refs/heads/other\n").unwrap();
        assert_eq!(
            read_head(dir.path()),
            GitHead {
                commit: None,
                branch: Some("main".into())
            }
        );
        std::fs::write(git.join("HEAD"), "x".repeat(5000)).unwrap();
        assert_eq!(read_head(dir.path()), GitHead::default());
    }

    #[test]
    fn gitdir_must_point_to_a_real_git_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("wt");
        std::fs::create_dir_all(&root).unwrap();
        // Destino sin HEAD (o inexistente): no es una carpeta de Git.
        let empty = dir.path().join("vacia");
        std::fs::create_dir_all(&empty).unwrap();
        for target in [empty.to_str().unwrap(), "/no/existe", "../vacia"] {
            std::fs::write(root.join(".git"), format!("gitdir: {target}\n")).unwrap();
            assert_eq!(read_head(&root), GitHead::default(), "{target}");
        }
        // Destino relativo válido, resuelto a su ruta real.
        let real = dir.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("HEAD"), format!("{SHA}\n")).unwrap();
        std::fs::write(root.join(".git"), "gitdir: ../real\n").unwrap();
        assert_eq!(read_head(&root).commit.as_deref(), Some(SHA));
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_as_head_is_unavailable_without_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        std::fs::create_dir_all(&git).unwrap();
        let status = std::process::Command::new("mkfifo")
            .arg(git.join("HEAD"))
            .status()
            .unwrap();
        assert!(status.success());
        let root = dir.path().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(read_head(&root)).unwrap());
        let head = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("read_head se colgó");
        assert_eq!(head, GitHead::default());
        // Y como referencia suelta de la rama.
        std::fs::remove_file(git.join("HEAD")).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir_all(git.join("refs/heads")).unwrap();
        std::process::Command::new("mkfifo")
            .arg(git.join("refs/heads/main"))
            .status()
            .unwrap();
        let root = dir.path().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(read_head(&root)).unwrap());
        let head = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("read_head se colgó");
        assert_eq!(head.commit, None);
        assert_eq!(head.branch.as_deref(), Some("main"));
    }
}
