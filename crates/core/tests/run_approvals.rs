//! Aprobaciones en ejecución (HU-020): peticiones de permiso del agente, decisión de una persona
//! una sola vez, caducidad, carrera con la cancelación, retirada por el agente y reinicio.
//! SQLite temporal real y reloj controlado; el supervisor lo hace la prueba con su token.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kanbania_core::error::{CoreError, ErrorCode};
use kanbania_core::features::columns::{ColumnCategory, list_columns};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::relations::{SetStoryAgentInput, set_story_agent};
use kanbania_core::features::repositories::{LinkRepositoryInput, link_repository};
use kanbania_core::features::runs::{
    APPROVAL_TTL_MS, ApprovalAsk, ApprovalDecision, ApprovalResolution, ApprovalState,
    CancelRunInput, CreateRunInput, DecideApprovalInput, DispatchOutcome, InterruptCause,
    MAX_PENDING_APPROVALS, RunApproval, RunEvent, RunExecutor, RunOutcome, RunState, RunSummary,
    StartRunInput, SupervisorToken, TestScenario, begin_dispatch, cancel_approval, cancel_run,
    create_run, decide_approval, finish_run, get_run, interrupt_unverifiable_runs,
    list_waiting_approvals, mark_running, record_worktree, request_approval, resolve_approval,
    start_run, withdraw_approval,
};
use kanbania_core::features::stories::{CreateStoryInput, Priority, create_story};
use kanbania_core::features::team::{
    Agent, AgentFields, AvatarSpec, Capability, CreateAgentInput, create_agent,
};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::clock::ManualClock;
use kanbania_core::platform::storage::Store;

const UI: &str = "ui";
const START_MS: i64 = 1_794_744_000_000;
const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn code<T: std::fmt::Debug>(result: Result<T, CoreError>) -> ErrorCode {
    result.unwrap_err().code()
}

fn ask(title: &str) -> ApprovalAsk {
    ApprovalAsk {
        tool_title: title.into(),
        tool_kind: Some("edit".into()),
        scope: Some("recetas.py".into()),
        can_allow: true,
    }
}

struct Fixture {
    _data: tempfile::TempDir,
    _repos: tempfile::TempDir,
    store: Store,
    clock: Arc<ManualClock>,
    project: Project,
    agent: Agent,
    token: SupervisorToken,
}

impl Fixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(START_MS));
        let store = Store::open(data.path())
            .await
            .unwrap()
            .with_clock(clock.clone());
        let repos = tempfile::tempdir().unwrap();
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
        let agent = create_agent(
            &store,
            UI,
            &CreateAgentInput {
                request_id: rid(),
                fields: AgentFields {
                    name: "Constructor".into(),
                    role: "Implementa".into(),
                    provider: "Proveedor".into(),
                    model: "modelo-1".into(),
                    capabilities: vec![Capability::Implement],
                    avatar: AvatarSpec::DEFAULT_AGENT,
                },
            },
        )
        .await
        .unwrap();
        let token = SupervisorToken::claim(&store).unwrap();
        let repo = git_repo(repos.path());
        link_repository(
            &store,
            UI,
            &LinkRepositoryInput {
                request_id: rid(),
                project_id: project.id.clone(),
                path: repo.to_str().unwrap().into(),
            },
        )
        .await
        .unwrap();
        Self {
            _data: data,
            _repos: repos,
            store,
            clock,
            project,
            agent,
            token,
        }
    }

    /// Intento en `running` de una historia nueva.
    async fn running(&self, title: &str) -> RunSummary {
        let columns = list_columns(&self.store, &self.project.id).await.unwrap();
        let ready = columns
            .iter()
            .find(|c| c.category == ColumnCategory::Ready)
            .unwrap()
            .id
            .clone();
        let story = create_story(
            &self.store,
            UI,
            &CreateStoryInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                title: title.into(),
                column_id: Some(ready),
                priority: Some(Priority::High),
            },
        )
        .await
        .unwrap();
        set_story_agent(
            &self.store,
            &SetStoryAgentInput {
                project_id: self.project.id.clone(),
                story_id: story.id.clone(),
                agent_id: Some(self.agent.id.clone()),
            },
        )
        .await
        .unwrap();
        let run = create_run(
            &self.store,
            &Actor::Human,
            &CreateRunInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                story_id: story.id.clone(),
                agent_id: None,
                note: None,
            },
        )
        .await
        .unwrap();
        start_run(
            &self.store,
            &Actor::Human,
            &StartRunInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                run_id: run.id.clone(),
                repository_id: None,
                executor: RunExecutor::TestProcess {
                    scenario: TestScenario::Success,
                },
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            begin_dispatch(&self.store, &self.token, &run.id)
                .await
                .unwrap(),
            DispatchOutcome::Ready(_)
        ));
        record_worktree(&self.store, &self.token, &run.id)
            .await
            .unwrap();
        mark_running(&self.store, &self.token, &run.id)
            .await
            .unwrap()
    }

    async fn request(&self, run: &RunSummary, title: &str) -> RunApproval {
        request_approval(&self.store, &self.token, &run.id, &ask(title))
            .await
            .unwrap()
            .approval
    }

    fn decide_input(
        &self,
        approval: &RunApproval,
        decision: ApprovalDecision,
    ) -> DecideApprovalInput {
        DecideApprovalInput {
            request_id: rid(),
            project_id: self.project.id.clone(),
            approval_id: approval.id.clone(),
            decision,
        }
    }

    async fn decide(
        &self,
        approval: &RunApproval,
        decision: ApprovalDecision,
    ) -> Result<RunApproval, CoreError> {
        decide_approval(
            &self.store,
            &Actor::Human,
            &self.decide_input(approval, decision),
        )
        .await
    }

    async fn resolve(&self, approval: &RunApproval) -> ApprovalResolution {
        resolve_approval(&self.store, &self.token, &approval.id)
            .await
            .unwrap()
    }

    async fn state(&self, run: &RunSummary) -> RunState {
        self.reload(run).await.state
    }

    async fn reload(&self, run: &RunSummary) -> RunSummary {
        get_run(&self.store, &self.project.id, &run.id)
            .await
            .unwrap()
            .run
    }

    async fn approval(&self, approval: &RunApproval) -> RunApproval {
        let detail = get_run(&self.store, &self.project.id, &approval.run_id)
            .await
            .unwrap();
        detail
            .approvals
            .into_iter()
            .find(|a| a.id == approval.id)
            .unwrap()
    }

    async fn cancel(&self, run: &RunSummary) {
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
        .unwrap();
    }
}

fn git_repo(parent: &Path) -> PathBuf {
    let root = parent.join("repo");
    std::fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
    std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(root.join(".git/refs/heads/main"), format!("{SHA_A}\n")).unwrap();
    root
}

#[tokio::test]
async fn a_request_waits_for_a_person_and_an_allow_is_delivered_once() {
    let f = Fixture::new().await;
    let run = f.running("Permitir").await;
    let requested = request_approval(&f.store, &f.token, &run.id, &ask("Write recetas.py"))
        .await
        .unwrap();
    let approval = requested.approval.clone();
    assert_eq!(approval.sequence, 1);
    assert_eq!(approval.state, ApprovalState::Pending);
    assert_eq!(requested.expires_at_ms, START_MS + APPROVAL_TTL_MS);
    assert_eq!(requested.story_id, run.story_id);
    assert_eq!(f.state(&run).await, RunState::WaitingApproval);
    // Señal real para la bandeja y el tablero.
    let waiting = list_waiting_approvals(&f.store, &f.project.id)
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].run_id, run.id);
    assert_eq!(waiting[0].pending, 1);
    assert_eq!(
        f.resolve(&approval).await,
        ApprovalResolution::Pending {
            expires_at_ms: START_MS + APPROVAL_TTL_MS
        }
    );

    // Una sola decisión: reintentar con el mismo `requestId` devuelve lo mismo; otra decisión
    // se rechaza.
    let input = f.decide_input(&approval, ApprovalDecision::AllowOnce);
    let allowed = decide_approval(&f.store, &Actor::Human, &input)
        .await
        .unwrap();
    assert_eq!(allowed.state, ApprovalState::Allowed);
    assert!(!allowed.delivered);
    let again = decide_approval(&f.store, &Actor::Human, &input)
        .await
        .unwrap();
    assert_eq!(again, allowed);
    assert_eq!(
        code(f.decide(&approval, ApprovalDecision::Reject).await),
        ErrorCode::ApprovalClosed
    );
    assert_eq!(f.state(&run).await, RunState::Running);
    assert!(
        list_waiting_approvals(&f.store, &f.project.id)
            .await
            .unwrap()
            .is_empty()
    );

    // Se entrega una vez.
    assert_eq!(
        f.resolve(&approval).await,
        ApprovalResolution::Deliver(ApprovalDecision::AllowOnce)
    );
    assert_eq!(f.resolve(&approval).await, ApprovalResolution::Closed);
    assert!(f.approval(&approval).await.delivered);

    // Eventos: petición, espera, decisión y vuelta a ejecución.
    let detail = get_run(&f.store, &f.project.id, &run.id).await.unwrap();
    let kinds: Vec<_> = detail
        .events
        .iter()
        .filter_map(|e| match &e.event {
            RunEvent::ApprovalRequested { .. } => Some("requested".to_owned()),
            RunEvent::ApprovalResolved { outcome, .. } => Some(format!("{outcome:?}")),
            RunEvent::StateChanged { to, .. } if e.sequence > 3 => Some(format!("{to:?}")),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        ["requested", "WaitingApproval", "Allowed", "Running"]
    );
}

#[tokio::test]
async fn a_rejection_is_delivered_as_a_rejection() {
    let f = Fixture::new().await;
    let run = f.running("Rechazar").await;
    let approval = f.request(&run, "Bash rm -rf build").await;
    let rejected = f.decide(&approval, ApprovalDecision::Reject).await.unwrap();
    assert_eq!(rejected.state, ApprovalState::Rejected);
    assert_eq!(
        f.resolve(&approval).await,
        ApprovalResolution::Deliver(ApprovalDecision::Reject)
    );
}

#[tokio::test]
async fn a_late_decision_after_expiry_is_refused_and_the_agent_gets_a_rejection() {
    let f = Fixture::new().await;
    let run = f.running("Caducar").await;
    let approval = f.request(&run, "Write nota.txt").await;
    f.clock.advance(Duration::from_millis(
        u64::try_from(APPROVAL_TTL_MS).unwrap(),
    ));
    // La bandeja ya no la muestra y decidir no cambia nada.
    assert!(
        list_waiting_approvals(&f.store, &f.project.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        code(f.decide(&approval, ApprovalDecision::AllowOnce).await),
        ErrorCode::ApprovalClosed
    );
    assert_eq!(f.approval(&approval).await.state, ApprovalState::Pending);
    // El supervisor la cierra como caducada (rechazo) y el intento sigue.
    assert_eq!(f.resolve(&approval).await, ApprovalResolution::Expired);
    let expired = f.approval(&approval).await;
    assert_eq!(expired.state, ApprovalState::Expired);
    assert!(expired.delivered);
    assert_eq!(f.state(&run).await, RunState::Running);
    assert_eq!(f.resolve(&approval).await, ApprovalResolution::Closed);
}

#[tokio::test]
async fn cancelling_invalidates_an_allow_that_was_not_delivered_yet() {
    let f = Fixture::new().await;
    let run = f.running("Carrera").await;
    let approval = f.request(&run, "Write nota.txt").await;
    f.decide(&approval, ApprovalDecision::AllowOnce)
        .await
        .unwrap();
    let decided_at = f.approval(&approval).await.decided_at;
    // La persona cancela (un minuto después) antes de que el supervisor entregue la aprobación.
    f.clock.advance(Duration::from_secs(60));
    f.cancel(&run).await;
    let closed = f.approval(&approval).await;
    assert_eq!(closed.state, ApprovalState::Cancelled);
    assert!(!closed.delivered);
    // Se conserva cuándo decidió la persona.
    assert_eq!(closed.decided_at, decided_at);
    assert_eq!(f.resolve(&approval).await, ApprovalResolution::Closed);
}

#[tokio::test]
async fn after_a_cancel_request_nothing_can_be_decided_or_requested() {
    let f = Fixture::new().await;
    let run = f.running("Cancelar primero").await;
    let first = f.request(&run, "Write a.txt").await;
    let second = f.request(&run, "Write b.txt").await;
    f.cancel(&run).await;
    for approval in [&first, &second] {
        assert_eq!(f.approval(approval).await.state, ApprovalState::Cancelled);
        assert_eq!(
            code(f.decide(approval, ApprovalDecision::AllowOnce).await),
            ErrorCode::ApprovalClosed
        );
        assert_eq!(f.resolve(approval).await, ApprovalResolution::Closed);
    }
    assert_eq!(
        code(request_approval(&f.store, &f.token, &run.id, &ask("Write c.txt")).await),
        ErrorCode::ApprovalClosed
    );
    // El supervisor confirma la cancelación desde `waiting_approval`.
    let ended = finish_run(&f.store, &f.token, &run.id, RunOutcome::Cancelled)
        .await
        .unwrap();
    assert_eq!(ended.state, RunState::Cancelled);
}

#[tokio::test]
async fn ending_the_run_closes_open_requests_but_keeps_delivered_ones() {
    let f = Fixture::new().await;
    let run = f.running("Terminar").await;
    let delivered = f.request(&run, "Write a.txt").await;
    f.decide(&delivered, ApprovalDecision::AllowOnce)
        .await
        .unwrap();
    f.resolve(&delivered).await;
    let pending = f.request(&run, "Write b.txt").await;
    let ended = finish_run(&f.store, &f.token, &run.id, RunOutcome::AgentEndedTurn)
        .await
        .unwrap();
    assert_eq!(ended.state, RunState::Completed);
    assert_eq!(f.approval(&delivered).await.state, ApprovalState::Allowed);
    assert_eq!(f.approval(&pending).await.state, ApprovalState::Cancelled);
    assert_eq!(
        code(f.decide(&pending, ApprovalDecision::Reject).await),
        ErrorCode::ApprovalClosed
    );
}

#[tokio::test]
async fn the_agent_can_withdraw_a_request() {
    let f = Fixture::new().await;
    let run = f.running("Retirar").await;
    let approval = f.request(&run, "Write nota.txt").await;
    let withdrawn = withdraw_approval(&f.store, &f.token, &approval.id)
        .await
        .unwrap();
    assert_eq!(withdrawn.state, ApprovalState::Withdrawn);
    assert_eq!(f.state(&run).await, RunState::Running);
    assert_eq!(
        code(f.decide(&approval, ApprovalDecision::AllowOnce).await),
        ErrorCode::ApprovalClosed
    );
    assert_eq!(f.resolve(&approval).await, ApprovalResolution::Closed);
    // Retirar dos veces no hace nada.
    let again = withdraw_approval(&f.store, &f.token, &approval.id)
        .await
        .unwrap();
    assert_eq!(again, withdrawn);
}

#[tokio::test]
async fn the_run_waits_until_every_request_is_answered() {
    let f = Fixture::new().await;
    let run = f.running("Varias").await;
    let first = f.request(&run, "Write a.txt").await;
    let second = f.request(&run, "Write b.txt").await;
    assert_eq!(second.sequence, 2);
    let waiting = list_waiting_approvals(&f.store, &f.project.id)
        .await
        .unwrap();
    assert_eq!(waiting[0].pending, 2);
    f.decide(&first, ApprovalDecision::Reject).await.unwrap();
    assert_eq!(f.state(&run).await, RunState::WaitingApproval);
    f.decide(&second, ApprovalDecision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(f.state(&run).await, RunState::Running);
}

#[tokio::test]
async fn allow_requires_the_option_from_the_agent() {
    let f = Fixture::new().await;
    let run = f.running("Sin permitir").await;
    let approval = request_approval(
        &f.store,
        &f.token,
        &run.id,
        &ApprovalAsk {
            can_allow: false,
            ..ask("Write nota.txt")
        },
    )
    .await
    .unwrap()
    .approval;
    assert!(!approval.can_allow);
    let error = f
        .decide(&approval, ApprovalDecision::AllowOnce)
        .await
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Validation);
    assert_eq!(error.to_payload(None).field.as_deref(), Some("decision"));
    f.decide(&approval, ApprovalDecision::Reject).await.unwrap();
}

#[tokio::test]
async fn agent_text_is_cleaned_and_secrets_are_hidden() {
    let f = Fixture::new().await;
    let run = f.running("Limpiar").await;
    let approval = request_approval(
        &f.store,
        &f.token,
        &run.id,
        &ApprovalAsk {
            tool_title: "Write\u{202E}\u{200B} nota\n.txt\t".into(),
            tool_kind: Some("Not A Kind!".into()),
            scope: Some("curl -H 'x-api-key: sk-ant-api03-abcdefghijklmnopqrstuvwxyz'".into()),
            can_allow: true,
        },
    )
    .await
    .unwrap()
    .approval;
    assert_eq!(approval.tool_title, "Write nota .txt");
    assert_eq!(approval.tool_kind, None);
    assert_eq!(
        approval.scope.as_deref(),
        Some("[oculto: parecía contener un secreto]")
    );
    let long = request_approval(
        &f.store,
        &f.token,
        &run.id,
        &ApprovalAsk {
            tool_title: "x".repeat(1000),
            tool_kind: None,
            scope: Some("   ".into()),
            can_allow: true,
        },
    )
    .await
    .unwrap()
    .approval;
    assert_eq!(long.tool_title.chars().count(), 300);
    assert!(long.tool_title.ends_with('…'));
    assert_eq!(long.scope, None);
}

#[tokio::test]
async fn only_people_of_the_project_decide() {
    let f = Fixture::new().await;
    let run = f.running("Quién decide").await;
    let approval = f.request(&run, "Write nota.txt").await;
    let agent = Actor::Agent {
        agent_id: f.agent.id.clone(),
    };
    assert_eq!(
        code(
            decide_approval(
                &f.store,
                &agent,
                &f.decide_input(&approval, ApprovalDecision::AllowOnce)
            )
            .await
        ),
        ErrorCode::Forbidden
    );
    let other = create_project(
        &f.store,
        UI,
        &CreateProjectInput {
            request_id: rid(),
            name: "Otro".into(),
        },
    )
    .await
    .unwrap();
    let input = DecideApprovalInput {
        project_id: other.id,
        ..f.decide_input(&approval, ApprovalDecision::AllowOnce)
    };
    assert_eq!(
        code(decide_approval(&f.store, &Actor::Human, &input).await),
        ErrorCode::NotFound
    );
    assert_eq!(f.approval(&approval).await.state, ApprovalState::Pending);
}

#[tokio::test]
async fn too_many_pending_requests_are_refused() {
    let f = Fixture::new().await;
    let run = f.running("Muchas").await;
    for n in 0..MAX_PENDING_APPROVALS {
        f.request(&run, &format!("Write {n}.txt")).await;
    }
    assert_eq!(
        code(request_approval(&f.store, &f.token, &run.id, &ask("Write otra.txt")).await),
        ErrorCode::Validation
    );
}

#[tokio::test]
async fn restarting_the_app_interrupts_the_run_and_closes_its_requests() {
    let f = Fixture::new().await;
    let run = f.running("Reiniciar").await;
    let approval = f.request(&run, "Write nota.txt").await;
    let changed = interrupt_unverifiable_runs(&f.store, &f.token, InterruptCause::AppRestarted)
        .await
        .unwrap();
    assert_eq!(changed.len(), 1);
    assert_eq!(f.state(&run).await, RunState::Interrupted);
    assert_eq!(f.approval(&approval).await.state, ApprovalState::Cancelled);
    assert_eq!(
        code(f.decide(&approval, ApprovalDecision::AllowOnce).await),
        ErrorCode::ApprovalClosed
    );
}

#[tokio::test]
async fn a_request_needs_a_running_run() {
    let f = Fixture::new().await;
    let run = f.running("Terminado").await;
    finish_run(&f.store, &f.token, &run.id, RunOutcome::AgentEndedTurn)
        .await
        .unwrap();
    assert_eq!(
        code(request_approval(&f.store, &f.token, &run.id, &ask("Write nota.txt")).await),
        ErrorCode::ApprovalClosed
    );
}

#[tokio::test]
async fn a_request_the_session_answered_cancelled_is_closed() {
    let f = Fixture::new().await;
    let run = f.running("Sesión cerrada").await;
    let approval = f.request(&run, "Write nota.txt").await;
    let closed = cancel_approval(&f.store, &f.token, &approval.id)
        .await
        .unwrap();
    assert_eq!(closed.state, ApprovalState::Cancelled);
    assert_eq!(f.state(&run).await, RunState::Running);
    assert_eq!(
        code(f.decide(&approval, ApprovalDecision::AllowOnce).await),
        ErrorCode::ApprovalClosed
    );
    assert_eq!(f.resolve(&approval).await, ApprovalResolution::Closed);
}
