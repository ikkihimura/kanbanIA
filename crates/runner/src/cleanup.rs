//! Limpieza de datos de ejecuciones (HU-025): plan (cuánto ocupa y qué se propone, sin borrar
//! nada) y limpieza de lo que la persona confirma, intento a intento. El host solo la expone
//! como comandos. Decisiones del
//! propietario: manual por antigüedad, ramas solo con casilla explícita. Los worktrees se
//! eliminan con las comprobaciones de HU-018 (cambios, ignorados, HEAD); descartar exige que la
//! persona lo marque. Rutas locales: solo para la persona, nunca para agentes. Los errores de
//! Git y de disco llegan con mensajes fijos (sin rutas ni salida de Git).

use std::path::{Path, PathBuf};

use kanbania_core::error::CoreError;
use kanbania_core::features::runs::{
    self, CleanRunInput, CleanRunResult, CleanupRecord, DiskUsage, OrphanWorktree, RUN_OUTPUT_DIR,
    RemoveOrphanWorktreeInput, RetentionBranch, RetentionItem, RetentionPlan, RetentionWorktree,
    WorktreeCheck, WorktreeRemoval,
};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::storage::Store;

use crate::retention::{Usage, measure, remove_run_output, worktree_dirs};
use crate::supervisor::git_config;
use crate::worktree::{
    DEFAULT_GIT_TIMEOUT, GitConfig, RemoveOutcome, WorktreeError, WorktreeErrorKind,
    WorktreeStatus, branch_status, delete_branch, orphan_origin, remove_worktree, worktree_status,
};

/// Error de Git o del worktree como `validation` con mensaje fijo (sin rutas ni salida de
/// Git). `force`: eliminarlo perdería algo y la persona debe marcar «descartar».
pub fn worktree_error(error: &WorktreeError) -> CoreError {
    let field = if error.needs_force() {
        "force"
    } else {
        "worktree"
    };
    eprintln!(
        "[retención] operación de worktree rechazada: {:?}",
        error.kind
    );
    CoreError::Validation {
        field,
        reason: error.to_string(),
    }
}

/// Rama que se pasa a las comprobaciones de un worktree huérfano cuyo HEAD no es una rama de
/// intento: nunca coincide, así que cuenta como «commits en riesgo» y eliminarlo exige
/// descartar (como en HU-018).
const NO_BRANCH: &str = "kanbania/sin-rama";

/// Mensaje único cuando no se sabe de qué repositorio sale un worktree.
const NO_REPOSITORY: &str =
    "No se reconoce de qué repositorio sale este worktree: no se elimina desde la app.";

fn usage(found: Option<Usage>) -> Option<DiskUsage> {
    found.map(|u| DiskUsage {
        bytes: u.bytes,
        files: u.files,
        complete: u.complete,
    })
}

async fn measured(path: PathBuf) -> Option<DiskUsage> {
    usage(
        tokio::task::spawn_blocking(move || measure(&path))
            .await
            .ok()
            .flatten(),
    )
}

/// Por qué eliminar exigiría descartar (mensajes de HU-018), o `None` si está limpio.
fn force_reason(status: &WorktreeStatus) -> Option<String> {
    let kind = if status.changes > 0 || !status.complete {
        WorktreeErrorKind::HasChanges {
            changes: status.changes,
        }
    } else if status.commits_at_risk {
        WorktreeErrorKind::CommitsAtRisk
    } else if status.ignored > 0 {
        WorktreeErrorKind::HasIgnored {
            ignored: status.ignored,
        }
    } else {
        return None;
    };
    Some(
        WorktreeError {
            kind,
            detail: String::new(),
            add_attempted: false,
        }
        .to_string(),
    )
}

async fn check(
    git: &GitConfig,
    repository: Option<&Path>,
    root: &Path,
    path: &Path,
    branch: &str,
) -> WorktreeCheck {
    let Some(repository) = repository else {
        return WorktreeCheck::Unverifiable {
            reason: NO_REPOSITORY.into(),
        };
    };
    match worktree_status(git, repository, root, path, branch).await {
        Ok(status) => match force_reason(&status) {
            None => WorktreeCheck::Clean,
            Some(reason) => WorktreeCheck::NeedsForce { reason },
        },
        Err(error) if error.kind == WorktreeErrorKind::Missing => WorktreeCheck::Missing,
        Err(error) => {
            eprintln!(
                "[retención] no se pudo comprobar un worktree: {:?}",
                error.kind
            );
            WorktreeCheck::Unverifiable {
                reason: error.to_string(),
            }
        }
    }
}

/// La misma carpeta tras resolver enlaces. Si alguna no se puede resolver: `if_unknown`
/// (al buscar huérfanos, «sí», para no tomar nunca un worktree vigente por huérfano; al elegir
/// la carpeta pedida, «no»).
fn same_dir(a: &Path, b: &Path, if_unknown: bool) -> bool {
    match (dunce::canonicalize(a), dunce::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => if_unknown,
    }
}

/// `path` no existe (y no por otro error de disco, que no prueba nada).
fn absent(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// Repositorio y rama de un worktree huérfano, solo si su `.git` señala a un repositorio
/// vinculado a algún proyecto: el `.git` lo escribe el agente y no se ejecuta Git en un
/// repositorio que él elija.
async fn known_origin(
    store: &Store,
    path: &Path,
) -> Result<Option<(PathBuf, Option<String>)>, CoreError> {
    let linked = runs::linked_repository_paths(store).await?;
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let (repository, branch) = orphan_origin(&path)?;
        linked
            .iter()
            .any(|known| same_dir(known, &repository, false))
            .then_some((repository, branch))
    })
    .await
    .map_err(|_| io_unavailable())
}

/// Carpetas de worktrees sin worktree vigente registrado (como mucho las que revisa
/// `worktree_dirs`).
async fn orphan_dirs(store: &Store, root: PathBuf) -> Result<(Vec<PathBuf>, bool), CoreError> {
    // Primero las carpetas y después las filas: un intento registra su worktree antes de
    // crearlo, así que toda carpeta listada de un intento vivo ya tiene su fila.
    let (dirs, truncated) = tokio::task::spawn_blocking(move || worktree_dirs(&root))
        .await
        .map_err(|_| io_unavailable())?;
    let live = runs::live_worktree_paths(store).await?;
    let orphans = tokio::task::spawn_blocking(move || {
        dirs.into_iter()
            .filter(|dir| !live.iter().any(|path| same_dir(path, dir, true)))
            .collect()
    })
    .await
    .map_err(|_| io_unavailable())?;
    Ok((orphans, truncated))
}

fn io_unavailable() -> CoreError {
    CoreError::StorageIo(std::io::Error::other("disco no disponible"))
}

/// Plan de limpieza: cuánto ocupan la salida de los intentos y los worktrees, qué intentos
/// terminados hace más de los días configurados se pueden limpiar (con el estado de su
/// worktree y su rama) y qué carpetas de worktrees no tienen intento (p. ej. tras restaurar
/// una copia anterior). No borra nada; solo anota (como persona) lo que ya no existe en disco
/// (salida o rama), para que no se vuelva a proponer.
pub async fn retention_plan(store: &Store, data_dir: &Path) -> Result<RetentionPlan, CoreError> {
    let root = runs::worktrees_root(store)?;
    let git = git_config(data_dir, DEFAULT_GIT_TIMEOUT);
    let runs_root = data_dir.join(RUN_OUTPUT_DIR);
    let candidates = runs::retention_candidates(store).await?;

    let mut items = Vec::new();
    for candidate in candidates.items {
        let output = match candidate.output_removed_at {
            Some(_) => None,
            None => measured(runs_root.join(&candidate.run_id)).await,
        };
        let gone = |record: CleanupRecord| {
            let (store, project_id, run_id) = (
                store.clone(),
                candidate.project_id.clone(),
                candidate.run_id.clone(),
            );
            async move {
                // Solo es una anotación: si falla (p. ej. entró una entrega), se reintenta en el
                // próximo cálculo.
                let _ =
                    runs::record_cleanup(&store, &Actor::Human, &project_id, &run_id, record).await;
            }
        };
        if candidate.output_removed_at.is_none()
            && output.is_none()
            && absent(&runs_root.join(&candidate.run_id))
        {
            gone(CleanupRecord::Output).await;
        }
        let repository = candidate.repository_path.as_deref();
        let mut worktree = None;
        let mut branch = None;
        if let Some(registered) = &candidate.worktree {
            let live = registered.removed_at.is_none();
            if live {
                let path = PathBuf::from(&registered.path);
                worktree = Some(RetentionWorktree {
                    path: registered.path.clone(),
                    branch: registered.branch.clone(),
                    usage: measured(path.clone()).await,
                    check: check(&git, repository, &root, &path, &registered.branch).await,
                });
            }
            if registered.branch_deleted_at.is_none()
                && let Some(repository) = repository
            {
                match branch_status(&git, repository, &registered.branch).await {
                    Ok(Some(unintegrated)) => {
                        branch = Some(RetentionBranch {
                            name: registered.branch.clone(),
                            unintegrated_commits: unintegrated,
                            worktree_live: live,
                        });
                    }
                    // Ya no existe (p. ej. se borró a mano): se anota si el worktree tampoco.
                    Ok(None) if !live => gone(CleanupRecord::Branch).await,
                    _ => {}
                }
            }
        }
        if output.is_none() && worktree.is_none() && branch.is_none() {
            continue;
        }
        items.push(RetentionItem {
            run_id: candidate.run_id,
            project_id: candidate.project_id,
            project_name: candidate.project_name,
            story_number: candidate.story_number,
            story_title: candidate.story_title,
            run_number: candidate.run_number,
            state: candidate.state,
            ended_at: candidate.ended_at,
            output,
            worktree,
            branch,
        });
    }

    let (orphan_paths, orphans_truncated) = orphan_dirs(store, root.clone()).await?;
    let mut orphans = Vec::new();
    for path in orphan_paths {
        let (repository, branch) = match known_origin(store, &path).await? {
            Some((repository, branch)) => (Some(repository), branch),
            None => (None, None),
        };
        let check = check(
            &git,
            repository.as_deref(),
            &root,
            &path,
            branch.as_deref().unwrap_or(NO_BRANCH),
        )
        .await;
        orphans.push(OrphanWorktree {
            path: path.to_string_lossy().into_owned(),
            branch,
            usage: measured(path).await,
            check,
        });
    }

    let empty = DiskUsage {
        bytes: 0,
        files: 0,
        complete: true,
    };
    Ok(RetentionPlan {
        retention_days: candidates.retention_days,
        cutoff: candidates.cutoff,
        runs_usage: measured(runs_root).await.unwrap_or(empty),
        worktrees_usage: measured(root).await.unwrap_or(empty),
        items,
        items_truncated: candidates.truncated,
        orphans,
        orphans_truncated,
    })
}

/// Limpia lo que la persona marcó de un intento, en este orden: worktree (descartando solo si
/// lo marcó), salida y rama (solo con el worktree eliminado antes o en esta misma limpieza).
/// Todas las precondiciones se comprueban antes de borrar nada; antes de cada paso se vuelve a
/// comprobar que el intento está terminado y sin entrega pendiente, y cada paso queda
/// registrado (el registro lo comprueba otra vez en su transacción). Si un paso falla, los
/// anteriores ya están hechos y registrados. Queda una ventana (lo que tarde Git) entre la
/// comprobación y el borrado.
pub async fn clean_run(
    store: &Store,
    actor: &Actor,
    data_dir: &Path,
    input: &CleanRunInput,
) -> Result<CleanRunResult, CoreError> {
    actor.require_human("limpiar los datos de un intento")?;
    let target = runs::cleanup_target(store, &input.project_id, &input.run_id).await?;
    let root = runs::worktrees_root(store)?;
    let git = git_config(data_dir, DEFAULT_GIT_TIMEOUT);
    let record = |record: CleanupRecord| {
        let store = store.clone();
        let (project_id, run_id) = (input.project_id.clone(), input.run_id.clone());
        async move { runs::record_cleanup(&store, actor, &project_id, &run_id, record).await }
    };
    let repository = || {
        target
            .repository_path
            .clone()
            .ok_or_else(|| CoreError::Validation {
                field: "worktree",
                reason: NO_REPOSITORY.into(),
            })
    };
    let mut result = CleanRunResult {
        output_removed: false,
        worktree_removal: None,
        branch_deleted: false,
    };
    let worktree_removed = target
        .worktree
        .as_ref()
        .is_some_and(|w| w.removed_at.is_some());
    // Todas las precondiciones antes de borrar nada.
    if input.worktree
        && target
            .worktree
            .as_ref()
            .is_none_or(|w| w.removed_at.is_some())
    {
        return Err(CoreError::NotFound { entity: "Worktree" });
    }
    if input.branch {
        if target.worktree.is_none() {
            return Err(CoreError::NotFound { entity: "Worktree" });
        }
        if !worktree_removed && !input.worktree {
            return Err(CoreError::Validation {
                field: "branch",
                reason: "Primero hay que eliminar el worktree del intento: Git no borra una rama \
                         en uso."
                    .into(),
            });
        }
    }
    if (input.worktree || input.branch) && target.repository_path.is_none() {
        return Err(CoreError::Validation {
            field: "worktree",
            reason: NO_REPOSITORY.into(),
        });
    }
    // Antes de cada paso que borra se vuelve a comprobar (terminado y sin entrega pendiente);
    // después, el registro lo comprueba otra vez en su transacción.
    let recheck = || runs::cleanup_target(store, &input.project_id, &input.run_id);

    if input.worktree {
        let worktree = target
            .worktree
            .as_ref()
            .filter(|w| w.removed_at.is_none())
            .ok_or(CoreError::NotFound { entity: "Worktree" })?;
        recheck().await?;
        let outcome = remove_worktree(
            &git,
            &repository()?,
            &root,
            Path::new(&worktree.path),
            &worktree.branch,
            input.discard_worktree_changes,
        )
        .await
        .map_err(|e| worktree_error(&e))?;
        let removal = match outcome {
            RemoveOutcome::Removed => WorktreeRemoval::Clean,
            RemoveOutcome::Forced => WorktreeRemoval::Discarded,
            RemoveOutcome::Missing => WorktreeRemoval::Missing,
        };
        record(CleanupRecord::Worktree(removal)).await?;
        result.worktree_removal = Some(removal);
    }

    if input.output {
        let runs_root = data_dir.join(RUN_OUTPUT_DIR);
        let run_id = target.run.id.clone();
        recheck().await?;
        let removed = tokio::task::spawn_blocking(move || remove_run_output(&runs_root, &run_id))
            .await
            .map_err(|_| io_unavailable())?;
        if let Err(error) = removed {
            eprintln!(
                "[retención] no se pudo borrar la salida: {:?}",
                error.kind()
            );
            return Err(io_unavailable());
        }
        record(CleanupRecord::Output).await?;
        result.output_removed = true;
    }

    if input.branch {
        let worktree = target
            .worktree
            .as_ref()
            .ok_or(CoreError::NotFound { entity: "Worktree" })?;
        recheck().await?;
        let deleted = delete_branch(&git, &repository()?, &worktree.branch)
            .await
            .map_err(|e| worktree_error(&e))?;
        record(CleanupRecord::Branch).await?;
        result.branch_deleted = deleted;
    }
    Ok(result)
}

/// Elimina una carpeta de worktree huérfana (sin intento registrado) con las comprobaciones de
/// HU-018: debe seguir siendo huérfana, señalar a un repositorio vinculado que la tenga
/// registrada (y que la señale a ella) y, si
/// tiene algo que se perdería, la persona debe haber marcado descartarlo. La rama se conserva.
pub async fn remove_orphan_worktree(
    store: &Store,
    actor: &Actor,
    data_dir: &Path,
    input: &RemoveOrphanWorktreeInput,
) -> Result<WorktreeRemoval, CoreError> {
    actor.require_human("eliminar un worktree")?;
    let root = runs::worktrees_root(store)?;
    let requested = PathBuf::from(&input.path);
    let (orphans, _) = orphan_dirs(store, root.clone()).await?;
    let Some(path) = orphans
        .into_iter()
        .find(|dir| same_dir(dir, &requested, false))
    else {
        return Err(CoreError::NotFound { entity: "Worktree" });
    };
    let Some((repository, branch)) = known_origin(store, &path).await? else {
        return Err(CoreError::Validation {
            field: "worktree",
            reason: NO_REPOSITORY.into(),
        });
    };
    let git = git_config(data_dir, DEFAULT_GIT_TIMEOUT);
    let outcome = remove_worktree(
        &git,
        &repository,
        &root,
        &path,
        branch.as_deref().unwrap_or(NO_BRANCH),
        input.discard_changes,
    )
    .await
    .map_err(|e| worktree_error(&e))?;
    Ok(match outcome {
        RemoveOutcome::Removed => WorktreeRemoval::Clean,
        RemoveOutcome::Forced => WorktreeRemoval::Discarded,
        RemoveOutcome::Missing => WorktreeRemoval::Missing,
    })
}
