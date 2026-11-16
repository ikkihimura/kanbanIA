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

use super::approval::close_orphaned_approvals;
use super::run::{RunState, RunSummary, require_run_any, transition, validate_id};
use super::runnable::ensure_runnable;
use super::settings::load_settings;
use super::snapshot::agent_instructions;
use super::start::{AcpAgent, RunExecutor, TestScenario, load_snapshot};
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

/// Ejecutor de un intento despachado, con lo que necesita para lanzarse sin releer nada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchExecutor {
    /// Proceso de prueba incluido (sin IA) con su escenario.
    TestProcess(TestScenario),
    /// Un agente real vía ACP (HU-019: Claude Code; HU-021: Codex).
    Acp {
        agent: AcpAgent,
        /// Ruta del adaptador leída de la configuración al despachar, tal como la escribió la
        /// persona (se validó en disco al guardarla; puede haber cambiado después: si no se
        /// puede lanzar → `RunFailure::AdapterMissing`).
        adapter_path: PathBuf,
        /// Instrucciones Markdown de la instantánea, generadas con el mismo código que
        /// `RunSnapshot.instructions` pero sin rutas absolutas de repositorios
        /// (`snapshot::agent_instructions`): el único texto del prompt. El contenido de la
        /// historia ya va delimitado como datos. La ruta del worktree es el `cwd`.
        instructions: String,
    },
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
    /// Ejecutor elegido al pedir el inicio.
    pub executor: DispatchExecutor,
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
    /// No se pudo lanzar el proceso (de prueba). Para el adaptador ACP: `AdapterMissing` o
    /// `AdapterSpawnFailed`.
    SpawnFailed,
    /// Error del supervisor (p. ej. al escribir la salida).
    Internal,
    /// ACP: el proceso terminó o cerró su salida antes de responder.
    AgentDisconnected,
    /// ACP: JSON o JSON-RPC no válido, o una respuesta con forma inesperada (incluido
    /// `stopReason: cancelled` sin que la app pidiera cancelar).
    AgentProtocolError,
    /// ACP: `initialize` respondió con una versión de protocolo distinta de la 1.
    AgentVersionUnsupported,
    /// ACP: el agente exige autenticarse (la app nunca autentica por su cuenta).
    AgentAuthRequired(AcpAgent),
    /// ACP: no hay un modo que pida permiso antes de actuar, o no se pudo fijar.
    AgentModeUnavailable,
    /// ACP: `stopReason: refusal`.
    AgentRefused,
    /// ACP: `stopReason: max_tokens` o `max_turn_requests`.
    AgentLimitReached,
    /// ACP: error JSON-RPC a una petición de la app durante la negociación (el mensaje del
    /// agente va recortado al registro, nunca al motivo).
    AgentRequestFailed,
    /// ACP: error JSON-RPC a `session/prompt` (el turno no terminó).
    AgentTurnFailed,
    /// ACP: el adaptador configurado no existe o no se puede ejecutar (o no hay ruta).
    AdapterMissing(AcpAgent),
    /// ACP: el adaptador existe pero no se pudo lanzar por otro motivo (el detalle va al
    /// registro del intento).
    AdapterSpawnFailed(AcpAgent),
    /// ACP (Codex, HU-021): el worktree trae configuración del agente (`.codex/`), que se
    /// cargaría al abrir la sesión (servidores MCP, reglas que ejecutan sin preguntar): no se
    /// lanza.
    AgentConfigInRepository(AcpAgent),
    /// ACP (Codex, HU-021): la carpeta de Codex de la app (`CODEX_HOME`) contiene configuración
    /// (`config.toml`, reglas, `AGENTS.md`…) que Codex cargaría: no se lanza.
    AgentConfigInCodexHome,
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
            Self::SpawnFailed => "No se pudo lanzar el proceso.",
            Self::AdapterSpawnFailed(AcpAgent::ClaudeCode) => {
                "No se pudo lanzar el adaptador de Claude Code configurado. Consulta el registro \
                 del intento."
            }
            Self::AdapterSpawnFailed(AcpAgent::Codex) => {
                "No se pudo lanzar el adaptador de Codex configurado. Consulta el registro del \
                 intento."
            }
            Self::Internal => {
                "El supervisor detuvo el intento por un error interno. Consulta el registro del \
                 intento."
            }
            Self::AgentDisconnected => "El agente se desconectó antes de terminar.",
            Self::AgentProtocolError => "El agente envió una respuesta ACP no válida.",
            Self::AgentVersionUnsupported => {
                "El agente no habla una versión de ACP compatible (se requiere la 1)."
            }
            Self::AgentAuthRequired(AcpAgent::ClaudeCode) => {
                "El agente necesita iniciar sesión: abre Claude Code en una terminal y usa /login."
            }
            Self::AgentAuthRequired(AcpAgent::Codex) => {
                "Codex necesita iniciar sesión en la carpeta de Codex de la app: usa el comando \
                 que muestra Sistema → Ejecuciones."
            }
            Self::AgentModeUnavailable => {
                "El agente no permite fijar un modo que pida permiso antes de actuar: no se \
                 ejecuta."
            }
            Self::AgentRefused => "El agente rechazó la tarea.",
            Self::AgentLimitReached => "El agente alcanzó su límite de tokens o de turnos.",
            Self::AgentRequestFailed => {
                "El agente devolvió un error al negociar la sesión. Consulta el registro del \
                 intento."
            }
            Self::AgentTurnFailed => {
                "El agente devolvió un error durante el turno. Consulta el registro del intento."
            }
            Self::AdapterMissing(AcpAgent::ClaudeCode) => {
                "No se encontró el adaptador de Claude Code configurado o no es ejecutable."
            }
            Self::AdapterMissing(AcpAgent::Codex) => {
                "No se encontró el adaptador de Codex configurado o no es ejecutable."
            }
            Self::AgentConfigInRepository(AcpAgent::Codex) => {
                "El repositorio contiene configuración de Codex (.codex/), que podría saltarse los \
                 permisos o lanzar programas: la app no ejecuta Codex en él."
            }
            Self::AgentConfigInCodexHome => {
                "La carpeta de Codex de la app contiene configuración (config.toml, reglas, \
                 AGENTS.md…) que podría saltarse los permisos: bórrala o vuelve a crear la \
                 carpeta e inicia sesión de nuevo."
            }
            Self::AgentConfigInRepository(AcpAgent::ClaudeCode) => {
                "El repositorio contiene configuración del agente que podría saltarse los \
                 permisos: la app no lo ejecuta en él."
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
    /// ACP: el agente respondió `stopReason: end_turn` → `completed` («El agente terminó su
    /// turno.»). No acepta la historia ni la mueve; es lo que el agente reporta, no una
    /// verificación. Después el supervisor cierra el adaptador y mata su árbol: su código de
    /// salida ya no dice nada del resultado.
    AgentEndedTurn,
    /// ACP (Codex, HU-021): el agente detuvo el turno él mismo justo después de que la app
    /// rechazara un permiso (por decisión de una persona o por caducidad); Codex no tiene
    /// «rechazar y seguir» para las ediciones → `completed` («El agente detuvo su turno tras un
    /// permiso rechazado.»). Igual que `AgentEndedTurn`, no acepta ni mueve la historia.
    AgentStoppedAfterRejection,
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
                let request: Option<(i64, String, String, Option<String>)> = sqlx::query_as(
                    "SELECT id, repository_id, executor, scenario FROM run_start_requests \
                     WHERE run_id = ? AND voided_at_ms IS NULL AND dispatched_at_ms IS NULL",
                )
                .bind(&run_id)
                .fetch_optional(&mut *conn)
                .await?;
                let Some((request_id, repository_id, executor, scenario)) = request else {
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
                let executor = (executor.as_str(), scenario.as_deref());
                match prepare(conn, &clock, &run, &repository_id, executor, now).await {
                    Ok(Prepared::Ready(dispatch)) => {
                        transition(conn, &run, RunState::Preparing, None, now).await?;
                        Ok(DispatchOutcome::Ready(*dispatch))
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
    Ready(Box<Dispatch>),
    Rejected(String),
}

/// Reevalúa `ensure_runnable` y calcula el destino del worktree.
async fn prepare(
    conn: &mut SqliteConnection,
    store: &Store,
    run: &RunSummary,
    repository_id: &str,
    (executor, scenario): (&str, Option<&str>),
    now: i64,
) -> Result<Prepared, CoreError> {
    let executor = RunExecutor::from_columns(executor, scenario)?;
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
    let executor = match executor {
        RunExecutor::TestProcess { scenario } => DispatchExecutor::TestProcess(scenario),
        RunExecutor::ClaudeAcp | RunExecutor::CodexAcp => {
            let Some(agent) = executor.acp_agent() else {
                return Err(CoreError::Database(sqlx::Error::Decode(
                    "ejecutor ACP sin agente".into(),
                )));
            };
            // La persona pudo quitar la ruta después de pedir el inicio.
            let Some(path) = settings.adapter_path(agent) else {
                return Ok(Prepared::Rejected(
                    RunFailure::AdapterMissing(agent).reason().to_owned(),
                ));
            };
            DispatchExecutor::Acp {
                agent,
                adapter_path: PathBuf::from(path),
                instructions: agent_instructions(
                    &load_snapshot(conn, &run.id).await?,
                    repository_id,
                ),
            }
        }
    };
    Ok(Prepared::Ready(Box::new(Dispatch {
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
        executor,
        timeout_minutes: settings.timeout_minutes,
    })))
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
                    RunOutcome::AgentEndedTurn => (
                        RunState::Completed,
                        "El agente terminó su turno.".to_owned(),
                    ),
                    RunOutcome::AgentStoppedAfterRejection => (
                        RunState::Completed,
                        "El agente detuvo su turno tras un permiso rechazado.".to_owned(),
                    ),
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
                // Peticiones de permiso abiertas de intentos ya terminados (p. ej. de una copia
                // restaurada, que se interrumpe antes de migrar).
                close_orphaned_approvals(conn, now).await?;
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
