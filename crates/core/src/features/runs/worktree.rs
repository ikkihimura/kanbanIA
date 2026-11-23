use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::run::{RunSummary, require_run, validate_id};
use super::start::{is_repository_linked, load_snapshot};
use super::supervisor::RunFailure;
use crate::error::CoreError;
use crate::platform::actor::Actor;
use crate::platform::clock::iso_from_ms;
use crate::platform::storage::Store;

/// Carpeta de worktrees dentro del directorio de datos.
pub const WORKTREES_DIR: &str = "worktrees";

/// Cómo se eliminó un worktree. La rama se conserva siempre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WorktreeRemoval {
    /// Estaba limpio (`git worktree remove`).
    Clean,
    /// Tenía cambios y la persona eligió descartarlos (`git worktree remove --force`).
    Discarded,
    /// La carpeta ya no existía: solo se marca como eliminado.
    Missing,
}

impl WorktreeRemoval {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Discarded => "discarded",
            Self::Missing => "missing",
        }
    }

    fn parse(value: &str) -> Result<Self, CoreError> {
        match value {
            "clean" => Ok(Self::Clean),
            "discarded" => Ok(Self::Discarded),
            "missing" => Ok(Self::Missing),
            other => Err(CoreError::Database(sqlx::Error::Decode(
                format!("eliminación de worktree desconocida: {other}").into(),
            ))),
        }
    }
}

/// Worktree registrado de un intento. `path` es una ruta absoluta local: se muestra a la
/// persona, pero nunca se expone a agentes (servicio local ni MCP).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunWorktree {
    pub run_id: String,
    /// Repositorio de la instantánea del que sale.
    pub repository_id: String,
    pub path: String,
    /// Rama creada para el intento; se conserva siempre (sus commits no se pierden).
    pub branch: String,
    /// Commit congelado en la instantánea desde el que se creó.
    pub base_commit: String,
    pub created_at: String,
    pub removed_at: Option<String>,
    pub removal: Option<WorktreeRemoval>,
    /// Cuándo se borró la rama en una limpieza (HU-025; solo con confirmación explícita).
    #[serde(default)]
    pub branch_deleted_at: Option<String>,
}

#[derive(sqlx::FromRow)]
struct WorktreeRow {
    run_id: String,
    repository_id: String,
    path: String,
    branch: String,
    base_commit: String,
    created_at: String,
    removed_at: Option<String>,
    removal: Option<String>,
    branch_deleted_at: Option<String>,
}

pub(super) async fn load_worktree(
    conn: &mut SqliteConnection,
    run_id: &str,
) -> Result<Option<RunWorktree>, CoreError> {
    let row: Option<WorktreeRow> = sqlx::query_as(concat!(
        "SELECT run_id, repository_id, path, branch, base_commit, ",
        iso_from_ms!("created_at_ms"),
        " AS created_at, ",
        iso_from_ms!("removed_at_ms"),
        " AS removed_at, removal, ",
        iso_from_ms!("branch_deleted_at_ms"),
        " AS branch_deleted_at FROM run_worktrees WHERE run_id = ?"
    ))
    .bind(run_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|r| {
        Ok(RunWorktree {
            removal: r
                .removal
                .as_deref()
                .map(WorktreeRemoval::parse)
                .transpose()?,
            run_id: r.run_id,
            repository_id: r.repository_id,
            path: r.path,
            branch: r.branch,
            base_commit: r.base_commit,
            created_at: r.created_at,
            removed_at: r.removed_at,
            branch_deleted_at: r.branch_deleted_at,
        })
    })
    .transpose()
}

/// Rama del intento: `kanbania/h<historia>-i<intento>-<8 hex>`. Los 8 hex son los **últimos**
/// del id (UUID v7): los primeros codifican la hora y se repiten durante semanas, así que no
/// distinguirían intentos de dos proyectos que comparten repositorio.
fn branch_name(story_number: i64, run_number: i64, run_id: &str) -> String {
    let hex: String = run_id.chars().filter(char::is_ascii_hexdigit).collect();
    let tail = hex.get(hex.len().saturating_sub(8)..).unwrap_or_default();
    format!(
        "kanbania/h{story_number}-i{run_number}-{}",
        tail.to_ascii_lowercase()
    )
}

/// `<data_dir>/worktrees/<project_id>/<run_id>`. Los ids los genera el núcleo (UUID), así que
/// no pueden salir de la carpeta; aun así se comprueba que sean componentes simples.
fn worktree_path(store: &Store, project_id: &str, run_id: &str) -> Result<PathBuf, CoreError> {
    let simple = |id: &str| {
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    let data_dir = store
        .data_dir()
        .filter(|d| d.is_absolute())
        .ok_or_else(|| {
            CoreError::StorageIo(std::io::Error::other(
                "el directorio de datos no es una ruta absoluta",
            ))
        })?;
    if !simple(project_id) || !simple(run_id) {
        return Err(CoreError::Validation {
            field: "runId",
            reason: "Identificador de intento no válido.".into(),
        });
    }
    Ok(data_dir.join(WORKTREES_DIR).join(project_id).join(run_id))
}

/// Carpeta raíz de worktrees del perfil (`<data_dir>/worktrees`). El runner nunca elimina nada
/// fuera de ella.
pub fn worktrees_root(store: &Store) -> Result<PathBuf, CoreError> {
    store
        .data_dir()
        .filter(|d| d.is_absolute())
        .map(|d| d.join(WORKTREES_DIR))
        .ok_or_else(|| {
            CoreError::StorageIo(std::io::Error::other(
                "el directorio de datos no es una ruta absoluta",
            ))
        })
}

/// Lo que necesita el host para comprobar o eliminar el worktree de un intento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeTarget {
    pub worktree: RunWorktree,
    /// Ruta absoluta del repositorio original (de la instantánea), para `git -C`.
    pub repository_path: PathBuf,
}

/// Worktree vigente (sin eliminar) de un intento **terminado** del proyecto, para comprobar
/// sus cambios o eliminarlo. Solo lectura.
///
/// Errores: intento de otro proyecto o inexistente, o sin worktree vigente → `not_found`;
/// intento aún activo → `validation` (primero hay que cancelarlo o esperar a que termine).
/// Uso exclusivo del host (persona): nunca desde el servicio local ni MCP.
pub async fn get_worktree_target(
    store: &Store,
    project_id: &str,
    run_id: &str,
) -> Result<WorktreeTarget, CoreError> {
    validate_id("runId", run_id)?;
    let mut conn = store.reader().acquire().await?;
    let run = require_run(&mut conn, project_id, run_id).await?;
    let worktree = live_worktree(&mut conn, &run).await?;
    let repository_path = snapshot_repository_path(&mut conn, &run.id, &worktree.repository_id)
        .await?
        .ok_or(CoreError::NotFound {
            entity: "Repositorio",
        })?;
    Ok(WorktreeTarget {
        worktree,
        repository_path,
    })
}

async fn live_worktree(
    conn: &mut SqliteConnection,
    run: &RunSummary,
) -> Result<RunWorktree, CoreError> {
    let worktree = load_worktree(conn, &run.id)
        .await?
        .filter(|w| w.removed_at.is_none())
        .ok_or(CoreError::NotFound { entity: "Worktree" })?;
    if !run.state.is_terminal() {
        return Err(CoreError::Validation {
            field: "runId",
            reason: format!(
                "El intento #{} sigue {}: cancélalo o espera a que termine antes de eliminar \
                 su worktree.",
                run.number,
                run.state.label()
            ),
        });
    }
    Ok(worktree)
}

/// Ruta del repositorio `repository_id` en la instantánea (ya deserializada) del intento;
/// `None` si no está o no es absoluta.
pub(super) async fn snapshot_repository_path(
    conn: &mut SqliteConnection,
    run_id: &str,
    repository_id: &str,
) -> Result<Option<PathBuf>, CoreError> {
    let snapshot = load_snapshot(conn, run_id).await?;
    Ok(snapshot
        .repositories
        .iter()
        .find(|r| r.id == repository_id)
        .map(|r| PathBuf::from(&r.path))
        .filter(|p| p.is_absolute()))
}

/// Destino del worktree de un intento: repositorio, commit, rama y ruta. Único cálculo que
/// usan `begin_dispatch` y `record_worktree`. `None` si el repositorio no está en la
/// instantánea, no tenía commit o su ruta no es absoluta.
pub(super) struct WorktreePlan {
    pub repository_path: PathBuf,
    pub base_commit: String,
    pub branch: String,
    pub worktree_path: PathBuf,
}

pub(super) async fn worktree_plan(
    conn: &mut SqliteConnection,
    store: &Store,
    run: &RunSummary,
    story_number: i64,
    repository_id: &str,
) -> Result<Result<WorktreePlan, RunFailure>, CoreError> {
    let snapshot = load_snapshot(conn, &run.id).await?;
    let Some(repository) = snapshot.repositories.iter().find(|r| r.id == repository_id) else {
        return Ok(Err(RunFailure::RepositoryUnavailable));
    };
    // Decisión del propietario: la instantánea no basta; el repositorio debe seguir vinculado
    // al proyecto en la misma ruta al despachar y al registrar el worktree.
    if !is_repository_linked(conn, &run.project_id, repository).await? {
        return Ok(Err(RunFailure::RepositoryUnlinked));
    }
    let repository_path = PathBuf::from(&repository.path);
    let (Some(head), true) = (repository.head.as_deref(), repository_path.is_absolute()) else {
        return Ok(Err(RunFailure::RepositoryUnavailable));
    };
    Ok(Ok(WorktreePlan {
        repository_path,
        base_commit: head.to_ascii_lowercase(),
        branch: branch_name(story_number, run.number, &run.id),
        worktree_path: worktree_path(store, &run.project_id, &run.id)?,
    }))
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecordWorktreeRemovalInput {
    pub project_id: String,
    pub run_id: String,
    pub removal: WorktreeRemoval,
}

/// Registra que el host eliminó el worktree de un intento terminado (o que ya no existía).
/// Solo personas: la eliminación es siempre una acción explícita. Una transacción
/// `BEGIN IMMEDIATE`. Repetirlo sobre un worktree ya eliminado → `not_found` (no hay worktree
/// vigente). Intento activo → `validation`.
///
/// El núcleo no toca el disco: quien llama ya ejecutó `git worktree remove` (con `--force` solo
/// si la persona eligió «Descartar cambios y eliminar»).
pub async fn record_worktree_removal(
    store: &Store,
    actor: &Actor,
    input: &RecordWorktreeRemovalInput,
) -> Result<RunWorktree, CoreError> {
    actor.require_human("eliminar el worktree de un intento")?;
    validate_id("runId", &input.run_id)?;
    let (input, clock) = (input.clone(), store.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let run = require_run(conn, &input.project_id, &input.run_id).await?;
                live_worktree(conn, &run).await?;
                sqlx::query(
                    "UPDATE run_worktrees SET removed_at_ms = ?, removal = ? \
                     WHERE run_id = ? AND removed_at_ms IS NULL",
                )
                .bind(now)
                .bind(input.removal.as_str())
                .bind(&run.id)
                .execute(&mut *conn)
                .await?;
                load_worktree(conn, &run.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Worktree" })
            })
        })
        .await
}
