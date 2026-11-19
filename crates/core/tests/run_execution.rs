//! Ejecución de intentos (HU-018): configuración, petición de inicio, despacho, worktrees,
//! cancelación por petición e interrupción al arrancar. SQLite temporal real y reloj
//! controlado; el supervisor lo hace la prueba con su `SupervisorToken`.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kanbania_core::error::{CoreError, ErrorCode};
use kanbania_core::features::access::{
    CreateAgentCredentialInput, authenticate_agent, create_agent_credential,
};
use kanbania_core::features::claims::{ClaimStoryInput, claim_story};
use kanbania_core::features::columns::{Column, ColumnCategory, list_columns};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::relations::{SetStoryAgentInput, set_story_agent};
use kanbania_core::features::repositories::{
    LinkRepositoryInput, UnlinkRepositoryInput, link_repository, list_repositories,
    unlink_repository,
};
use kanbania_core::features::review::{SetBlockerInput, set_story_blocker};
use kanbania_core::features::runs::{
    CancelRunInput, CreateRunInput, DispatchExecutor, DispatchOutcome, InterruptCause,
    RecordRunEventInput, RecordWorktreeRemovalInput, RunEvent, RunExecutor, RunFailure, RunOutcome,
    RunState, RunSummary, StartRunInput, SupervisorToken, TestScenario, UpdateRunSettingsInput,
    WORKTREES_DIR, WorktreeRemoval, begin_dispatch, cancel_run, create_run,
    discard_unused_worktree, finish_run, get_run, get_run_settings, get_worktree_target,
    interrupt_unverifiable_runs, mark_running, next_dispatchable, pending_cancellations,
    record_run_event, record_worktree, record_worktree_removal, start_run, update_run_settings,
    void_start_requests,
};
use kanbania_core::features::stories::{
    ArchiveStoryInput, CreateStoryInput, Priority, StorySummary, archive_story, create_story,
    get_story,
};
use kanbania_core::features::team::{
    Agent, AgentFields, AvatarSpec, Capability, CreateAgentInput, create_agent,
};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::clock::ManualClock;
use kanbania_core::platform::storage::Store;

const UI: &str = "ui";
const START_MS: i64 = 1_794_744_000_000;
const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn code<T: std::fmt::Debug>(result: Result<T, CoreError>) -> ErrorCode {
    result.unwrap_err().code()
}

fn field<T: std::fmt::Debug>(result: Result<T, CoreError>) -> String {
    result
        .unwrap_err()
        .to_payload(None)
        .field
        .unwrap_or_default()
}

fn agent_actor() -> Actor {
    Actor::Agent {
        agent_id: "agente".into(),
    }
}

struct Board {
    data: tempfile::TempDir,
    repos: tempfile::TempDir,
    store: Store,
    clock: Arc<ManualClock>,
    project: Project,
    columns: Vec<Column>,
    agent: Agent,
    token: SupervisorToken,
}

impl Board {
    /// Tablero con un repositorio vinculado cuyo HEAD es `SHA_A` (rama `main`).
    async fn new() -> Self {
        let b = Self::bare().await;
        let repo = git_repo(b.repos.path(), "repo con espacios", "main", SHA_A);
        b.link(&repo).await;
        b
    }

    /// Tablero sin repositorios.
    async fn bare() -> Self {
        let data = tempfile::tempdir().unwrap();
        let repos = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(START_MS));
        let store = Store::open(data.path())
            .await
            .unwrap()
            .with_clock(clock.clone());
        let project = create_project(
            &store,
            UI,
            &CreateProjectInput {
                request_id: rid(),
                name: "Proyecto".into(),
            },
        )
        .await
        .unwrap();
        let columns = list_columns(&store, &project.id).await.unwrap();
        let agent = agent(&store, "Constructor").await;
        let token = SupervisorToken::claim(&store).unwrap();
        Self {
            data,
            repos,
            store,
            clock,
            project,
            columns,
            agent,
            token,
        }
    }

    async fn link(&self, path: &Path) -> String {
        link_repository(
            &self.store,
            UI,
            &LinkRepositoryInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                path: path.to_str().unwrap().into(),
            },
        )
        .await
        .unwrap()
        .id
    }

    fn column(&self, category: ColumnCategory) -> String {
        self.columns
            .iter()
            .find(|c| c.category == category)
            .unwrap()
            .id
            .clone()
    }

    /// Historia lista con el agente asignado.
    async fn assigned(&self, title: &str) -> StorySummary {
        let s = create_story(
            &self.store,
            UI,
            &CreateStoryInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                title: title.into(),
                column_id: Some(self.column(ColumnCategory::Ready)),
                priority: Some(Priority::High),
            },
        )
        .await
        .unwrap();
        set_story_agent(
            &self.store,
            &SetStoryAgentInput {
                project_id: self.project.id.clone(),
                story_id: s.id.clone(),
                agent_id: Some(self.agent.id.clone()),
            },
        )
        .await
        .unwrap();
        s
    }

    async fn run(&self, s: &StorySummary) -> RunSummary {
        create_run(
            &self.store,
            &Actor::Human,
            &CreateRunInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                story_id: s.id.clone(),
                agent_id: None,
                note: None,
            },
        )
        .await
        .unwrap()
    }

    fn start_input(&self, run: &RunSummary, scenario: TestScenario) -> StartRunInput {
        StartRunInput {
            request_id: rid(),
            project_id: self.project.id.clone(),
            run_id: run.id.clone(),
            repository_id: None,
            executor: RunExecutor::TestProcess { scenario },
        }
    }

    async fn start(&self, run: &RunSummary) -> RunSummary {
        start_run(
            &self.store,
            &Actor::Human,
            &self.start_input(run, TestScenario::Success),
        )
        .await
        .unwrap()
    }

    async fn cancel(&self, run: &RunSummary) -> Result<RunSummary, CoreError> {
        cancel_run(
            &self.store,
            &Actor::Human,
            &CancelRunInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                run_id: run.id.clone(),
            },
        )
        .await
    }

    async fn dispatch(&self, run: &RunSummary) -> DispatchOutcome {
        begin_dispatch(&self.store, &self.token, &run.id)
            .await
            .unwrap()
    }

    /// Intento iniciado, despachado, con worktree y en `running`.
    async fn running(&self, title: &str) -> RunSummary {
        let s = self.assigned(title).await;
        let run = self.run(&s).await;
        self.start(&run).await;
        assert!(matches!(
            self.dispatch(&run).await,
            DispatchOutcome::Ready(_)
        ));
        record_worktree(&self.store, &self.token, &run.id)
            .await
            .unwrap();
        mark_running(&self.store, &self.token, &run.id)
            .await
            .unwrap()
    }

    async fn reload(&self, run: &RunSummary) -> RunSummary {
        get_run(&self.store, &self.project.id, &run.id)
            .await
            .unwrap()
            .run
    }
}

async fn agent(store: &Store, name: &str) -> Agent {
    create_agent(
        store,
        UI,
        &CreateAgentInput {
            request_id: rid(),
            fields: AgentFields {
                name: name.into(),
                role: "Implementa".into(),
                provider: "Proveedor".into(),
                model: "modelo-1".into(),
                capabilities: vec![Capability::Implement],
                avatar: AvatarSpec::DEFAULT_AGENT,
            },
        },
    )
    .await
    .unwrap()
}

/// Repositorio falso: solo los archivos de `.git` que lee el núcleo para el HEAD.
fn git_repo(parent: &Path, name: &str, branch: &str, sha: &str) -> PathBuf {
    let root = parent.join(name);
    std::fs::create_dir_all(root.join(".git").join("refs").join("heads")).unwrap();
    std::fs::write(
        root.join(".git/HEAD"),
        format!("ref: refs/heads/{branch}\n"),
    )
    .unwrap();
    std::fs::write(
        root.join(".git/refs/heads").join(branch),
        format!("{sha}\n"),
    )
    .unwrap();
    root
}

#[tokio::test]
async fn start_dispatch_and_finish_follow_the_state_machine() {
    let b = Board::new().await;
    let s = b.assigned("Buscador").await;
    let story_before = get_story(&b.store, &b.project.id, &s.id).await.unwrap();
    let run = b.run(&s).await;
    assert!(run.start_requested_at.is_none());
    assert!(
        next_dispatchable(&b.store, &b.token)
            .await
            .unwrap()
            .is_none()
    );

    // Iniciar registra la petición; el intento sigue en cola.
    let started = b.start(&run).await;
    assert_eq!(started.state, RunState::Queued);
    assert!(started.start_requested_at.is_some());
    assert_eq!(started.last_sequence, 1);
    let pending = next_dispatchable(&b.store, &b.token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.run_id, run.id);
    assert_eq!(pending.project_id, b.project.id);

    // Despachar: `preparing` y todo lo necesario para el worktree.
    let DispatchOutcome::Ready(dispatch) = b.dispatch(&run).await else {
        panic!("debía despacharse");
    };
    let tail: String = run.id.chars().filter(char::is_ascii_hexdigit).collect();
    let tail = &tail[tail.len() - 8..];
    assert_eq!(
        dispatch.branch,
        format!("kanbania/h{}-i{}-{tail}", s.number, run.number)
    );
    assert_eq!(dispatch.base_commit, SHA_A);
    assert_eq!(
        dispatch.executor,
        DispatchExecutor::TestProcess(TestScenario::Success)
    );
    assert_eq!(dispatch.timeout_minutes, 30);
    assert!(dispatch.repository_path.ends_with("repo con espacios"));
    assert_eq!(
        dispatch.worktree_path,
        b.data
            .path()
            .join(WORKTREES_DIR)
            .join(&b.project.id)
            .join(&run.id)
    );
    let preparing = b.reload(&run).await;
    assert_eq!(preparing.state, RunState::Preparing);
    assert!(preparing.start_requested_at.is_none());
    assert!(
        next_dispatchable(&b.store, &b.token)
            .await
            .unwrap()
            .is_none()
    );
    // Despachar otra vez: nada que hacer.
    assert_eq!(b.dispatch(&run).await, DispatchOutcome::Stale);

    // Sin worktree registrado no pasa a `running`; un proceso no puede «terminar» en preparación.
    assert_eq!(
        field(mark_running(&b.store, &b.token, &run.id).await),
        "runId"
    );
    assert_eq!(
        code(finish_run(&b.store, &b.token, &run.id, RunOutcome::Exited { code: 0 }).await),
        ErrorCode::InvalidRunTransition
    );
    let worktree = record_worktree(&b.store, &b.token, &run.id).await.unwrap();
    assert_eq!(worktree.branch, dispatch.branch);
    assert_eq!(worktree.base_commit, SHA_A);
    assert_eq!(
        PathBuf::from(&worktree.path),
        dispatch.worktree_path,
        "el núcleo registra la ruta que calculó"
    );
    // Repetirlo devuelve el mismo.
    assert_eq!(
        record_worktree(&b.store, &b.token, &run.id).await.unwrap(),
        worktree
    );
    let running = mark_running(&b.store, &b.token, &run.id).await.unwrap();
    assert_eq!(running.state, RunState::Running);

    let done = finish_run(&b.store, &b.token, &run.id, RunOutcome::Exited { code: 0 })
        .await
        .unwrap();
    assert_eq!(done.state, RunState::Completed);
    assert_eq!(done.end_reason.as_deref(), Some("Terminó con código 0."));
    assert_eq!(done.last_sequence, 4);
    // Completado ≠ aceptado: la historia no cambia.
    let story_after = get_story(&b.store, &b.project.id, &s.id).await.unwrap();
    assert_eq!(story_after.revision, story_before.revision);
    assert_eq!(story_after.column_id, story_before.column_id);
    assert!(story_after.accepted_at.is_none());
    // Terminado: nada más.
    assert_eq!(
        code(finish_run(&b.store, &b.token, &run.id, RunOutcome::Exited { code: 1 }).await),
        ErrorCode::InvalidRunTransition
    );

    let detail = get_run(&b.store, &b.project.id, &run.id).await.unwrap();
    let request = detail.start_request.unwrap();
    assert_eq!(
        request.executor,
        RunExecutor::TestProcess {
            scenario: TestScenario::Success
        }
    );
    assert!(request.dispatched_at.is_some());
    assert!(request.voided_at.is_none());
    assert_eq!(detail.worktree.unwrap(), worktree);
    let states: Vec<RunState> = detail
        .events
        .iter()
        .filter_map(|e| match &e.event {
            RunEvent::StateChanged { to, .. } => Some(*to),
            _ => None,
        })
        .collect();
    assert_eq!(
        states,
        [
            RunState::Queued,
            RunState::Preparing,
            RunState::Running,
            RunState::Completed
        ]
    );
}

#[tokio::test]
async fn process_outcomes_produce_safe_core_reasons() {
    let b = Board::new().await;
    let cases = [
        (RunOutcome::Exited { code: 3 }, "Terminó con código 3."),
        (
            RunOutcome::TimedOut { minutes: 30 },
            "Superó el tiempo máximo (30 min).",
        ),
        (
            RunOutcome::Signaled { signal: Some(9) },
            "El proceso terminó por una señal del sistema (9).",
        ),
        (
            RunOutcome::Failed(RunFailure::SpawnFailed),
            "No se pudo lanzar el proceso.",
        ),
    ];
    for (i, (outcome, reason)) in cases.into_iter().enumerate() {
        let run = b.running(&format!("Caso {i}")).await;
        let done = finish_run(&b.store, &b.token, &run.id, outcome)
            .await
            .unwrap();
        assert_eq!(done.state, RunState::Failed);
        assert_eq!(done.end_reason.as_deref(), Some(reason));
    }

    // Fallo al preparar (antes del proceso): `preparing → failed`, sin rutas en el motivo.
    let s = b.assigned("Rama existente").await;
    let run = b.run(&s).await;
    b.start(&run).await;
    b.dispatch(&run).await;
    let failed = finish_run(
        &b.store,
        &b.token,
        &run.id,
        RunOutcome::Failed(RunFailure::BranchExists),
    )
    .await
    .unwrap();
    assert_eq!(failed.state, RunState::Failed);
    let reason = failed.end_reason.unwrap();
    assert!(reason.contains("rama"), "{reason}");
    assert!(!reason.contains('/'), "{reason}");
}

#[tokio::test]
async fn start_requires_a_frozen_repository_commit_and_is_idempotent() {
    // Sin repositorios: no hay referencia explícita.
    let b = Board::bare().await;
    let s = b.assigned("Sin repositorio").await;
    let run = b.run(&s).await;
    let error = start_run(
        &b.store,
        &Actor::Human,
        &b.start_input(&run, TestScenario::Success),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Validation);
    assert_eq!(
        error.to_payload(None).field.as_deref(),
        Some("repositoryId")
    );
    assert!(
        error.to_string().contains("vincula un repositorio"),
        "{error}"
    );
    b.cancel(&run).await.unwrap();

    // Dos repositorios con commit y uno sin él: hay que elegir uno con commit.
    let first = git_repo(b.repos.path(), "uno", "main", SHA_A);
    let second = git_repo(b.repos.path(), "dos", "dev", SHA_B);
    // Repositorio sin commits: HEAD apunta a una rama que aún no existe.
    let empty = b.repos.path().join("vacio");
    std::fs::create_dir_all(empty.join(".git").join("refs").join("heads")).unwrap();
    std::fs::write(empty.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    b.link(&first).await;
    let second_id = b.link(&second).await;
    let empty_id = b.link(&empty).await;
    let run = b.run(&s).await;
    let input = b.start_input(&run, TestScenario::Write);
    assert_eq!(
        field(start_run(&b.store, &Actor::Human, &input).await),
        "repositoryId"
    );
    for repository in [empty_id.as_str(), "desconocido"] {
        let input = StartRunInput {
            repository_id: Some(repository.into()),
            ..b.start_input(&run, TestScenario::Write)
        };
        assert_eq!(
            field(start_run(&b.store, &Actor::Human, &input).await),
            "repositoryId"
        );
    }

    // Solo personas.
    let chosen = StartRunInput {
        repository_id: Some(second_id.clone()),
        ..b.start_input(&run, TestScenario::Write)
    };
    assert_eq!(
        code(start_run(&b.store, &agent_actor(), &chosen).await),
        ErrorCode::Forbidden
    );

    // Reintentar con el mismo `requestId` devuelve lo mismo y no duplica la petición.
    let started = start_run(&b.store, &Actor::Human, &chosen).await.unwrap();
    let again = start_run(&b.store, &Actor::Human, &chosen).await.unwrap();
    assert_eq!(again, started);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_start_requests WHERE run_id = ?")
        .bind(&run.id)
        .fetch_one(b.store.reader())
        .await
        .unwrap();
    assert_eq!(count, 1);
    // Mismo `requestId` con otro escenario: conflicto de idempotencia.
    let other = StartRunInput {
        executor: RunExecutor::TestProcess {
            scenario: TestScenario::Fail,
        },
        ..chosen.clone()
    };
    assert_eq!(
        code(start_run(&b.store, &Actor::Human, &other).await),
        ErrorCode::IdempotencyConflict
    );
    // Otra petición (nuevo `requestId`) con una pendiente: validación.
    assert_eq!(
        field(
            start_run(
                &b.store,
                &Actor::Human,
                &b.start_input(&run, TestScenario::Write)
            )
            .await
        ),
        "runId"
    );

    let DispatchOutcome::Ready(dispatch) = b.dispatch(&run).await else {
        panic!("debía despacharse");
    };
    assert_eq!(dispatch.repository_id, second_id);
    assert_eq!(dispatch.base_commit, SHA_B);
    assert_eq!(
        dispatch.executor,
        DispatchExecutor::TestProcess(TestScenario::Write)
    );
    // Ya no está en cola: iniciar es una transición no válida.
    assert_eq!(
        code(
            start_run(
                &b.store,
                &Actor::Human,
                &b.start_input(&run, TestScenario::Write)
            )
            .await
        ),
        ErrorCode::InvalidRunTransition
    );
    // Intento de otro proyecto: no encontrado.
    let foreign = StartRunInput {
        project_id: "otro".into(),
        ..b.start_input(&run, TestScenario::Write)
    };
    assert_eq!(
        code(start_run(&b.store, &Actor::Human, &foreign).await),
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn dispatch_reevaluates_runnable_and_fails_with_the_reason() {
    let b = Board::new().await;
    // Bloqueada mientras esperaba hueco.
    let blocked = b.assigned("Se bloquea").await;
    let run = b.run(&blocked).await;
    b.start(&run).await;
    set_story_blocker(
        &b.store,
        &Actor::Human,
        &SetBlockerInput {
            project_id: b.project.id.clone(),
            story_id: blocked.id.clone(),
            reason: Some("Falta acceso".into()),
        },
    )
    .await
    .unwrap();
    let DispatchOutcome::Rejected(failed) = b.dispatch(&run).await else {
        panic!("debía rechazarse");
    };
    assert_eq!(failed.state, RunState::Failed);
    let reason = failed.end_reason.unwrap();
    assert!(reason.starts_with("No se pudo iniciar"), "{reason}");
    assert!(reason.contains("bloqueada"), "{reason}");
    assert!(
        next_dispatchable(&b.store, &b.token)
            .await
            .unwrap()
            .is_none()
    );

    // Otro agente reclamó la historia mientras esperaba hueco.
    let claimed = b.assigned("Se reclama").await;
    let run = b.run(&claimed).await;
    b.start(&run).await;
    let other = agent(&b.store, "Otro").await;
    set_story_agent(
        &b.store,
        &SetStoryAgentInput {
            project_id: b.project.id.clone(),
            story_id: claimed.id.clone(),
            agent_id: None,
        },
    )
    .await
    .unwrap();
    let credential = create_agent_credential(
        &b.store,
        &Actor::Human,
        &CreateAgentCredentialInput {
            project_id: b.project.id.clone(),
            agent_id: other.id.clone(),
        },
    )
    .await
    .unwrap();
    let session = authenticate_agent(&b.store, &credential.token)
        .await
        .unwrap();
    claim_story(
        &b.store,
        &session,
        &ClaimStoryInput {
            request_id: rid(),
            story_id: Some(claimed.id.clone()),
            number: None,
        },
    )
    .await
    .unwrap();
    let DispatchOutcome::Rejected(failed) = b.dispatch(&run).await else {
        panic!("debía rechazarse");
    };
    assert_eq!(failed.state, RunState::Failed);
    assert!(
        failed.end_reason.unwrap().contains("reclamo vigente"),
        "motivo del reclamo"
    );
}

/// Decisión del propietario: la instantánea no basta; el repositorio debe seguir vinculado al
/// iniciar, al despachar y al registrar el worktree.
#[tokio::test]
async fn an_unlinked_repository_cannot_be_started_dispatched_or_recorded() {
    let b = Board::new().await;
    let unlink = || async {
        let links = list_repositories(&b.store, &b.project.id).await.unwrap();
        for link in links {
            unlink_repository(
                &b.store,
                &UnlinkRepositoryInput {
                    project_id: b.project.id.clone(),
                    id: link.id.clone(),
                    expected_revision: link.revision,
                },
            )
            .await
            .unwrap();
        }
    };

    // Preparados con el repositorio vinculado (instantánea con commit).
    let before_start = b.run(&b.assigned("Antes de iniciar").await).await;
    let before_dispatch = b.run(&b.assigned("Antes de despachar").await).await;
    let before_record = b.run(&b.assigned("Antes de registrar").await).await;
    b.start(&before_dispatch).await;
    b.start(&before_record).await;
    assert!(matches!(
        b.dispatch(&before_record).await,
        DispatchOutcome::Ready(_)
    ));
    unlink().await;

    // Iniciar → `validation` en `repositoryId`, sin petición.
    let result = start_run(
        &b.store,
        &Actor::Human,
        &b.start_input(&before_start, TestScenario::Success),
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(error.code(), ErrorCode::Validation);
    let payload = error.to_payload(None);
    assert_eq!(payload.field.as_deref(), Some("repositoryId"));
    assert!(
        payload.message.contains("ya no está vinculado"),
        "{}",
        payload.message
    );
    assert!(b.reload(&before_start).await.start_requested_at.is_none());

    // Despachar → `failed` con el motivo, sin bloquear la cola.
    let DispatchOutcome::Rejected(failed) = b.dispatch(&before_dispatch).await else {
        panic!("debía rechazarse");
    };
    assert_eq!(failed.state, RunState::Failed);
    assert!(
        failed.end_reason.unwrap().contains("ya no está vinculado"),
        "motivo del desvínculo"
    );
    assert!(
        next_dispatchable(&b.store, &b.token)
            .await
            .unwrap()
            .is_none()
    );

    // Registrar el worktree (último paso antes del disco) → `validation`, sin fila.
    assert_eq!(
        field(record_worktree(&b.store, &b.token, &before_record.id).await),
        "repositoryId"
    );
    assert!(
        get_run(&b.store, &b.project.id, &before_record.id)
            .await
            .unwrap()
            .worktree
            .is_none()
    );
}

#[tokio::test]
async fn dispatch_order_is_fifo_and_cancelled_requests_are_stale() {
    let b = Board::new().await;
    let first = b.run(&b.assigned("Primera").await).await;
    let second = b.run(&b.assigned("Segunda").await).await;
    let third = b.run(&b.assigned("Tercera").await).await;
    // Se piden en otro orden que el de creación.
    b.start(&second).await;
    b.clock.advance(Duration::from_secs(1));
    b.start(&first).await;
    b.clock.advance(Duration::from_secs(1));
    b.start(&third).await;
    let next = next_dispatchable(&b.store, &b.token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.run_id, second.id);

    // En cola, cancelar es directo (sin proceso) y saca el intento de la cola.
    let cancelled = b.cancel(&second).await.unwrap();
    assert_eq!(cancelled.state, RunState::Cancelled);
    assert!(cancelled.cancel_requested_at.is_none());
    assert!(cancelled.start_requested_at.is_none());
    assert_eq!(b.dispatch(&second).await, DispatchOutcome::Stale);
    let next = next_dispatchable(&b.store, &b.token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.run_id, first.id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_dispatches_of_the_same_run_prepare_it_once() {
    let b = Board::new().await;
    let run = b.run(&b.assigned("Carrera").await).await;
    b.start(&run).await;
    let (x, y) = tokio::join!(
        begin_dispatch(&b.store, &b.token, &run.id),
        begin_dispatch(&b.store, &b.token, &run.id),
    );
    let outcomes = [x.unwrap(), y.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| matches!(o, DispatchOutcome::Ready(_)))
            .count(),
        1
    );
    assert!(outcomes.contains(&DispatchOutcome::Stale));
    assert_eq!(b.reload(&run).await.last_sequence, 2);
}

#[tokio::test]
async fn cancelling_a_run_with_a_process_is_a_request_the_supervisor_confirms() {
    let b = Board::new().await;
    let run = b.running("Larga").await;
    assert!(
        pending_cancellations(&b.store, &b.token)
            .await
            .unwrap()
            .is_empty()
    );
    // Sin petición, el supervisor no puede registrar `cancelled`.
    assert_eq!(
        field(finish_run(&b.store, &b.token, &run.id, RunOutcome::Cancelled).await),
        "runId"
    );

    let requested = b.cancel(&run).await.unwrap();
    assert_eq!(requested.state, RunState::Running);
    assert_eq!(requested.last_sequence, run.last_sequence);
    let first_request = requested.cancel_requested_at.clone().unwrap();
    // Pedirlo otra vez no cambia la petición.
    b.clock.advance(Duration::from_secs(5));
    let again = b.cancel(&run).await.unwrap();
    assert_eq!(
        again.cancel_requested_at.as_deref(),
        Some(first_request.as_str())
    );
    let pending = pending_cancellations(&b.store, &b.token).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].run_id, run.id);
    assert_eq!(pending[0].state, RunState::Running);

    let done = finish_run(&b.store, &b.token, &run.id, RunOutcome::Cancelled)
        .await
        .unwrap();
    assert_eq!(done.state, RunState::Cancelled);
    assert_eq!(
        done.end_reason.as_deref(),
        Some("Cancelado por una persona.")
    );
    assert!(done.cancel_requested_at.is_some());
    assert!(
        pending_cancellations(&b.store, &b.token)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(code(b.cancel(&run).await), ErrorCode::InvalidRunTransition);

    // El proceso terminó solo antes de que el supervisor lo matara: manda lo observado.
    let raced = b.running("Carrera").await;
    b.cancel(&raced).await.unwrap();
    let done = finish_run(
        &b.store,
        &b.token,
        &raced.id,
        RunOutcome::Exited { code: 0 },
    )
    .await
    .unwrap();
    assert_eq!(done.state, RunState::Completed);

    // Archivar con proceso: petición con su motivo; la historia se archiva igual.
    let run = b.running("Se archiva").await;
    let story = get_story(&b.store, &b.project.id, &run.story_id)
        .await
        .unwrap();
    archive_story(
        &b.store,
        &ArchiveStoryInput {
            project_id: b.project.id.clone(),
            id: story.id.clone(),
            expected_revision: story.revision,
        },
    )
    .await
    .unwrap();
    let detail = get_run(&b.store, &b.project.id, &run.id).await.unwrap();
    assert_eq!(detail.run.state, RunState::Running);
    assert_eq!(
        detail.cancel_reason.as_deref(),
        Some("La historia se archivó.")
    );
    let done = finish_run(&b.store, &b.token, &run.id, RunOutcome::Cancelled)
        .await
        .unwrap();
    assert_eq!(done.end_reason.as_deref(), Some("La historia se archivó."));
}

#[tokio::test]
async fn startup_interrupts_unverifiable_runs_and_voids_pending_starts() {
    let b = Board::new().await;
    let running = b.running("En ejecución").await;
    let preparing = b.run(&b.assigned("Preparando").await).await;
    b.start(&preparing).await;
    b.dispatch(&preparing).await;
    let waiting = b.run(&b.assigned("Esperando hueco").await).await;
    b.start(&waiting).await;
    let idle = b.run(&b.assigned("Sin petición").await).await;

    let interrupted = interrupt_unverifiable_runs(&b.store, &b.token, InterruptCause::AppRestarted)
        .await
        .unwrap();
    let mut ids: Vec<&str> = interrupted.iter().map(|r| r.id.as_str()).collect();
    ids.sort_unstable();
    let mut expected = [running.id.as_str(), preparing.id.as_str()];
    expected.sort_unstable();
    assert_eq!(ids, expected);
    for run in &interrupted {
        assert_eq!(run.state, RunState::Interrupted);
        assert_eq!(
            run.end_reason.as_deref(),
            Some(
                "La app se cerró de forma inesperada durante la ejecución: sus procesos deberían haberse detenido con ella y el worktree, si llegó a crearse, se conserva para revisarlo."
            )
        );
    }
    // Los de la cola no se tocan.
    assert_eq!(b.reload(&waiting).await.state, RunState::Queued);
    assert_eq!(b.reload(&idle).await.state, RunState::Queued);

    // Anular: el intento sigue en cola, sin petición, y nada se despacha solo.
    let voided = void_start_requests(&b.store, &b.token).await.unwrap();
    assert_eq!(voided.len(), 1);
    assert_eq!(voided[0].id, waiting.id);
    assert_eq!(voided[0].state, RunState::Queued);
    assert!(voided[0].start_requested_at.is_none());
    assert!(
        next_dispatchable(&b.store, &b.token)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(b.dispatch(&waiting).await, DispatchOutcome::Stale);
    let detail = get_run(&b.store, &b.project.id, &waiting.id).await.unwrap();
    assert!(detail.start_request.unwrap().voided_at.is_some());
    assert!(
        void_start_requests(&b.store, &b.token)
            .await
            .unwrap()
            .is_empty()
    );

    // Una persona lo vuelve a pedir en esta sesión.
    b.start(&waiting).await;
    let next = next_dispatchable(&b.store, &b.token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.run_id, waiting.id);

    // Al salir de la app: otro motivo.
    let exiting = b.running("Al salir").await;
    let out = interrupt_unverifiable_runs(&b.store, &b.token, InterruptCause::AppExiting)
        .await
        .unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].id, exiting.id);
    assert_eq!(out[0].end_reason.as_deref(), Some("Se cerró la app."));
}

#[tokio::test]
async fn settings_have_defaults_limits_and_revision() {
    let b = Board::new().await;
    let settings = get_run_settings(&b.store).await.unwrap();
    assert_eq!(
        (
            settings.max_concurrency,
            settings.timeout_minutes,
            settings.revision
        ),
        (2, 30, 1)
    );
    let update = |expected_revision, max_concurrency, timeout_minutes| UpdateRunSettingsInput {
        expected_revision,
        max_concurrency,
        timeout_minutes,
        claude_acp_path: None,
        codex_acp_path: None,
        approval_notifications: false,
    };
    for (input, name) in [
        (update(1, 0, 30), "maxConcurrency"),
        (update(1, 9, 30), "maxConcurrency"),
        (update(1, 2, 0), "timeoutMinutes"),
        (update(1, 2, 241), "timeoutMinutes"),
    ] {
        assert_eq!(
            field(update_run_settings(&b.store, &Actor::Human, &input).await),
            name
        );
    }
    assert_eq!(
        code(update_run_settings(&b.store, &agent_actor(), &update(1, 4, 60)).await),
        ErrorCode::Forbidden
    );
    for (concurrency, timeout) in [(1, 1), (8, 240)] {
        let current = get_run_settings(&b.store).await.unwrap();
        let saved = update_run_settings(
            &b.store,
            &Actor::Human,
            &update(current.revision, concurrency, timeout),
        )
        .await
        .unwrap();
        assert_eq!(
            (saved.max_concurrency, saved.timeout_minutes),
            (concurrency, timeout)
        );
        assert_eq!(saved.revision, current.revision + 1);
    }
    // Revisión obsoleta: conflicto con la vigente, sin cambios.
    let error = update_run_settings(&b.store, &Actor::Human, &update(1, 3, 10))
        .await
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Conflict);
    assert_eq!(error.to_payload(None).current.unwrap()["maxConcurrency"], 8);
    // La base también impone los límites.
    assert!(
        sqlx::query("UPDATE run_settings SET max_concurrency = 9")
            .execute(b.store.reader())
            .await
            .is_err()
    );

    // El despacho toma el tiempo máximo vigente.
    let run = b.run(&b.assigned("Con tiempo").await).await;
    b.start(&run).await;
    let DispatchOutcome::Ready(dispatch) = b.dispatch(&run).await else {
        panic!("debía despacharse");
    };
    assert_eq!(dispatch.timeout_minutes, 240);
}

#[tokio::test]
async fn supervisor_token_is_unique_per_store_and_bound_to_it() {
    let b = Board::new().await;
    // Un solo supervisor por almacén (también desde un clon).
    assert_eq!(
        code(SupervisorToken::claim(&b.store.clone())),
        ErrorCode::Forbidden
    );
    // Token de otro almacén: prohibido.
    let other = Board::bare().await;
    assert_eq!(
        code(next_dispatchable(&b.store, &other.token).await),
        ErrorCode::Forbidden
    );
    let run = b.run(&b.assigned("Ajena").await).await;
    let event = RecordRunEventInput {
        project_id: b.project.id.clone(),
        run_id: run.id.clone(),
        sequence: 2,
        event: RunEvent::UsageReported(kanbania_core::features::runs::RunUsage {
            input_tokens: Some(1),
            output_tokens: None,
            cost_usd_micros: None,
        }),
    };
    assert_eq!(
        code(record_run_event(&b.store, &other.token, &event).await),
        ErrorCode::Forbidden
    );
    assert_eq!(
        code(begin_dispatch(&b.store, &other.token, &run.id).await),
        ErrorCode::Forbidden
    );
    // Al soltarlo se puede volver a pedir.
    let Board { store, token, .. } = b;
    drop(token);
    SupervisorToken::claim(&store).unwrap();
}

#[tokio::test]
async fn worktree_removal_is_an_explicit_human_action_on_finished_runs() {
    let b = Board::new().await;
    let run = b.running("Con cambios").await;
    // Activo: primero hay que cancelarlo o esperar.
    assert_eq!(
        field(get_worktree_target(&b.store, &b.project.id, &run.id).await),
        "runId"
    );
    finish_run(&b.store, &b.token, &run.id, RunOutcome::Exited { code: 0 })
        .await
        .unwrap();
    let target = get_worktree_target(&b.store, &b.project.id, &run.id)
        .await
        .unwrap();
    assert!(target.repository_path.ends_with("repo con espacios"));
    assert!(
        Path::new(&target.worktree.path).starts_with(b.data.path().join(WORKTREES_DIR)),
        "bajo el directorio de datos"
    );
    assert!(target.worktree.path.contains(' ') == b.data.path().to_str().unwrap().contains(' '));
    // Otro proyecto: no encontrado.
    assert_eq!(
        code(get_worktree_target(&b.store, "otro", &run.id).await),
        ErrorCode::NotFound
    );

    let input = RecordWorktreeRemovalInput {
        project_id: b.project.id.clone(),
        run_id: run.id.clone(),
        removal: WorktreeRemoval::Discarded,
    };
    assert_eq!(
        code(record_worktree_removal(&b.store, &agent_actor(), &input).await),
        ErrorCode::Forbidden
    );
    let removed = record_worktree_removal(&b.store, &Actor::Human, &input)
        .await
        .unwrap();
    assert_eq!(removed.removal, Some(WorktreeRemoval::Discarded));
    assert!(removed.removed_at.is_some());
    // Ya eliminado: no hay worktree vigente.
    assert_eq!(
        code(record_worktree_removal(&b.store, &Actor::Human, &input).await),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(get_worktree_target(&b.store, &b.project.id, &run.id).await),
        ErrorCode::NotFound
    );
    let detail = get_run(&b.store, &b.project.id, &run.id).await.unwrap();
    assert_eq!(
        detail.worktree.unwrap().removal,
        Some(WorktreeRemoval::Discarded)
    );
}

#[tokio::test]
async fn people_only_operations_reject_agents() {
    let b = Board::new().await;
    let run = b.run(&b.assigned("Personas").await).await;
    let agent = agent_actor();
    assert_eq!(
        code(
            start_run(
                &b.store,
                &agent,
                &b.start_input(&run, TestScenario::Success)
            )
            .await
        ),
        ErrorCode::Forbidden
    );
    assert_eq!(
        code(
            cancel_run(
                &b.store,
                &agent,
                &CancelRunInput {
                    request_id: rid(),
                    project_id: b.project.id.clone(),
                    run_id: run.id.clone(),
                },
            )
            .await
        ),
        ErrorCode::Forbidden
    );
    // Nada cambió.
    let current = b.reload(&run).await;
    assert_eq!(current.state, RunState::Queued);
    assert!(current.start_requested_at.is_none());
}

#[tokio::test]
async fn a_persistent_preparation_failure_does_not_block_the_queue() {
    let b = Board::new().await;
    let first = b.run(&b.assigned("Ilegible").await).await;
    b.start(&first).await;
    let second = b.run(&b.assigned("Siguiente").await).await;
    b.start(&second).await;
    // Instantánea ilegible: solo la prueba quita el trigger de inmutabilidad para simularla.
    let id = first.id.clone();
    b.store
        .write(move |conn| {
            Box::pin(async move {
                sqlx::query("DROP TRIGGER runs_snapshot_immutable")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("UPDATE runs SET snapshot = '{}' WHERE id = ?")
                    .bind(&id)
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .await
        .unwrap();

    let next = next_dispatchable(&b.store, &b.token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.run_id, first.id);
    let DispatchOutcome::Rejected(failed) = b.dispatch(&first).await else {
        panic!("debía fallar");
    };
    assert_eq!(failed.state, RunState::Failed);
    assert!(
        failed
            .end_reason
            .as_deref()
            .unwrap()
            .contains("error interno")
    );
    // La petición quedó consumida: la cola sigue con el siguiente.
    let next = next_dispatchable(&b.store, &b.token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.run_id, second.id);
    assert!(matches!(
        b.dispatch(&second).await,
        DispatchOutcome::Ready(_)
    ));
}

#[tokio::test]
async fn an_unused_worktree_registration_is_discarded_and_restoring_has_its_own_reason() {
    let b = Board::new().await;
    let run = b.run(&b.assigned("Rama ajena").await).await;
    b.start(&run).await;
    b.dispatch(&run).await;
    record_worktree(&b.store, &b.token, &run.id).await.unwrap();
    discard_unused_worktree(&b.store, &b.token, &run.id)
        .await
        .unwrap();
    let detail = get_run(&b.store, &b.project.id, &run.id).await.unwrap();
    assert!(detail.worktree.is_none());
    assert_eq!(detail.run.state, RunState::Preparing);

    let running = b.running("En ejecución").await;
    assert_eq!(
        code(discard_unused_worktree(&b.store, &b.token, &running.id).await),
        ErrorCode::InvalidRunTransition
    );
    assert!(
        get_run(&b.store, &b.project.id, &running.id)
            .await
            .unwrap()
            .worktree
            .is_some()
    );
    let interrupted = interrupt_unverifiable_runs(&b.store, &b.token, InterruptCause::Restoring)
        .await
        .unwrap();
    assert_eq!(interrupted.len(), 2);
    for run in interrupted {
        assert_eq!(
            run.end_reason.as_deref(),
            Some("Se restauró una copia de los datos: no se puede verificar el proceso.")
        );
    }
}

#[tokio::test]
async fn an_unreadable_run_voids_its_request_instead_of_blocking_the_queue() {
    let b = Board::new().await;
    let first = b.run(&b.assigned("Ilegible").await).await;
    b.start(&first).await;
    let second = b.run(&b.assigned("Siguiente").await).await;
    b.start(&second).await;
    // Un estado desconocido (solo la prueba salta el CHECK para simularlo) hace que el
    // intento no se pueda leer: error persistente de decodificación.
    let id = first.id.clone();
    b.store
        .write(move |conn| {
            Box::pin(async move {
                sqlx::query("PRAGMA ignore_check_constraints = ON")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("UPDATE runs SET state = 'desconocido' WHERE id = ?")
                    .bind(&id)
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("PRAGMA ignore_check_constraints = OFF")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .await
        .unwrap();
    assert!(get_run(&b.store, &b.project.id, &first.id).await.is_err());

    // Despacharlo no devuelve un error eterno: su petición se anula y devuelve `Stale`.
    assert_eq!(b.dispatch(&first).await, DispatchOutcome::Stale);
    let voided: Option<i64> =
        sqlx::query_scalar("SELECT voided_at_ms FROM run_start_requests WHERE run_id = ?")
            .bind(&first.id)
            .fetch_one(b.store.reader())
            .await
            .unwrap();
    assert!(voided.is_some());
    // La cola sigue con el siguiente.
    let next = next_dispatchable(&b.store, &b.token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.run_id, second.id);
    assert!(matches!(
        b.dispatch(&second).await,
        DispatchOutcome::Ready(_)
    ));
}
