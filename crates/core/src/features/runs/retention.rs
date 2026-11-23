//! Retención de datos de ejecuciones (HU-025). Decisiones del propietario:
//!
//! - Limpieza **manual** por antigüedad: el núcleo propone los intentos terminados hace más de
//!   `run_settings.retention_days` días; la persona revisa y confirma cada cosa. Nada se borra
//!   solo.
//! - Nunca se toca un intento activo ni uno cuya historia tiene una entrega pendiente de
//!   revisión ([`ensure_cleanable`], que vuelve a comprobarse en cada acción).
//! - Las ramas se conservan salvo que la persona marque una por una (y solo tras eliminar el
//!   worktree, que Git exige).
//! - Los intentos y sus eventos nunca se borran (triggers de 0011): solo la salida
//!   (`<data_dir>/runs/<id>/`), el worktree y la rama, y queda constancia de cuándo.
//!
//! El núcleo no toca el disco: el host mide, borra y después registra aquí lo que borró.
//! Rutas locales: solo para la persona, nunca para agentes.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::run::{RunState, RunSummary, require_run, validate_id};
use super::settings::load_settings;
use super::worktree::{RunWorktree, WorktreeRemoval, load_worktree, snapshot_repository_path};
use crate::error::CoreError;
use crate::features::agent_work::pending_delivery;
use crate::platform::actor::Actor;
use crate::platform::clock::iso_from_ms;
use crate::platform::storage::Store;

/// Como mucho tantos intentos por plan (los más antiguos primero); si hay más, `truncated`.
pub const MAX_RETENTION_ITEMS: usize = 500;
const DAY_MS: i64 = 86_400_000;

/// Un intento terminado que se puede limpiar, con lo que el host necesita para medirlo y
/// comprobarlo. `repository_path` no se serializa: lo usa el host para `git -C`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionCandidate {
    pub run_id: String,
    pub project_id: String,
    pub project_name: String,
    pub story_number: i64,
    pub story_title: String,
    pub run_number: i64,
    pub state: RunState,
    pub ended_at: String,
    /// La salida ya se borró en una limpieza anterior.
    pub output_removed_at: Option<String>,
    /// Worktree registrado (vigente o eliminado), con su rama.
    pub worktree: Option<RunWorktree>,
    /// Repositorio original del worktree (de la instantánea), si es una ruta absoluta.
    pub repository_path: Option<PathBuf>,
}

/// Intentos propuestos para limpiar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionCandidates {
    pub retention_days: i64,
    /// Terminados antes de esta hora (ms del reloj del núcleo) e ISO para mostrarla.
    pub cutoff_ms: i64,
    pub cutoff: String,
    pub items: Vec<RetentionCandidate>,
    /// Había más de [`MAX_RETENTION_ITEMS`]: limpia y vuelve a calcular.
    pub truncated: bool,
}

#[derive(sqlx::FromRow)]
struct CandidateRow {
    id: String,
    project_id: String,
    project_name: String,
    story_number: i64,
    story_title: String,
    run_number: i64,
    state: String,
    ended_at: String,
    output_removed_at: Option<String>,
}

/// Intentos terminados hace más de `retention_days` días que aún pueden tener algo que limpiar
/// (la salida sin borrar de un intento que llegó a despacharse, un worktree vigente o una rama
/// sin borrar), sin entrega pendiente en su historia, del más antiguo al más reciente. Lo que
/// el host ya no encuentra en disco lo anota con [`record_cleanup`] para que no se proponga
/// otra vez. Solo lectura; la hora sale del reloj del núcleo.
pub async fn retention_candidates(store: &Store) -> Result<RetentionCandidates, CoreError> {
    let mut conn = store.reader().acquire().await?;
    let retention_days = load_settings(&mut conn).await?.retention_days;
    let cutoff_ms = store.now_ms() - retention_days * DAY_MS;
    let cutoff: String = sqlx::query_scalar(concat!("SELECT ", iso_from_ms!("?")))
        .bind(cutoff_ms)
        .fetch_one(&mut *conn)
        .await?;
    let limit = i64::try_from(MAX_RETENTION_ITEMS + 1).unwrap_or(i64::MAX);
    let rows: Vec<CandidateRow> = sqlx::query_as(concat!(
        "SELECT r.id, r.project_id, p.name AS project_name, s.number AS story_number, ",
        "s.title AS story_title, r.number AS run_number, r.state, ",
        iso_from_ms!("r.ended_at_ms"),
        " AS ended_at, ",
        iso_from_ms!("r.output_removed_at_ms"),
        " AS output_removed_at ",
        "FROM runs r JOIN stories s ON s.id = r.story_id JOIN projects p ON p.id = r.project_id ",
        "WHERE r.ended_at_ms IS NOT NULL AND r.ended_at_ms < ? AND NOT ",
        pending_delivery!(),
        // La salida solo existe si el intento llegó a despacharse (o a tener worktree): un
        // intento cancelado en la cola no tiene nada que limpiar y nunca se propone.
        " AND ((r.output_removed_at_ms IS NULL AND (EXISTS (SELECT 1 FROM run_worktrees w0 ",
        "WHERE w0.run_id = r.id) OR EXISTS (SELECT 1 FROM run_start_requests q ",
        "WHERE q.run_id = r.id AND q.dispatched_at_ms IS NOT NULL))) ",
        "OR EXISTS (SELECT 1 FROM run_worktrees w ",
        "WHERE w.run_id = r.id AND (w.removed_at_ms IS NULL OR w.branch_deleted_at_ms IS NULL))) ",
        "ORDER BY r.ended_at_ms, r.id LIMIT ?"
    ))
    .bind(cutoff_ms)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    let truncated = rows.len() > MAX_RETENTION_ITEMS;
    let mut items = Vec::with_capacity(rows.len().min(MAX_RETENTION_ITEMS));
    for row in rows.into_iter().take(MAX_RETENTION_ITEMS) {
        let worktree = load_worktree(&mut conn, &row.id).await?;
        let repository_path = match &worktree {
            Some(worktree) => {
                snapshot_repository_path(&mut conn, &row.id, &worktree.repository_id).await?
            }
            None => None,
        };
        items.push(RetentionCandidate {
            state: RunState::parse(&row.state)?,
            run_id: row.id,
            project_id: row.project_id,
            project_name: row.project_name,
            story_number: row.story_number,
            story_title: row.story_title,
            run_number: row.run_number,
            ended_at: row.ended_at,
            output_removed_at: row.output_removed_at,
            worktree,
            repository_path,
        });
    }
    Ok(RetentionCandidates {
        retention_days,
        cutoff_ms,
        cutoff,
        items,
        truncated,
    })
}

/// Se puede limpiar: el intento está terminado hace más de los días de retención (la misma
/// regla que [`retention_candidates`]; la UI solo propone esos, y el núcleo no se fía de ella)
/// y su historia no tiene una entrega pendiente de revisión. Si no → `validation` (`runId`)
/// con el motivo.
async fn ensure_cleanable(
    conn: &mut SqliteConnection,
    project_id: &str,
    run_id: &str,
    now_ms: i64,
) -> Result<RunSummary, CoreError> {
    validate_id("runId", run_id)?;
    let run = require_run(conn, project_id, run_id).await?;
    if !run.state.is_terminal() {
        return Err(CoreError::Validation {
            field: "runId",
            reason: format!(
                "El intento #{} sigue {}: no se limpia un intento activo.",
                run.number,
                run.state.label()
            ),
        });
    }
    let retention_days = load_settings(conn).await?.retention_days;
    let ended_at_ms: Option<i64> = sqlx::query_scalar("SELECT ended_at_ms FROM runs WHERE id = ?")
        .bind(&run.id)
        .fetch_one(&mut *conn)
        .await?;
    if ended_at_ms.is_none_or(|ended| ended >= now_ms - retention_days * DAY_MS) {
        return Err(CoreError::Validation {
            field: "runId",
            reason: format!(
                "El intento #{} terminó hace menos de {retention_days} días: todavía no se \
                 limpia.",
                run.number
            ),
        });
    }
    let pending: bool = sqlx::query_scalar(concat!(
        "SELECT ",
        pending_delivery!(),
        " FROM stories s WHERE s.id = ?"
    ))
    .bind(&run.story_id)
    .fetch_one(&mut *conn)
    .await?;
    if pending {
        return Err(CoreError::Validation {
            field: "runId",
            reason: "La historia tiene una entrega pendiente de revisión: sus intentos no se \
                     limpian hasta aceptarla o devolverla."
                .into(),
        });
    }
    Ok(run)
}

/// Lo que el host necesita para limpiar un intento concreto (comprobado con
/// [`ensure_cleanable`]). Solo lectura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupTarget {
    pub run: RunSummary,
    pub worktree: Option<RunWorktree>,
    pub repository_path: Option<PathBuf>,
}

/// Comprueba que el intento se puede limpiar y devuelve su worktree y repositorio. Errores:
/// otro proyecto o inexistente → `not_found`; activo o con entrega pendiente → `validation`.
pub async fn cleanup_target(
    store: &Store,
    project_id: &str,
    run_id: &str,
) -> Result<CleanupTarget, CoreError> {
    let mut conn = store.reader().acquire().await?;
    let run = ensure_cleanable(&mut conn, project_id, run_id, store.now_ms()).await?;
    let worktree = load_worktree(&mut conn, &run.id).await?;
    let repository_path = match &worktree {
        Some(worktree) => {
            snapshot_repository_path(&mut conn, &run.id, &worktree.repository_id).await?
        }
        None => None,
    };
    Ok(CleanupTarget {
        run,
        worktree,
        repository_path,
    })
}

/// Qué borró el host en una limpieza.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupRecord {
    /// La salida (`runs/<id>/`), o constancia de que no existía.
    Output,
    /// El worktree, como en HU-018 (la rama se conserva).
    Worktree(WorktreeRemoval),
    /// La rama del intento (el worktree ya estaba eliminado).
    Branch,
}

/// Registra lo que el host borró al limpiar un intento. Solo personas; vuelve a comprobar en
/// la transacción que el intento se puede limpiar. Repetir `Output` o `Branch` no cambia la
/// hora registrada; `Worktree` exige un worktree vigente (`not_found` si no) y `Branch`, uno
/// ya eliminado (`validation` si no).
pub async fn record_cleanup(
    store: &Store,
    actor: &Actor,
    project_id: &str,
    run_id: &str,
    record: CleanupRecord,
) -> Result<(), CoreError> {
    actor.require_human("limpiar los datos de un intento")?;
    let (project_id, run_id, clock) = (project_id.to_owned(), run_id.to_owned(), store.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let run = ensure_cleanable(conn, &project_id, &run_id, now).await?;
                match record {
                    CleanupRecord::Output => {
                        sqlx::query(
                            "UPDATE runs SET output_removed_at_ms = ? \
                             WHERE id = ? AND output_removed_at_ms IS NULL",
                        )
                        .bind(now)
                        .bind(&run.id)
                        .execute(&mut *conn)
                        .await?;
                    }
                    CleanupRecord::Worktree(removal) => {
                        let updated = sqlx::query(
                            "UPDATE run_worktrees SET removed_at_ms = ?, removal = ? \
                             WHERE run_id = ? AND removed_at_ms IS NULL",
                        )
                        .bind(now)
                        .bind(removal.as_str())
                        .bind(&run.id)
                        .execute(&mut *conn)
                        .await?
                        .rows_affected();
                        if updated == 0 {
                            return Err(CoreError::NotFound { entity: "Worktree" });
                        }
                    }
                    CleanupRecord::Branch => {
                        let worktree = load_worktree(conn, &run.id)
                            .await?
                            .ok_or(CoreError::NotFound { entity: "Worktree" })?;
                        if worktree.removed_at.is_none() {
                            return Err(CoreError::Validation {
                                field: "runId",
                                reason: "Primero hay que eliminar el worktree del intento: Git \
                                         no borra una rama en uso."
                                    .into(),
                            });
                        }
                        sqlx::query(
                            "UPDATE run_worktrees SET branch_deleted_at_ms = ? \
                             WHERE run_id = ? AND branch_deleted_at_ms IS NULL",
                        )
                        .bind(now)
                        .bind(&run.id)
                        .execute(&mut *conn)
                        .await?;
                    }
                }
                Ok(())
            })
        })
        .await
}

/// Rutas de los worktrees vigentes registrados (de cualquier estado del intento): el resto de
/// carpetas `worktrees/<proyecto>/<intento>` son huérfanas (p. ej. tras restaurar una copia
/// anterior). Solo lectura.
pub async fn live_worktree_paths(store: &Store) -> Result<Vec<PathBuf>, CoreError> {
    let mut conn = store.reader().acquire().await?;
    let paths: Vec<String> =
        sqlx::query_scalar("SELECT path FROM run_worktrees WHERE removed_at_ms IS NULL")
            .fetch_all(&mut *conn)
            .await?;
    Ok(paths.into_iter().map(PathBuf::from).collect())
}

/// Cuánto ocupa algo en disco (lo mide el host, con límites).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiskUsage {
    #[ts(type = "number")]
    pub bytes: u64,
    #[ts(type = "number")]
    pub files: u64,
    /// `false` si se alcanzó el límite de archivos o de tiempo: los valores son un mínimo.
    pub complete: bool,
}

/// Estado de un worktree según Git (las comprobaciones de HU-018).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[ts(export)]
pub enum WorktreeCheck {
    /// Sin cambios, ignorados ni commits en riesgo: se elimina sin perder nada.
    Clean,
    /// Eliminarlo exige descartar lo que tiene (mensaje fijo de Git/HU-018, sin rutas).
    NeedsForce { reason: String },
    /// La carpeta ya no existe: solo se marca como eliminado.
    Missing,
    /// No se pudo comprobar (repositorio movido, no registrado, Git no disponible…): no se
    /// ofrece eliminarlo.
    Unverifiable { reason: String },
}

/// Worktree vigente de un intento propuesto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RetentionWorktree {
    pub path: String,
    pub branch: String,
    pub usage: Option<DiskUsage>,
    pub check: WorktreeCheck,
}

/// Rama de un intento propuesto que aún existe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RetentionBranch {
    pub name: String,
    /// Commits que no están en ninguna otra rama («trabajo sin integrar»); `null` = no se pudo
    /// saber.
    #[ts(type = "number | null")]
    pub unintegrated_commits: Option<u64>,
    /// Solo se puede borrar con el worktree eliminado (o marcado para eliminar en la misma
    /// limpieza).
    pub worktree_live: bool,
}

/// Un intento propuesto para limpiar, con lo que ocupa.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RetentionItem {
    pub run_id: String,
    pub project_id: String,
    pub project_name: String,
    #[ts(type = "number")]
    pub story_number: i64,
    pub story_title: String,
    #[ts(type = "number")]
    pub run_number: i64,
    pub state: RunState,
    pub ended_at: String,
    /// Salida del intento (`null` si no hay o ya se borró).
    pub output: Option<DiskUsage>,
    pub worktree: Option<RetentionWorktree>,
    pub branch: Option<RetentionBranch>,
}

/// Carpeta `worktrees/<proyecto>/<intento>` sin worktree vigente registrado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OrphanWorktree {
    pub path: String,
    /// Rama según su HEAD, si es una rama.
    pub branch: Option<String>,
    pub usage: Option<DiskUsage>,
    pub check: WorktreeCheck,
}

/// Plan de limpieza: cuánto ocupa todo y qué se propone borrar. No borra nada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RetentionPlan {
    #[ts(type = "number")]
    pub retention_days: i64,
    /// Se proponen los intentos terminados antes de esta fecha.
    pub cutoff: String,
    /// Toda la salida de intentos (`runs/`) y todos los worktrees (`worktrees/`).
    pub runs_usage: DiskUsage,
    pub worktrees_usage: DiskUsage,
    pub items: Vec<RetentionItem>,
    pub items_truncated: bool,
    pub orphans: Vec<OrphanWorktree>,
    /// Había más carpetas huérfanas de las que se revisan de una vez.
    pub orphans_truncated: bool,
}

/// Qué limpiar de un intento (lo elige la persona en la lista).
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CleanRunInput {
    pub project_id: String,
    pub run_id: String,
    pub output: bool,
    pub worktree: bool,
    /// Descartar lo que tenga el worktree (cambios, ignorados, commits fuera de rama).
    pub discard_worktree_changes: bool,
    /// Borrar la rama (casilla explícita por rama).
    pub branch: bool,
}

/// Resultado de limpiar un intento.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CleanRunResult {
    pub output_removed: bool,
    pub worktree_removal: Option<WorktreeRemoval>,
    pub branch_deleted: bool,
}

/// Eliminar una carpeta huérfana (mismas comprobaciones que un worktree de un intento).
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoveOrphanWorktreeInput {
    pub path: String,
    pub discard_changes: bool,
}

/// Rutas de los repositorios vinculados a algún proyecto (tal como se guardaron). La limpieza
/// solo ejecuta Git sobre un worktree huérfano si su `.git` señala a uno de ellos: el `.git` lo
/// escribe el agente. Solo lectura.
pub async fn linked_repository_paths(store: &Store) -> Result<Vec<PathBuf>, CoreError> {
    let mut conn = store.reader().acquire().await?;
    let paths: Vec<String> = sqlx::query_scalar("SELECT DISTINCT path FROM repository_links")
        .fetch_all(&mut *conn)
        .await?;
    Ok(paths.into_iter().map(PathBuf::from).collect())
}
