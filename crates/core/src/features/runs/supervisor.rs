//! API del núcleo para el supervisor de ejecuciones (HU-018, `crates/runner`).
//!
//! Toda función de este módulo exige un [`SupervisorToken`] del mismo almacén. Rust no permite
//! restringir una función `pub` a un crate concreto, así que la garantía es esta (la más
//! honesta posible sin `unsafe`):
//!
//! - el token no se puede construir fuera de [`SupervisorToken::claim`] (campo privado), no es
//!   `Clone` y solo puede existir **uno a la vez por almacén** (los clones del `Store` comparten
//!   la marca): un segundo `claim` falla mientras el primero viva;
//! - cada función comprueba que el token es del almacén que recibe;
//! - `claim` solo se llama desde el host (al arrancar el supervisor, igual que el servicio
//!   local). Ni `crates/local-service` ni `crates/mcp` lo llaman: es el punto de auditoría
//!   (búsqueda de `SupervisorToken::claim`). Nada de esto se expone a agentes.
//!
//! Flujo: `next_dispatchable` → `begin_dispatch` (`queued` → `preparing` o `failed`) → crear
//! worktree en disco → `record_worktree` → lanzar el proceso → `mark_running` → `finish_run`.
//! En paralelo, `pending_cancellations` dice qué árboles matar. Al arrancar el host (antes del
//! supervisor) y al salir: `interrupt_unverifiable_runs` y `void_start_requests`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sqlx::SqliteConnection;

use super::run::{RunState, RunSummary, require_run_any, transition, validate_id};
use super::runnable::ensure_runnable;
use super::settings::load_settings;
use super::start::TestScenario;
use super::worktree::{RunWorktree, load_worktree, worktree_plan};
use crate::error::CoreError;
use crate::features::stories::load_story;
use crate::platform::storage::Store;

/// Permiso del supervisor de ejecuciones sobre un almacén. Ver la documentación del módulo.
pub struct SupervisorToken {
    flag: Arc<AtomicBool>,
}

impl std::fmt::Debug for SupervisorToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SupervisorToken")
    }
}

impl SupervisorToken {
    /// **Solo el host** al arrancar el supervisor de ejecuciones. Falla con `forbidden` si ya
    /// hay un token vivo para este almacén (un solo supervisor por perfil). Al soltarse el
    /// token, se puede volver a pedir (p. ej. tras restaurar una copia, con el almacén nuevo).
    pub fn claim(store: &Store) -> Result<Self, CoreError> {
        let flag = store.supervisor_flag().clone();
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| CoreError::Forbidden {
                reason: "Ya hay un supervisor de ejecuciones activo para estos datos.".into(),
            })?;
        Ok(Self { flag })
    }

    fn check(&self, store: &Store) -> Result<(), CoreError> {
        if Arc::ptr_eq(&self.flag, store.supervisor_flag()) {
            Ok(())
        } else {
            Err(CoreError::Forbidden {
                reason: "El supervisor no corresponde a estos datos.".into(),
            })
        }
    }
}

impl Drop for SupervisorToken {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

/// Petición de inicio pendiente más antigua (ver [`next_dispatchable`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingStart {
    pub run_id: String,
    pub project_id: String,
    pub requested_at_ms: i64,
}

/// Todo lo que el supervisor necesita para preparar y lanzar un intento despachado. Contiene
/// rutas absolutas locales: nunca se envía a agentes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatch {
    pub run_id: String,
    pub project_id: String,
    pub story_id: String,
    pub story_number: i64,
    pub run_number: i64,
    /// Repositorio de la instantánea elegido al iniciar.
    pub repository_id: String,
    /// Ruta absoluta del repositorio original (de la instantánea): `git -C`, nunca el `cwd`
    /// del proceso.
    pub repository_path: PathBuf,
    /// Commit congelado en la instantánea: la referencia explícita del worktree.
    pub base_commit: String,
    /// `kanbania/h<n>-i<n>-<8 hex>`; si ya existe, el intento falla (nunca se reutiliza).
    pub branch: String,
    /// `<data_dir>/worktrees/<project_id>/<run_id>`: `cwd` del proceso.
    pub worktree_path: PathBuf,
    pub scenario: TestScenario,
    /// Tiempo máximo leído de la configuración al despachar.
    pub timeout_minutes: i64,
}

/// Resultado de [`begin_dispatch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// El intento pasó a `preparing`: crear el worktree y lanzar el proceso.
    Ready(Dispatch),
    /// La historia ya no es ejecutable: el intento pasó a `failed` con el motivo.
    Rejected(RunSummary),
    /// Ya no hay nada que despachar (se canceló, se anuló la petición o ya se despachó).
    Stale,
}

/// Por qué no se pudo preparar o lanzar un intento. El núcleo genera el motivo (seguro, sin
/// rutas ni salida de git); los detalles van al registro JSONL del intento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunFailure {
    /// El repositorio no está disponible o no contiene el commit de la instantánea.
    RepositoryUnavailable,
    /// El repositorio de la instantánea ya no está vinculado al proyecto (o se reubicó).
    RepositoryUnlinked,
    /// La rama del intento ya existe en el repositorio.
    BranchExists,
    /// Falló `git worktree add` por otro motivo.
    WorktreeFailed,
    /// No se pudo lanzar el proceso.
    SpawnFailed,
    /// Error del supervisor (p. ej. al escribir la salida).
    Internal,
}

impl RunFailure {
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::RepositoryUnlinked => {
                "El repositorio ya no está vinculado al proyecto (o se reubicó): prepara un \
                 intento nuevo con los repositorios vinculados ahora."
            }
            Self::RepositoryUnavailable => {
                "El repositorio no está disponible o no contiene el commit de la instantánea."
            }
            Self::BranchExists => {
                "La rama del intento ya existe en el repositorio: nunca se reutiliza ni se \
                 sobrescribe."
            }
            Self::WorktreeFailed => {
                "No se pudo crear el worktree del intento. Consulta el registro del intento."
            }
            Self::SpawnFailed => "No se pudo lanzar el proceso de prueba.",
            Self::Internal => {
                "El supervisor detuvo el intento por un error interno. Consulta el registro del \
                 intento."
            }
        }
    }
}

/// Cómo terminó el proceso (o el intento) según el supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// El proceso terminó con este código: 0 → `completed` (no acepta la historia ni la
    /// mueve); otro → `failed` («Terminó con código n.»).
    Exited { code: i32 },
    /// Lo terminó una señal que no envió el supervisor → `failed`.
    Signaled { signal: Option<i32> },
    /// Superó el tiempo máximo y el supervisor mató su árbol → `failed`.
    TimedOut { minutes: i64 },
    /// El supervisor mató el árbol por una petición de cancelación → `cancelled` con el motivo
    /// de la petición. Sin petición registrada → `validation`.
    Cancelled,
    /// No se pudo preparar o lanzar → `failed` con un motivo del núcleo.
    Failed(RunFailure),
}

/// Por qué se interrumpen los intentos no verificables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptCause {
    /// Al arrancar el host: intentos que quedaron activos de una sesión anterior.
    AppRestarted,
    /// Al salir la app normalmente, tras matar los árboles de procesos.
    AppExiting,
    /// Antes de restaurar una copia de los datos, tras matar los árboles de procesos.
    Restoring,
}

impl InterruptCause {
    fn reason(self) -> &'static str {
        match self {
            Self::AppRestarted => {
                "La app se cerró durante la ejecución: no se puede verificar el proceso."
            }
            Self::AppExiting => "Se cerró la app.",
            Self::Restoring => {
                "Se restauró una copia de los datos: no se puede verificar el proceso."
            }
        }
    }
}

/// Intento con una petición de cancelación pendiente de confirmar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCancellation {
    pub run_id: String,
    pub project_id: String,
    pub state: RunState,
}

/// Petición de inicio vigente más antigua (FIFO por hora de la petición) cuyo intento sigue en
/// `queued`. Solo lectura; no reserva nada: el supervisor llama después a [`begin_dispatch`],
/// que lo vuelve a comprobar en su transacción.
pub async fn next_dispatchable(
    store: &Store,
    token: &SupervisorToken,
) -> Result<Option<PendingStart>, CoreError> {
    token.check(store)?;
    let row: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT q.run_id, r.project_id, q.requested_at_ms FROM run_start_requests q \
         JOIN runs r ON r.id = q.run_id \
         WHERE q.voided_at_ms IS NULL AND q.dispatched_at_ms IS NULL AND r.state = 'queued' \
         ORDER BY q.requested_at_ms, q.id LIMIT 1",
    )
    .fetch_optional(store.reader())
    .await?;
    Ok(
        row.map(|(run_id, project_id, requested_at_ms)| PendingStart {
            run_id,
            project_id,
            requested_at_ms,
        }),
    )
}

/// Despacha un intento con petición de inicio vigente, en una transacción `BEGIN IMMEDIATE`:
///
/// - si ya no está `queued` o su petición se anuló o despachó → `Stale` (sin cambios);
/// - si el intento no se puede leer (error persistente) → anula su petición y `Stale`;
/// - un error persistente tras consumir la petición → `failed` con `RunFailure::Internal`;
/// - vuelve a evaluar `ensure_runnable` con la hora actual: si la historia ya no es ejecutable
///   (archivada, aceptada, bloqueada, prerequisitos o entrega pendientes, reclamo vigente de
///   otro agente) → `queued → failed` con el motivo y `Rejected`;
/// - si no → marca la petición como despachada, `queued → preparing` y devuelve `Ready` con
///   el repositorio, el commit, la rama, la ruta del worktree y el tiempo máximo vigente.
///
/// Errores: token de otro almacén → `forbidden`; intento inexistente → `not_found`; base de
/// datos → `storage_unavailable`.
pub async fn begin_dispatch(
    store: &Store,
    token: &SupervisorToken,
    run_id: &str,
) -> Result<DispatchOutcome, CoreError> {
    token.check(store)?;
    validate_id("runId", run_id)?;
    let (run_id, clock) = (run_id.to_owned(), store.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let request: Option<(i64, String, String)> = sqlx::query_as(
                    "SELECT id, repository_id, scenario FROM run_start_requests \
                     WHERE run_id = ? AND voided_at_ms IS NULL AND dispatched_at_ms IS NULL",
                )
                .bind(&run_id)
                .fetch_optional(&mut *conn)
                .await?;
                let Some((request_id, repository_id, scenario)) = request else {
                    return Ok(DispatchOutcome::Stale);
                };
                let run = match require_run_any(conn, &run_id).await {
                    Ok(run) => run,
                    Err(error) if is_transient(&error) => return Err(error),
                    // El intento no se puede leer: no se puede pasar a `failed`, pero su petición
                    // se anula para que no bloquee la cola (la persona verá el intento en cola,
                    // sin petición).
                    Err(_) => {
                        sqlx::query("UPDATE run_start_requests SET voided_at_ms = ? WHERE id = ?")
                            .bind(now)
                            .bind(request_id)
                            .execute(&mut *conn)
                            .await?;
                        return Ok(DispatchOutcome::Stale);
                    }
                };
                if run.state != RunState::Queued {
                    return Ok(DispatchOutcome::Stale);
                }
                sqlx::query("UPDATE run_start_requests SET dispatched_at_ms = ? WHERE id = ?")
                    .bind(now)
                    .bind(request_id)
                    .execute(&mut *conn)
                    .await?;

                // A partir de aquí la petición está consumida: un error persistente no puede
                // dejarla pendiente (bloquearía la cola FIFO), así que el intento falla. Solo
                // un error transitorio de la base devuelve `Err` (y deshace todo).
                match prepare(conn, &clock, &run, &repository_id, &scenario, now).await {
                    Ok(Prepared::Ready(dispatch)) => {
                        transition(conn, &run, RunState::Preparing, None, now).await?;
                        Ok(DispatchOutcome::Ready(dispatch))
                    }
                    Ok(Prepared::Rejected(reason)) => reject(conn, &run, &reason, now).await,
                    Err(error) if is_transient(&error) => Err(error),
                    Err(_) => reject(conn, &run, RunFailure::Internal.reason(), now).await,
                }
            })
        })
        .await
}

enum Prepared {
    Ready(Dispatch),
    Rejected(String),
}

/// Reevalúa `ensure_runnable` y calcula el destino del worktree.
async fn prepare(
    conn: &mut SqliteConnection,
    store: &Store,
    run: &RunSummary,
    repository_id: &str,
    scenario: &str,
    now: i64,
) -> Result<Prepared, CoreError> {
    let story = load_story(conn, &run.project_id, &run.story_id)
        .await?
        .ok_or(CoreError::NotFound { entity: "Historia" })?;
    match ensure_runnable(conn, &story, &run.agent_id, now).await {
        Ok(()) => {}
        Err(CoreError::Validation { reason, .. }) => {
            return Ok(Prepared::Rejected(format!("No se pudo iniciar: {reason}")));
        }
        Err(e @ CoreError::ClaimedForRun) => {
            return Ok(Prepared::Rejected(format!("No se pudo iniciar: {e}")));
        }
        Err(other) => return Err(other),
    }
    let plan = match worktree_plan(conn, store, run, story.number, repository_id).await? {
        Ok(plan) => plan,
        Err(failure) => return Ok(Prepared::Rejected(failure.reason().to_owned())),
    };
    let settings = load_settings(conn).await?;
    Ok(Prepared::Ready(Dispatch {
        worktree_path: plan.worktree_path,
        branch: plan.branch,
        run_id: run.id.clone(),
        project_id: run.project_id.clone(),
        story_id: run.story_id.clone(),
        story_number: story.number,
        run_number: run.number,
        repository_id: repository_id.to_owned(),
        repository_path: plan.repository_path,
        base_commit: plan.base_commit,
        scenario: TestScenario::parse(scenario)?,
        timeout_minutes: settings.timeout_minutes,
    }))
}

/// Errores que desaparecen al reintentar: base ocupada o bloqueada (`SQLITE_BUSY`,
/// `SQLITE_LOCKED` y sus códigos extendidos), sin conexión libre en el pool o almacén
/// bloqueado. Cualquier otro se trata como persistente: el intento falla (o su petición se
/// anula) para no bloquear la cola.
fn is_transient(error: &CoreError) -> bool {
    match error {
        CoreError::Database(sqlx::Error::Database(db)) => db
            .code()
            .and_then(|code| code.parse::<i64>().ok())
            .is_some_and(|code| matches!(code & 0xff, 5 | 6)),
        CoreError::Database(sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed) => true,
        CoreError::StorageLocked => true,
        _ => false,
    }
}

async fn reject(
    conn: &mut SqliteConnection,
    run: &RunSummary,
    reason: &str,
    now: i64,
) -> Result<DispatchOutcome, CoreError> {
    transition(conn, run, RunState::Failed, Some(reason), now).await?;
    Ok(DispatchOutcome::Rejected(
        require_run_any(conn, &run.id).await?,
    ))
}

/// Registra el worktree que el supervisor acaba de crear para un intento en `preparing`, en
/// una transacción `BEGIN IMMEDIATE`. Ruta, rama y commit los calcula el núcleo (los mismos
/// que devolvió `begin_dispatch`): el supervisor no puede registrar otra ruta. Repetirlo
/// devuelve el ya registrado.
///
/// Errores: intento inexistente → `not_found`; no está en `preparing` →
/// `invalid_run_transition`; sin petición despachada → `validation`.
pub async fn record_worktree(
    store: &Store,
    token: &SupervisorToken,
    run_id: &str,
) -> Result<RunWorktree, CoreError> {
    token.check(store)?;
    validate_id("runId", run_id)?;
    let (run_id, clock) = (run_id.to_owned(), store.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let run = require_run_any(conn, &run_id).await?;
                if let Some(existing) = load_worktree(conn, &run.id).await? {
                    return Ok(existing);
                }
                if run.state != RunState::Preparing {
                    return Err(CoreError::InvalidRunTransition {
                        from: run.state.label(),
                        to: RunState::Running.label(),
                    });
                }
                let repository_id: String = sqlx::query_scalar(
                    "SELECT repository_id FROM run_start_requests \
                     WHERE run_id = ? AND dispatched_at_ms IS NOT NULL",
                )
                .bind(&run.id)
                .fetch_optional(&mut *conn)
                .await?
                .ok_or_else(|| CoreError::Validation {
                    field: "runId",
                    reason: "El intento no tiene una petición de inicio despachada.".into(),
                })?;
                let story_number: i64 =
                    sqlx::query_scalar("SELECT number FROM stories WHERE id = ?")
                        .bind(&run.story_id)
                        .fetch_one(&mut *conn)
                        .await?;
                // El mismo cálculo que `begin_dispatch`: el supervisor no elige nada.
                let plan = worktree_plan(conn, &clock, &run, story_number, &repository_id)
                    .await?
                    .map_err(|failure| CoreError::Validation {
                        field: "repositoryId",
                        reason: failure.reason().into(),
                    })?;
                let path = plan.worktree_path.to_str().ok_or_else(|| {
                    CoreError::StorageIo(std::io::Error::other("la ruta del worktree no es UTF-8"))
                })?;
                sqlx::query(
                    "INSERT INTO run_worktrees (run_id, repository_id, path, branch, \
                     base_commit, created_at_ms) VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(&run.id)
                .bind(&repository_id)
                .bind(path)
                .bind(&plan.branch)
                .bind(&plan.base_commit)
                .bind(now)
                .execute(&mut *conn)
                .await?;
                load_worktree(conn, &run.id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Worktree" })
            })
        })
        .await
}

/// Borra el registro del worktree de un intento en `preparing` cuando el supervisor sabe que
/// `git worktree add` no llegó a ejecutarse (la rama ya existía, el repositorio o el commit no
/// están, la carpeta ya existía…): así ninguna fila atribuye al intento una rama o carpeta
/// ajenas, y la limpieza nunca las tocará. Sin registro, no hace nada. Una transacción
/// `BEGIN IMMEDIATE`.
///
/// Errores: intento inexistente → `not_found`; no está en `preparing` →
/// `invalid_run_transition`.
pub async fn discard_unused_worktree(
    store: &Store,
    token: &SupervisorToken,
    run_id: &str,
) -> Result<(), CoreError> {
    token.check(store)?;
    validate_id("runId", run_id)?;
    let run_id = run_id.to_owned();
    store
        .write(move |conn| {
            Box::pin(async move {
                let run = require_run_any(conn, &run_id).await?;
                if run.state != RunState::Preparing {
                    return Err(CoreError::InvalidRunTransition {
                        from: run.state.label(),
                        to: RunState::Failed.label(),
                    });
                }
                sqlx::query("DELETE FROM run_worktrees WHERE run_id = ? AND removed_at_ms IS NULL")
                    .bind(&run.id)
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .await
}

/// `preparing → running` tras lanzar el proceso, en una transacción `BEGIN IMMEDIATE`. Exige
/// el worktree registrado. Si el resultado trae `cancelRequestedAt`, el supervisor debe matar
/// el árbol en seguida y llamar a `finish_run` con `RunOutcome::Cancelled`.
///
/// Errores: intento inexistente → `not_found`; sin worktree → `validation`; otro estado →
/// `invalid_run_transition`.
pub async fn mark_running(
    store: &Store,
    token: &SupervisorToken,
    run_id: &str,
) -> Result<RunSummary, CoreError> {
    token.check(store)?;
    validate_id("runId", run_id)?;
    let (run_id, clock) = (run_id.to_owned(), store.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let run = require_run_any(conn, &run_id).await?;
                if run.state == RunState::Preparing && load_worktree(conn, &run.id).await?.is_none()
                {
                    return Err(CoreError::Validation {
                        field: "runId",
                        reason: "El intento no tiene worktree registrado.".into(),
                    });
                }
                transition(conn, &run, RunState::Running, None, now).await?;
                require_run_any(conn, &run.id).await
            })
        })
        .await
}

/// Termina un intento con el resultado del supervisor, en una transacción `BEGIN IMMEDIATE`,
/// con la transición normal de la máquina de estados (ver [`RunOutcome`]). Ninguna salida
/// acepta la historia ni la mueve de columna.
///
/// Errores: intento inexistente → `not_found`; transición no admitida (p. ej. ya terminado o
/// `Exited` desde `preparing`) → `invalid_run_transition`; `Cancelled` sin petición →
/// `validation`.
pub async fn finish_run(
    store: &Store,
    token: &SupervisorToken,
    run_id: &str,
    outcome: RunOutcome,
) -> Result<RunSummary, CoreError> {
    token.check(store)?;
    validate_id("runId", run_id)?;
    let (run_id, clock) = (run_id.to_owned(), store.clone());
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let run = require_run_any(conn, &run_id).await?;
                let (to, reason) = match outcome {
                    RunOutcome::Exited { code: 0 } => {
                        (RunState::Completed, "Terminó con código 0.".to_owned())
                    }
                    RunOutcome::Exited { code } => {
                        (RunState::Failed, format!("Terminó con código {code}."))
                    }
                    RunOutcome::Signaled { signal: Some(n) } => (
                        RunState::Failed,
                        format!("El proceso terminó por una señal del sistema ({n})."),
                    ),
                    RunOutcome::Signaled { signal: None } => (
                        RunState::Failed,
                        "El proceso terminó por una señal del sistema.".to_owned(),
                    ),
                    RunOutcome::TimedOut { minutes } => (
                        RunState::Failed,
                        format!("Superó el tiempo máximo ({minutes} min)."),
                    ),
                    RunOutcome::Cancelled => {
                        let reason: Option<String> =
                            sqlx::query_scalar("SELECT cancel_reason FROM runs WHERE id = ?")
                                .bind(&run.id)
                                .fetch_one(&mut *conn)
                                .await?;
                        let reason = reason.ok_or_else(|| CoreError::Validation {
                            field: "runId",
                            reason: "El intento no tiene una petición de cancelación.".into(),
                        })?;
                        (RunState::Cancelled, reason)
                    }
                    RunOutcome::Failed(failure) => (RunState::Failed, failure.reason().to_owned()),
                };
                transition(conn, &run, to, Some(&reason), now).await?;
                require_run_any(conn, &run.id).await
            })
        })
        .await
}

/// Intentos activos con una petición de cancelación sin confirmar (solo lectura). El supervisor
/// mata su árbol de procesos y llama a `finish_run` con `RunOutcome::Cancelled`.
pub async fn pending_cancellations(
    store: &Store,
    token: &SupervisorToken,
) -> Result<Vec<PendingCancellation>, CoreError> {
    token.check(store)?;
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, project_id, state FROM runs WHERE cancel_requested_at_ms IS NOT NULL \
         AND state IN ('preparing', 'running', 'waiting_approval') \
         ORDER BY cancel_requested_at_ms",
    )
    .fetch_all(store.reader())
    .await?;
    rows.into_iter()
        .map(|(run_id, project_id, state)| {
            Ok(PendingCancellation {
                run_id,
                project_id,
                state: RunState::parse(&state)?,
            })
        })
        .collect()
}

/// Marca `interrupted` todo intento en `preparing`, `running` o `waiting_approval` (no
/// verificable: no hay proceso que el supervisor actual conozca), en una sola transacción
/// `BEGIN IMMEDIATE` y con la transición normal. Los `queued` no se tocan. Nunca relanza nada.
///
/// El host la llama con `AppRestarted` al abrir el almacén, **antes** de arrancar el supervisor,
/// y el supervisor con `AppExiting` al salir, tras matar los árboles. Devuelve los intentos
/// cambiados (para el aviso `run-change`).
pub async fn interrupt_unverifiable_runs(
    store: &Store,
    token: &SupervisorToken,
    cause: InterruptCause,
) -> Result<Vec<RunSummary>, CoreError> {
    token.check(store)?;
    let clock = store.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let ids: Vec<String> = sqlx::query_scalar(
                    "SELECT id FROM runs \
                     WHERE state IN ('preparing', 'running', 'waiting_approval')",
                )
                .fetch_all(&mut *conn)
                .await?;
                let mut changed = Vec::with_capacity(ids.len());
                for id in &ids {
                    let run = require_run_any(conn, id).await?;
                    transition(conn, &run, RunState::Interrupted, Some(cause.reason()), now)
                        .await?;
                    changed.push(require_run_any(conn, id).await?);
                }
                Ok(changed)
            })
        })
        .await
}

/// Anula todas las peticiones de inicio sin despachar, en una transacción `BEGIN IMMEDIATE`:
/// sus intentos siguen `queued`, sin petición, y solo arrancan si una persona vuelve a pedirlo
/// en esta sesión. El host la llama al abrir el almacén, antes de arrancar el supervisor.
/// Devuelve los intentos en cola afectados (para el aviso `run-change`).
pub async fn void_start_requests(
    store: &Store,
    token: &SupervisorToken,
) -> Result<Vec<RunSummary>, CoreError> {
    token.check(store)?;
    let clock = store.clone();
    store
        .write(move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let ids: Vec<String> = sqlx::query_scalar(
                    "SELECT q.run_id FROM run_start_requests q JOIN runs r ON r.id = q.run_id \
                     WHERE q.voided_at_ms IS NULL AND q.dispatched_at_ms IS NULL \
                     AND r.state = 'queued'",
                )
                .fetch_all(&mut *conn)
                .await?;
                sqlx::query(
                    "UPDATE run_start_requests SET voided_at_ms = ? \
                     WHERE voided_at_ms IS NULL AND dispatched_at_ms IS NULL",
                )
                .bind(now)
                .execute(&mut *conn)
                .await?;
                let mut changed = Vec::with_capacity(ids.len());
                for id in &ids {
                    changed.push(require_run_any(conn, id).await?);
                }
                Ok(changed)
            })
        })
        .await
}

pub(super) fn ensure_token(store: &Store, token: &SupervisorToken) -> Result<(), CoreError> {
    token.check(store)
}
