//! Worktrees de Git para los intentos, con la CLI de `git` (argumentos explícitos, sin shell).
//!
//! - Ruta: `<raíz>/<proyecto>/<intento>`; ambos segmentos `[A-Za-z0-9_-]`.
//! - Crear: `git worktree add --no-track -b <rama> -- <ruta> <commit>` desde un commit
//!   explícito; la rama no puede existir (nunca se reutiliza ni se sobrescribe).
//! - Estado: `git status --porcelain=v1 -z --untracked-files=all` en el worktree.
//! - Eliminar: solo una carpeta que sea exactamente un worktree registrado del repositorio bajo
//!   la raíz; sin forzar, uno con cambios se rechaza. La rama se conserva siempre.
//!
//! Nunca se ejecuta nada que toque el árbol de trabajo del repositorio original (`checkout`,
//! `reset`, `clean`, `stash`…). Todo `git` corre con entorno limpio, sin preguntas de
//! terminal, con hooks desactivados (`core.hooksPath` = carpeta vacía propia),
//! `core.fsmonitor=false`, límite de tiempo y salida acotada. La configuración del usuario y
//! del repositorio sí se respeta (p. ej. filtros como Git LFS).

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::time::timeout;

use crate::env::allowed_env;
use crate::process::CancelToken;
use crate::tree::ProcessTree;

/// Límite por defecto de cada orden de Git.
pub const DEFAULT_GIT_TIMEOUT: Duration = Duration::from_secs(120);
/// Salida estándar que se conserva de Git (estado y lista de worktrees).
const MAX_GIT_STDOUT: usize = 8 * 1024 * 1024;
/// Error estándar que se conserva (solo para el detalle del log).
const MAX_GIT_STDERR: usize = 8 * 1024;
const MAX_SEGMENT: usize = 64;
const MAX_BRANCH: usize = 100;

/// Cómo se invoca Git.
#[derive(Debug, Clone)]
pub struct GitConfig {
    /// Programa: `git` (se busca en el `PATH` permitido) o una ruta absoluta.
    pub program: PathBuf,
    /// Carpeta vacía que se pasa como `core.hooksPath` para que no se ejecute ningún hook.
    pub empty_hooks_dir: PathBuf,
    pub timeout: Duration,
}

impl GitConfig {
    /// `git` del `PATH`, con la carpeta de hooks indicada (que se crea vacía si falta).
    pub fn new(empty_hooks_dir: PathBuf) -> Self {
        Self {
            program: PathBuf::from("git"),
            empty_hooks_dir,
            timeout: DEFAULT_GIT_TIMEOUT,
        }
    }
}

/// Qué falló. Los mensajes (`Display`) son seguros: sin rutas ni salida de Git.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeErrorKind {
    /// Rama, commit, segmento o ruta con formato inválido.
    InvalidInput,
    /// La carpeta de hooks no está vacía o no se pudo crear.
    HooksDir,
    /// `git` no está disponible.
    GitUnavailable,
    /// La carpeta del repositorio no existe o no es un repositorio de Git.
    NotARepository,
    /// El commit de la instantánea no existe en el repositorio.
    CommitNotFound,
    BranchExists,
    PathExists,
    /// La ruta no es un worktree registrado del repositorio bajo la raíz de worktrees.
    NotRegistered,
    /// La carpeta del worktree ya no existe.
    Missing,
    /// Tiene cambios; eliminarlo exige forzar.
    HasChanges {
        changes: usize,
    },
    /// HEAD no está en la rama del intento o tiene commits que no están en ninguna rama:
    /// eliminarlo los perdería; exige forzar (como `HasChanges`).
    CommitsAtRisk,
    /// Tiene archivos ignorados por Git (`.env`, compilados…): Git los borraría sin forzar,
    /// pero eliminarlos exige forzar (decisión del propietario, como `HasChanges`).
    HasIgnored {
        ignored: usize,
    },
    Timeout,
    /// Se canceló mientras Git trabajaba (se mató su árbol de procesos).
    Cancelled,
    /// Git terminó con error.
    GitFailed,
    Io,
}

#[derive(Debug, Clone)]
pub struct WorktreeError {
    pub kind: WorktreeErrorKind,
    /// Detalle para el log (puede contener rutas y salida de Git); nunca para la UI.
    pub detail: String,
    /// `true` si el error ocurrió en `git worktree add` o después: puede haber quedado algo en
    /// disco. Si es `false`, no se llegó a crear nada (la rama y la carpeta no son del intento).
    pub add_attempted: bool,
}

impl WorktreeError {
    fn new(kind: WorktreeErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
            add_attempted: false,
        }
    }

    fn after_add(mut self) -> Self {
        self.add_attempted = true;
        self
    }

    /// Errores que la UI trata como «tiene cambios»: eliminar exige descartarlos.
    pub fn needs_force(&self) -> bool {
        matches!(
            self.kind,
            WorktreeErrorKind::HasChanges { .. }
                | WorktreeErrorKind::CommitsAtRisk
                | WorktreeErrorKind::HasIgnored { .. }
        )
    }
}

impl std::fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            WorktreeErrorKind::InvalidInput => {
                f.write_str("Los datos del worktree no son válidos.")
            }
            WorktreeErrorKind::HooksDir => {
                f.write_str("No se pudo preparar la carpeta interna para desactivar los hooks.")
            }
            WorktreeErrorKind::GitUnavailable => {
                f.write_str("No se encontró Git. Instálalo y comprueba que está en el PATH.")
            }
            WorktreeErrorKind::NotARepository => {
                f.write_str("La carpeta del repositorio no existe o no es un repositorio de Git.")
            }
            WorktreeErrorKind::CommitNotFound => {
                f.write_str("El commit de la instantánea no existe en el repositorio.")
            }
            WorktreeErrorKind::BranchExists => {
                f.write_str("Ya existe una rama con ese nombre: no se reutiliza.")
            }
            WorktreeErrorKind::PathExists => f.write_str("La carpeta del worktree ya existe."),
            WorktreeErrorKind::NotRegistered => {
                f.write_str("La carpeta no es un worktree de este intento: no se toca.")
            }
            WorktreeErrorKind::Missing => f.write_str("La carpeta del worktree ya no existe."),
            WorktreeErrorKind::HasChanges { changes } => write!(
                f,
                "El worktree tiene {changes} cambio(s) sin guardar en un commit: para eliminarlo hay que descartarlos explícitamente."
            ),
            WorktreeErrorKind::CommitsAtRisk => f.write_str(
                "El worktree no está en la rama del intento o tiene commits que no están en ninguna rama: eliminarlo los perdería. Para eliminarlo hay que descartarlos explícitamente.",
            ),
            WorktreeErrorKind::HasIgnored { ignored } => write!(
                f,
                "El worktree tiene {ignored} archivo(s) ignorado(s) por Git (p. ej. .env o compilados): para eliminarlo hay que descartarlos explícitamente."
            ),
            WorktreeErrorKind::Timeout => f.write_str("Git no respondió a tiempo."),
            WorktreeErrorKind::Cancelled => f.write_str("Se canceló mientras Git trabajaba."),
            WorktreeErrorKind::GitFailed => f.write_str("Git no pudo completar la operación."),
            WorktreeErrorKind::Io => f.write_str("No se pudo acceder a la carpeta de worktrees."),
        }
    }
}

impl std::error::Error for WorktreeError {}

/// Petición de creación.
#[derive(Debug, Clone)]
pub struct CreateWorktree {
    /// Raíz (working tree) del repositorio original, absoluta.
    pub repository: PathBuf,
    /// `<data_dir>/worktrees`, absoluta.
    pub root: PathBuf,
    /// Segmento del proyecto (`[A-Za-z0-9_-]`, ≤ 64).
    pub project_segment: String,
    /// Segmento del intento (`[A-Za-z0-9_-]`, ≤ 64).
    pub run_segment: String,
    /// Rama nueva (ver [`validate_branch`]).
    pub branch: String,
    /// Commit completo (40 o 64 hexadecimales).
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedWorktree {
    /// Ruta canónica del worktree.
    pub path: PathBuf,
    pub branch: String,
    pub base_commit: String,
}

/// Estado del worktree según Git.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorktreeStatus {
    /// Entradas con cambios (modificadas, borradas, renombradas, sin seguimiento…; cada una
    /// cuenta una vez).
    pub changes: usize,
    /// Archivos o carpetas ignorados por `.gitignore`: `git worktree remove` los borraría
    /// incluso sin forzar, así que [`remove_worktree`] exige forzar si hay alguno.
    pub ignored: usize,
    /// `false` si la salida de Git superó el límite: los recuentos son entonces un mínimo.
    pub complete: bool,
    /// HEAD no está en la rama del intento o hay commits que no están en ninguna rama.
    pub commits_at_risk: bool,
}

impl WorktreeStatus {
    /// Se puede eliminar sin perder nada: sin cambios, sin ignorados y sin commits en riesgo.
    pub fn is_clean(&self) -> bool {
        self.changes == 0 && self.ignored == 0 && self.complete && !self.commits_at_risk
    }
}

/// Resultado de eliminar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveOutcome {
    /// Estaba limpio y se eliminó.
    Removed,
    /// Se eliminó forzando (descartando cambios, si los había).
    Forced,
    /// La carpeta ya no existía: no se tocó nada (Git lo verá como «prunable»).
    Missing,
}

/// Crea el worktree y su rama desde `commit`. No modifica el árbol de trabajo original.
pub async fn create_worktree(
    git: &GitConfig,
    request: &CreateWorktree,
) -> Result<CreatedWorktree, WorktreeError> {
    create_worktree_cancellable(git, request, None).await
}

/// Como [`create_worktree`], pero si `cancel` se activa mata el árbol de procesos de la orden de
/// Git en curso y devuelve [`WorktreeErrorKind::Cancelled`] (con `add_attempted` si ya estaba en
/// `git worktree add`: puede haber quedado una carpeta a medias).
pub async fn create_worktree_cancellable(
    git: &GitConfig,
    request: &CreateWorktree,
    cancel: Option<&CancelToken>,
) -> Result<CreatedWorktree, WorktreeError> {
    validate_segment(&request.project_segment)?;
    validate_segment(&request.run_segment)?;
    validate_branch(&request.branch)?;
    validate_commit(&request.commit)?;
    let repository = existing_dir(&request.repository)?;
    prepare_hooks_dir(&git.empty_hooks_dir)?;
    require_absolute(&request.root)?;

    let project_dir = request.root.join(&request.project_segment);
    std::fs::create_dir_all(&project_dir)
        .map_err(|e| WorktreeError::new(WorktreeErrorKind::Io, e.to_string()))?;
    // La carpeta del proyecto debe ser una carpeta real directamente bajo la raíz (no un
    // enlace que lleve el worktree a otro sitio).
    let metadata = std::fs::symlink_metadata(&project_dir)
        .map_err(|e| WorktreeError::new(WorktreeErrorKind::Io, e.to_string()))?;
    if !metadata.is_dir() {
        return Err(WorktreeError::new(
            WorktreeErrorKind::InvalidInput,
            "la carpeta del proyecto es un enlace o no es una carpeta",
        ));
    }
    let project_dir = canonical(&project_dir)?;
    if project_dir.parent() != Some(canonical(&request.root)?.as_path()) {
        return Err(WorktreeError::new(
            WorktreeErrorKind::InvalidInput,
            "la carpeta del proyecto no está directamente bajo la raíz",
        ));
    }
    let path = project_dir.join(&request.run_segment);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => {
            return Err(WorktreeError::new(
                WorktreeErrorKind::PathExists,
                path.display().to_string(),
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(WorktreeError::new(WorktreeErrorKind::Io, e.to_string())),
    }

    let in_repo = |args: Vec<OsString>| {
        let mut full: Vec<OsString> = vec!["-C".into(), repository.clone().into_os_string()];
        full.extend(args);
        full
    };

    require_toplevel(git, &repository, cancel).await?;
    // ¿Existe el commit?
    let object = format!("{}^{{commit}}", request.commit);
    let output = run_git_cancellable(
        git,
        in_repo(vec!["cat-file".into(), "-e".into(), object.into()]),
        cancel,
    )
    .await?;
    if !output.success {
        return Err(WorktreeError::new(
            WorktreeErrorKind::CommitNotFound,
            output.stderr_text(),
        ));
    }
    // ¿Existe la rama? (`show-ref --verify --quiet`: 0 = existe, 1 = no.)
    let reference = format!("refs/heads/{}", request.branch);
    let output = run_git_cancellable(
        git,
        in_repo(vec![
            "show-ref".into(),
            "--verify".into(),
            "--quiet".into(),
            reference.into(),
        ]),
        cancel,
    )
    .await?;
    if output.success {
        return Err(WorktreeError::new(
            WorktreeErrorKind::BranchExists,
            request.branch.clone(),
        ));
    }

    let output = run_git_cancellable(
        git,
        in_repo(vec![
            "worktree".into(),
            "add".into(),
            "--no-track".into(),
            "-b".into(),
            request.branch.clone().into(),
            "--".into(),
            path.clone().into_os_string(),
            request.commit.clone().into(),
        ]),
        cancel,
    )
    .await
    .map_err(WorktreeError::after_add)?;
    if !output.success {
        return Err(
            WorktreeError::new(WorktreeErrorKind::GitFailed, output.stderr_text()).after_add(),
        );
    }
    Ok(CreatedWorktree {
        path: canonical(&path).map_err(WorktreeError::after_add)?,
        branch: request.branch.clone(),
        base_commit: request.commit.clone(),
    })
}

/// Cambios del worktree. Antes comprueba que es un worktree registrado bajo la raíz: así nunca
/// se ejecuta Git (ni la configuración de otro repositorio) en una carpeta arbitraria.
/// `branch` es la rama registrada del intento: si HEAD no está en ella o hay commits fuera de
/// toda rama, `commits_at_risk`.
pub async fn worktree_status(
    git: &GitConfig,
    repository: &Path,
    root: &Path,
    path: &Path,
    branch: &str,
) -> Result<WorktreeStatus, WorktreeError> {
    validate_branch(branch)?;
    if !exists(path)? {
        return Err(WorktreeError::new(WorktreeErrorKind::Missing, ""));
    }
    let path = validate_registered(git, repository, root, path).await?;
    status_of(git, &path, branch).await
}

/// Elimina el worktree. Sin `force`, uno con cambios → [`WorktreeErrorKind::HasChanges`] (y Git
/// también se negaría), y uno cuyo HEAD no está en `branch` (la rama registrada del intento) o
/// con commits fuera de toda rama → [`WorktreeErrorKind::CommitsAtRisk`], y uno con archivos
/// ignorados → [`WorktreeErrorKind::HasIgnored`] (Git los borraría sin preguntar; decisión del
/// propietario: también exigen descartarlos explícitamente). La rama no se borra nunca. Si la carpeta ya no existe,
/// no toca nada.
pub async fn remove_worktree(
    git: &GitConfig,
    repository: &Path,
    root: &Path,
    path: &Path,
    branch: &str,
    force: bool,
) -> Result<RemoveOutcome, WorktreeError> {
    validate_branch(branch)?;
    if !exists(path)? {
        return Ok(RemoveOutcome::Missing);
    }
    let path = validate_registered(git, repository, root, path).await?;
    if !force {
        let status = status_of(git, &path, branch).await?;
        if status.changes > 0 || !status.complete {
            return Err(WorktreeError::new(
                WorktreeErrorKind::HasChanges {
                    changes: status.changes,
                },
                "",
            ));
        }
        if status.commits_at_risk {
            return Err(WorktreeError::new(WorktreeErrorKind::CommitsAtRisk, ""));
        }
        if status.ignored > 0 {
            return Err(WorktreeError::new(
                WorktreeErrorKind::HasIgnored {
                    ignored: status.ignored,
                },
                "",
            ));
        }
    }
    let repository = existing_dir(repository)?;
    let mut args: Vec<OsString> = vec![
        "-C".into(),
        repository.into_os_string(),
        "worktree".into(),
        "remove".into(),
    ];
    if force {
        // Una sola vez: un worktree bloqueado (`lock`) sigue sin eliminarse.
        args.push("--force".into());
    }
    args.push("--".into());
    args.push(path.into_os_string());
    let output = run_git(git, args).await?;
    if !output.success {
        return Err(WorktreeError::new(
            WorktreeErrorKind::GitFailed,
            output.stderr_text(),
        ));
    }
    Ok(if force {
        RemoveOutcome::Forced
    } else {
        RemoveOutcome::Removed
    })
}

/// Comprueba que `path` es exactamente `<raíz>/<proyecto>/<intento>` (tras resolver enlaces),
/// que la carpeta no es un enlace y que el repositorio la tiene registrada como worktree
/// enlazado (nunca el principal). Devuelve la ruta canónica.
pub async fn validate_registered(
    git: &GitConfig,
    repository: &Path,
    root: &Path,
    path: &Path,
) -> Result<PathBuf, WorktreeError> {
    require_absolute(path)?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| WorktreeError::new(WorktreeErrorKind::NotRegistered, e.to_string()))?;
    if !metadata.is_dir() {
        return Err(WorktreeError::new(
            WorktreeErrorKind::NotRegistered,
            "no es una carpeta (o es un enlace)",
        ));
    }
    let canonical_root = canonical(root)?;
    let canonical_path = canonical(path)?;
    let relative = canonical_path
        .strip_prefix(&canonical_root)
        .map_err(|_| WorktreeError::new(WorktreeErrorKind::NotRegistered, "fuera de la raíz"))?;
    let segments: Vec<_> = relative.components().collect();
    let layout_ok = segments.len() == 2
        && segments.iter().all(|component| match component {
            Component::Normal(name) => name.to_str().is_some_and(|s| validate_segment(s).is_ok()),
            _ => false,
        });
    if !layout_ok {
        return Err(WorktreeError::new(
            WorktreeErrorKind::NotRegistered,
            "no es <raíz>/<proyecto>/<intento>",
        ));
    }

    prepare_hooks_dir(&git.empty_hooks_dir)?;
    let repository = existing_dir(repository)?;
    // Antes de ejecutar Git en el worktree: su `.git` lo escribe el agente y podría señalar a
    // un repositorio propio (con filtros que el Git de la app ejecutaría sin sandbox).
    check_admin_link(&repository, &canonical_path)?;
    require_toplevel(git, &repository, None).await?;
    let output = run_git(
        git,
        vec![
            "-C".into(),
            repository.into_os_string(),
            "worktree".into(),
            "list".into(),
            "--porcelain".into(),
            "-z".into(),
        ],
    )
    .await?;
    if !output.success || output.stdout_truncated {
        return Err(WorktreeError::new(
            WorktreeErrorKind::GitFailed,
            output.stderr_text(),
        ));
    }
    let registered = linked_worktrees(&output.stdout)
        .into_iter()
        .filter_map(|candidate| dunce::canonicalize(candidate).ok())
        .any(|candidate| candidate == canonical_path);
    if !registered {
        return Err(WorktreeError::new(
            WorktreeErrorKind::NotRegistered,
            "no figura en `git worktree list`",
        ));
    }
    Ok(canonical_path)
}

/// Prefijo de las ramas de los intentos: la limpieza (HU-025) nunca toca otra rama.
pub const RUN_BRANCH_PREFIX: &str = "kanbania/";

/// Rama de un intento en el repositorio: `None` si ya no existe; si existe, cuántos commits
/// suyos no están en ninguna otra rama (`None` dentro si no se pudo saber). Solo lectura.
pub async fn branch_status(
    git: &GitConfig,
    repository: &Path,
    branch: &str,
) -> Result<Option<Option<u64>>, WorktreeError> {
    validate_run_branch(branch)?;
    prepare_hooks_dir(&git.empty_hooks_dir)?;
    let repository = existing_dir(repository)?;
    require_toplevel(git, &repository, None).await?;
    let in_repo = |args: Vec<OsString>| {
        let mut full: Vec<OsString> = vec!["-C".into(), repository.clone().into_os_string()];
        full.extend(args);
        full
    };
    let reference = format!("refs/heads/{branch}");
    let exists = run_git(
        git,
        in_repo(vec![
            "show-ref".into(),
            "--verify".into(),
            "--quiet".into(),
            reference.clone().into(),
        ]),
    )
    .await?;
    if !exists.success {
        return Ok(None);
    }
    // Commits de la rama que no están en ninguna otra (`--exclude` usa el nombre sin
    // `refs/heads/` cuando acompaña a `--branches`).
    let output = run_git(
        git,
        in_repo(vec![
            "rev-list".into(),
            "--count".into(),
            reference.into(),
            "--not".into(),
            format!("--exclude={branch}").into(),
            "--branches".into(),
        ]),
    )
    .await?;
    let count = output
        .success
        .then(|| {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<u64>()
                .ok()
        })
        .flatten();
    Ok(Some(count))
}

/// Borra la rama de un intento (`git branch -D`: también con trabajo sin integrar; la persona
/// lo confirmó una por una). `false` si ya no existía. Git se niega si la rama está en uso en
/// algún worktree → [`WorktreeErrorKind::GitFailed`].
pub async fn delete_branch(
    git: &GitConfig,
    repository: &Path,
    branch: &str,
) -> Result<bool, WorktreeError> {
    if branch_status(git, repository, branch).await?.is_none() {
        return Ok(false);
    }
    let repository = existing_dir(repository)?;
    let output = run_git(
        git,
        vec![
            "-C".into(),
            repository.into_os_string(),
            "branch".into(),
            "-D".into(),
            "--".into(),
            branch.into(),
        ],
    )
    .await?;
    if !output.success {
        return Err(WorktreeError::new(
            WorktreeErrorKind::GitFailed,
            output.stderr_text(),
        ));
    }
    Ok(true)
}

fn validate_run_branch(branch: &str) -> Result<(), WorktreeError> {
    validate_branch(branch)?;
    if branch.starts_with(RUN_BRANCH_PREFIX) {
        Ok(())
    } else {
        Err(WorktreeError::new(
            WorktreeErrorKind::InvalidInput,
            "no es una rama de intento",
        ))
    }
}

/// De dónde sale una carpeta de worktree sin fila (huérfana, HU-025): el repositorio cuyo
/// `.git/worktrees/<nombre>` señala su archivo `.git` y la rama de su HEAD (si es una rama de
/// intento válida). Solo lee dos archivos pequeños; `None` si no tiene esa forma (entonces no
/// se puede comprobar ni eliminar con Git). Quien llama debe pasar después por
/// [`validate_registered`] (lo hacen [`worktree_status`] y [`remove_worktree`]).
pub fn orphan_origin(path: &Path) -> Option<(PathBuf, Option<String>)> {
    let gitdir = gitdir_pointer(path)?;
    let common = gitdir.parent()?;
    if common.file_name()? != "worktrees" {
        return None;
    }
    let dot_git = common.parent()?;
    if dot_git.file_name()? != ".git" {
        return None;
    }
    let repository = dot_git.parent()?.to_path_buf();
    let branch = read_small(&gitdir.join("HEAD"))
        .and_then(|head| {
            head.trim()
                .strip_prefix("ref: refs/heads/")
                .map(str::to_owned)
        })
        .filter(|branch| validate_run_branch(branch).is_ok());
    Some((repository, branch))
}

/// Carpeta de administración a la que señala el archivo `<worktree>/.git` (`gitdir: …`;
/// relativa al worktree si Git usa rutas relativas), canónica. `None` si no tiene esa forma.
fn gitdir_pointer(worktree: &Path) -> Option<PathBuf> {
    let pointer = read_small(&worktree.join(".git"))?;
    let target = Path::new(pointer.strip_prefix("gitdir:")?.trim());
    dunce::canonicalize(worktree.join(target)).ok()
}

/// El worktree y el repositorio se señalan mutuamente, comprobado sin Git: `<worktree>/.git`
/// apunta a `<repo>/.git/worktrees/<n>`, que guarda en `gitdir` la ruta de ese `.git` y cuyo
/// `commondir` es `<repo>/.git`. Si no (un `.git` reescrito, un repositorio cuyo `.git` no es
/// una carpeta…), [`WorktreeErrorKind::NotRegistered`]: no se ejecuta Git ahí.
fn check_admin_link(repository: &Path, worktree: &Path) -> Result<(), WorktreeError> {
    let fail = |detail: &str| WorktreeError::new(WorktreeErrorKind::NotRegistered, detail);
    let common = repository.join(".git");
    let is_dir = std::fs::symlink_metadata(&common).is_ok_and(|m| m.is_dir());
    let common = dunce::canonicalize(&common)
        .ok()
        .filter(|_| is_dir)
        .ok_or_else(|| fail("el repositorio no tiene una carpeta .git"))?;
    let admin =
        gitdir_pointer(worktree).ok_or_else(|| fail("el .git del worktree no es válido"))?;
    if admin.parent() != Some(common.join("worktrees").as_path()) {
        return Err(fail("el .git del worktree señala a otro repositorio"));
    }
    let back = read_small(&admin.join("gitdir"))
        .and_then(|text| dunce::canonicalize(admin.join(text.trim())).ok());
    if back.as_deref() != Some(worktree.join(".git").as_path()) {
        return Err(fail("el repositorio no señala a este worktree"));
    }
    let commondir = read_small(&admin.join("commondir"))
        .and_then(|text| dunce::canonicalize(admin.join(text.trim())).ok());
    if commondir.as_deref() != Some(common.as_path()) {
        return Err(fail("commondir no es el del repositorio"));
    }
    Ok(())
}

/// Archivo normal (nunca un FIFO ni un enlace) de como mucho 4 KiB, como texto.
fn read_small(path: &Path) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Comprueba el nombre de rama: ASCII `[A-Za-z0-9._/-]`, ≤ 100, sin `..`, `//`, `@{`, sin
/// empezar por `-`, `/` o `.`, sin acabar en `/`, `.` ni `.lock`, y ningún componente que
/// empiece por `.`. Más estricto que `git check-ref-format --branch`.
pub fn validate_branch(branch: &str) -> Result<(), WorktreeError> {
    let invalid = || WorktreeError::new(WorktreeErrorKind::InvalidInput, "rama inválida");
    let allowed = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-');
    if branch.is_empty()
        || branch.len() > MAX_BRANCH
        || !branch.bytes().all(allowed)
        || branch.starts_with(['-', '/', '.'])
        || branch.ends_with(['/', '.'])
        || branch.ends_with(".lock")
        || branch.contains("..")
        || branch.contains("//")
        || branch
            .split('/')
            .any(|part| part.starts_with('.') || part.ends_with(".lock"))
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_segment(segment: &str) -> Result<(), WorktreeError> {
    let ok = !segment.is_empty()
        && segment.len() <= MAX_SEGMENT
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(WorktreeError::new(
            WorktreeErrorKind::InvalidInput,
            "segmento inválido",
        ))
    }
}

fn validate_commit(commit: &str) -> Result<(), WorktreeError> {
    let ok = matches!(commit.len(), 40 | 64)
        && commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if ok {
        Ok(())
    } else {
        Err(WorktreeError::new(
            WorktreeErrorKind::InvalidInput,
            "commit inválido",
        ))
    }
}

fn require_absolute(path: &Path) -> Result<(), WorktreeError> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(WorktreeError::new(
            WorktreeErrorKind::InvalidInput,
            "ruta no absoluta",
        ))
    }
}

fn existing_dir(path: &Path) -> Result<PathBuf, WorktreeError> {
    require_absolute(path)?;
    // Movido o borrado: para quien llama es «no disponible», no un error de E/S propio.
    let canonical = dunce::canonicalize(path)
        .map_err(|e| WorktreeError::new(WorktreeErrorKind::NotARepository, e.to_string()))?;
    if canonical.is_dir() {
        Ok(canonical)
    } else {
        Err(WorktreeError::new(
            WorktreeErrorKind::NotARepository,
            "no es una carpeta",
        ))
    }
}

fn canonical(path: &Path) -> Result<PathBuf, WorktreeError> {
    dunce::canonicalize(path).map_err(|e| WorktreeError::new(WorktreeErrorKind::Io, e.to_string()))
}

fn exists(path: &Path) -> Result<bool, WorktreeError> {
    require_absolute(path)?;
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(WorktreeError::new(WorktreeErrorKind::Io, e.to_string())),
    }
}

/// Crea la carpeta de hooks si falta y exige que esté vacía (si alguien pusiera ahí un hook,
/// se ejecutaría).
fn prepare_hooks_dir(dir: &Path) -> Result<(), WorktreeError> {
    let error = |detail: String| WorktreeError::new(WorktreeErrorKind::HooksDir, detail);
    require_absolute(dir)?;
    std::fs::create_dir_all(dir).map_err(|e| error(e.to_string()))?;
    let metadata = std::fs::symlink_metadata(dir).map_err(|e| error(e.to_string()))?;
    if !metadata.is_dir() {
        return Err(error("no es una carpeta".into()));
    }
    let mut entries = std::fs::read_dir(dir).map_err(|e| error(e.to_string()))?;
    if entries.next().is_some() {
        return Err(error("no está vacía".into()));
    }
    Ok(())
}

/// `true` si la carpeta es exactamente la raíz de un repositorio (`rev-parse --show-toplevel`):
/// una subcarpeta de otro repositorio no cuenta (Git subiría al padre).
async fn require_toplevel(
    git: &GitConfig,
    repository: &Path,
    cancel: Option<&CancelToken>,
) -> Result<(), WorktreeError> {
    let output = run_git_cancellable(
        git,
        vec![
            "-C".into(),
            repository.as_os_str().to_owned(),
            "rev-parse".into(),
            "--show-toplevel".into(),
        ],
        cancel,
    )
    .await?;
    let not_repo = |detail: String| WorktreeError::new(WorktreeErrorKind::NotARepository, detail);
    if !output.success {
        return Err(not_repo(output.stderr_text()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let toplevel = dunce::canonicalize(text.trim_end_matches(['\n', '\r']))
        .map_err(|e| not_repo(e.to_string()))?;
    if toplevel != repository {
        return Err(not_repo(
            "la carpeta está dentro de otro repositorio".into(),
        ));
    }
    Ok(())
}

async fn status_of(
    git: &GitConfig,
    path: &Path,
    branch: &str,
) -> Result<WorktreeStatus, WorktreeError> {
    let in_worktree = |args: &[&str]| {
        let mut full: Vec<OsString> = vec!["-C".into(), path.as_os_str().to_owned()];
        full.extend(args.iter().map(OsString::from));
        full
    };
    let output = run_git(
        git,
        in_worktree(&[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=traditional",
            "--ignore-submodules=none",
        ]),
    )
    .await?;
    if !output.success {
        return Err(WorktreeError::new(
            WorktreeErrorKind::GitFailed,
            output.stderr_text(),
        ));
    }
    let (changes, ignored) = count_status_entries(&output.stdout);

    // ¿HEAD en la rama del intento? (`symbolic-ref -q`: 0 = rama, 1 = separado.)
    let head = run_git(git, in_worktree(&["symbolic-ref", "-q", "HEAD"])).await?;
    let expected = format!("refs/heads/{branch}");
    let on_branch = head.success && String::from_utf8_lossy(&head.stdout).trim_end() == expected;
    // ¿Commits alcanzables desde HEAD que no están en ninguna rama?
    let unreachable = run_git(
        git,
        in_worktree(&["rev-list", "--count", "HEAD", "--not", "--branches"]),
    )
    .await?;
    let only_in_head = if unreachable.success {
        String::from_utf8_lossy(&unreachable.stdout)
            .trim()
            .parse::<u64>()
            .unwrap_or(u64::MAX)
    } else {
        u64::MAX
    };
    Ok(WorktreeStatus {
        changes,
        ignored,
        complete: !output.stdout_truncated,
        commits_at_risk: !on_branch || only_in_head > 0,
    })
}

/// Cuenta las entradas de `status --porcelain=v1 -z`: `XY ruta\0`, y en renombrados o
/// copiados (`R`/`C` en X o Y) una ruta de origen más `\0`. Devuelve (cambios, ignorados `!!`).
fn count_status_entries(stdout: &[u8]) -> (usize, usize) {
    let mut fields = stdout.split(|b| *b == 0).filter(|f| !f.is_empty());
    let (mut changes, mut ignored) = (0, 0);
    while let Some(entry) = fields.next() {
        let x = entry.first().copied().unwrap_or(b' ');
        let y = entry.get(1).copied().unwrap_or(b' ');
        if x == b'!' && y == b'!' {
            ignored += 1;
            continue;
        }
        changes += 1;
        if matches!(x, b'R' | b'C') || matches!(y, b'R' | b'C') {
            fields.next();
        }
    }
    (changes, ignored)
}

/// Rutas de los worktrees enlazados en `worktree list --porcelain -z` (sin el principal, que
/// es siempre el primero).
fn linked_worktrees(stdout: &[u8]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut first = true;
    for field in stdout.split(|b| *b == 0) {
        let Some(raw) = field.strip_prefix(b"worktree ") else {
            continue;
        };
        if first {
            first = false;
            continue;
        }
        if let Some(path) = path_from_bytes(raw) {
            paths.push(path);
        }
    }
    paths
}

#[cfg(unix)]
fn path_from_bytes(raw: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(raw)))
}

#[cfg(not(unix))]
fn path_from_bytes(raw: &[u8]) -> Option<PathBuf> {
    std::str::from_utf8(raw).ok().map(PathBuf::from)
}

struct GitOutput {
    success: bool,
    stdout: Vec<u8>,
    stdout_truncated: bool,
    stderr: Vec<u8>,
}

impl GitOutput {
    fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_owned()
    }
}

/// Ejecuta Git con hooks y fsmonitor desactivados, entorno limpio, sin terminal y con límite de
/// tiempo (al agotarse, se mata su árbol de procesos).
async fn run_git(git: &GitConfig, args: Vec<OsString>) -> Result<GitOutput, WorktreeError> {
    run_git_cancellable(git, args, None).await
}

/// Como `run_git`; si `cancel` se activa, mata el árbol de procesos de Git en seguida.
async fn run_git_cancellable(
    git: &GitConfig,
    args: Vec<OsString>,
    cancel: Option<&CancelToken>,
) -> Result<GitOutput, WorktreeError> {
    let mut hooks = OsString::from("core.hooksPath=");
    hooks.push(git.empty_hooks_dir.as_os_str());
    let mut command = Command::new(&git.program);
    command
        .arg("-c")
        .arg(hooks)
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .env_clear()
        .envs(allowed_env())
        .env("GIT_TERMINAL_PROMPT", "0")
        // Que `status` no reescriba el índice para refrescarlo.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut tree = ProcessTree::spawn(command).map_err(|e| {
        let kind = if e.kind() == std::io::ErrorKind::NotFound {
            WorktreeErrorKind::GitUnavailable
        } else {
            WorktreeErrorKind::Io
        };
        WorktreeError::new(kind, e.to_string())
    })?;
    let stdout = tree.take_stdout();
    let stderr = tree.take_stderr();
    let mut cancel = cancel.cloned();
    let cancelled = async {
        match cancel.as_mut() {
            Some(token) => token.cancelled().await,
            None => std::future::pending().await,
        }
    };
    let outcome = {
        let work = async {
            let (out, err) = tokio::join!(
                read_capped(stdout, MAX_GIT_STDOUT),
                read_capped(stderr, MAX_GIT_STDERR)
            );
            let status = tree.wait_leader().await;
            (out, err, status)
        };
        tokio::select! {
            result = timeout(git.timeout, work) => Some(result),
            () = cancelled => None,
        }
    };
    // Siempre: tras salir la raíz no quedan descendientes; al agotarse el tiempo o cancelar,
    // se mata el árbol.
    tree.kill().await;
    match outcome {
        Some(Ok(((stdout, stdout_truncated), (stderr, _), Ok(status)))) => Ok(GitOutput {
            success: status.success(),
            stdout,
            stdout_truncated,
            stderr,
        }),
        Some(Ok((_, _, Err(e)))) => Err(WorktreeError::new(WorktreeErrorKind::Io, e.to_string())),
        Some(Err(_)) => Err(WorktreeError::new(WorktreeErrorKind::Timeout, "")),
        None => Err(WorktreeError::new(WorktreeErrorKind::Cancelled, "")),
    }
}

/// Lee hasta el final conservando como mucho `cap` bytes (el resto se descarta, no se
/// acumula).
async fn read_capped<R: AsyncRead + Unpin>(stream: Option<R>, cap: usize) -> (Vec<u8>, bool) {
    let Some(mut stream) = stream else {
        return (Vec::new(), false);
    };
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let room = cap.saturating_sub(kept.len());
                let take = read.min(room);
                kept.extend_from_slice(&chunk[..take]);
                truncated |= take < read;
            }
        }
    }
    (kept, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names() {
        for ok in ["kanbania/h12-i3-0123abcd", "a", "feature/x.y_z"] {
            assert!(validate_branch(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "-x", "/x", "x/", "x..y", "x//y", "x.lock", "x/.y", "x y", "x~1", "x^", "x:y",
            "x@{1}", "ñ", ".x", "x.",
        ] {
            assert!(validate_branch(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn commits_and_segments() {
        assert!(validate_commit(&"a".repeat(40)).is_ok());
        assert!(validate_commit(&"0".repeat(64)).is_ok());
        assert!(validate_commit(&"A".repeat(40)).is_err());
        assert!(validate_commit("HEAD").is_err());
        assert!(validate_segment("0190-abc").is_ok());
        assert!(validate_segment("..").is_err());
        assert!(validate_segment("a/b").is_err());
        assert!(validate_segment("").is_err());
    }

    #[test]
    fn counts_status_entries_with_renames() {
        let out = b" M a.txt\0?? nuevo dir/b.txt\0R  nuevo.txt\0viejo.txt\0D  c\0!! x.log\0";
        assert_eq!(count_status_entries(out), (4, 1));
        assert_eq!(count_status_entries(b""), (0, 0));
    }

    #[test]
    fn linked_worktrees_skip_main() {
        let out = b"worktree /repo\0HEAD abc\0branch refs/heads/main\0\0worktree /wt/a b\0HEAD abc\0detached\0\0";
        assert_eq!(linked_worktrees(out), vec![PathBuf::from("/wt/a b")]);
    }
}
