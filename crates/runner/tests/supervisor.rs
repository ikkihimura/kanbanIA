//! Supervisor de extremo a extremo: SQLite temporal real, repositorio Git real (sucio) y el
//! proceso de prueba real (`kanbania-test-process`). Rutas con espacios en datos y repositorio.

#![cfg(feature = "supervisor")]
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kanbania_core::features::columns::{ColumnCategory, list_columns};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::relations::{SetStoryAgentInput, set_story_agent};
use kanbania_core::features::repositories::{LinkRepositoryInput, link_repository};
use kanbania_core::features::review::{AcceptStoryInput, accept_story};
use kanbania_core::features::review::{SetBlockerInput, set_story_blocker};
use kanbania_core::features::runs::{
    CancelRunInput, CreateRunInput, RunChange, RunDetail, RunEvent, RunOutputStream, RunState,
    RunSummary, StartRunInput, SupervisorToken, TestScenario, UpdateRunSettingsInput, cancel_run,
    create_run, get_run, get_run_settings, start_run, update_run_settings,
};
use kanbania_core::features::stories::{
    ArchiveStoryInput, CreateStoryInput, CriterionDraft, Priority, StorySummary, UpdateStoryInput,
    archive_story, create_story, get_story, update_story,
};
use kanbania_core::features::team::{
    Agent, AgentFields, AvatarSpec, Capability, CreateAgentInput, create_agent,
};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::storage::Store;
use kanbania_runner::supervisor::{Supervisor, SupervisorConfig, read_run_output};
use kanbania_runner::test_process::PIDS_PREFIX;

const UI: &str = "ui";
const TEST_PROCESS: &str = env!("CARGO_BIN_EXE_kanbania-test-process");

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=Prueba",
            "-c",
            "user.email=prueba@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

struct Env {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    repo: PathBuf,
    store: Store,
    project: Project,
    agent: Agent,
    ready_column: String,
    changes: Arc<Mutex<Vec<RunChange>>>,
}

impl Env {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dunce::canonicalize(dir.path()).unwrap();
        let data_dir = base.join("datos con espacios");
        std::fs::create_dir(&data_dir).unwrap();
        let repo = base.join("repositorio con espacios");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("seguido.txt"), "original\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "inicial"]);
        // Repositorio original sucio: cambio sin commit y archivo sin seguimiento.
        std::fs::write(repo.join("seguido.txt"), "cambio local\n").unwrap();
        std::fs::write(repo.join("sin seguimiento.txt"), "nuevo\n").unwrap();

        let store = Store::open(&data_dir).await.unwrap();
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
        let ready_column = list_columns(&store, &project.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.category == ColumnCategory::Ready)
            .unwrap()
            .id;
        Self {
            _dir: dir,
            data_dir,
            repo,
            store,
            project,
            agent,
            ready_column,
            changes: Arc::default(),
        }
    }

    fn config(&self) -> SupervisorConfig {
        let mut config = SupervisorConfig::new(self.data_dir.clone(), PathBuf::from(TEST_PROCESS));
        config.poll_interval = Duration::from_millis(100);
        config.grace = Duration::from_secs(1);
        config
    }

    async fn supervisor(&self, config: SupervisorConfig) -> Supervisor {
        let token = SupervisorToken::claim(&self.store).unwrap();
        let sink = self.changes.clone();
        Supervisor::start(
            self.store.clone(),
            token,
            config,
            Arc::new(move |change| sink.lock().unwrap().push(change)),
        )
        .await
        .unwrap()
    }

    async fn story(&self, title: &str) -> StorySummary {
        let story = create_story(
            &self.store,
            UI,
            &CreateStoryInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                title: title.into(),
                column_id: Some(self.ready_column.clone()),
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
        story
    }

    async fn run(&self, story: &StorySummary) -> RunSummary {
        create_run(
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
        .unwrap()
    }

    async fn start(&self, run: &RunSummary, scenario: TestScenario) {
        start_run(
            &self.store,
            &Actor::Human,
            &StartRunInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                run_id: run.id.clone(),
                repository_id: None,
                executor: kanbania_core::features::runs::RunExecutor::TestProcess { scenario },
            },
        )
        .await
        .unwrap();
    }

    async fn queued(
        &self,
        title: &str,
        scenario: TestScenario,
        supervisor: &Supervisor,
    ) -> RunSummary {
        let story = self.story(title).await;
        let run = self.run(&story).await;
        self.start(&run, scenario).await;
        supervisor.wake();
        run
    }

    async fn detail(&self, run: &RunSummary) -> RunDetail {
        get_run(&self.store, &self.project.id, &run.id)
            .await
            .unwrap()
    }

    async fn wait_for(&self, run: &RunSummary, states: &[RunState]) -> RunDetail {
        let started = Instant::now();
        loop {
            let detail = self.detail(run).await;
            if states.contains(&detail.run.state) {
                return detail;
            }
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "el intento sigue en {:?}",
                detail.run.state
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Pide cancelar (sin despertar al supervisor: eso lo decide cada prueba).
    async fn cancel(&self, run: &RunSummary) -> RunSummary {
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
        .unwrap()
    }

    /// Hora (ISO, comparable como texto) del primer evento que lleva el intento a `to`.
    fn reached(detail: &RunDetail, to: RunState) -> String {
        detail
            .events
            .iter()
            .find(|e| matches!(e.event, RunEvent::StateChanged { to: t, .. } if t == to))
            .map(|e| e.at.clone())
            .unwrap()
    }

    /// `git` falso (Unix): antepone `before_add` a `git worktree add` y añade `after_add`
    /// después; el resto de órdenes van al `git` real.
    #[cfg(unix)]
    fn fake_git(&self, before_add: &str, after_add: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = self.data_dir.join("git falso");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\ncase \" $* \" in\n  *\" worktree add \"*)\n    {before_add}\n    git \"$@\"\n    rc=$?\n    {after_add}\n    exit $rc ;;\nesac\nexec git \"$@\"\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    async fn set_concurrency(&self, max: i64) {
        let current = get_run_settings(&self.store).await.unwrap();
        update_run_settings(
            &self.store,
            &Actor::Human,
            &UpdateRunSettingsInput {
                expected_revision: current.revision,
                max_concurrency: max,
                timeout_minutes: current.timeout_minutes,
                claude_acp_path: current.claude_acp_path,
                codex_acp_path: current.codex_acp_path,
                node_path: None,
                retention_days: None,
                approval_notifications: false,
            },
        )
        .await
        .unwrap();
    }

    fn output(&self, run: &RunSummary, stream: RunOutputStream) -> Vec<String> {
        read_run_output(&self.data_dir, &run.id)
            .unwrap()
            .lines
            .into_iter()
            .filter(|l| l.stream == stream)
            .map(|l| l.text)
            .collect()
    }

    /// Estado observable del repositorio original.
    fn original_state(&self) -> (String, String, String, String) {
        (
            git(&self.repo, &["rev-parse", "HEAD"]),
            git(&self.repo, &["symbolic-ref", "HEAD"]),
            git(
                &self.repo,
                &["status", "--porcelain=v1", "--untracked-files=all"],
            ),
            std::fs::read_to_string(self.repo.join("seguido.txt")).unwrap(),
        )
    }

    /// Espera a que la salida del intento traiga los pids de `slow`.
    async fn slow_pids(&self, run: &RunSummary) -> Vec<i32> {
        let started = Instant::now();
        loop {
            if let Some(line) = self
                .output(run, RunOutputStream::Stdout)
                .iter()
                .find_map(|l| l.strip_prefix(PIDS_PREFIX).map(str::to_owned))
            {
                return line.split(' ').map(|p| p.parse().unwrap()).collect();
            }
            assert!(started.elapsed() < Duration::from_secs(30), "sin pids");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

#[cfg(unix)]
async fn gone(pid: i32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    for _ in 0..100 {
        if kill(Pid::from_raw(pid), None).is_err() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn success_completes_without_accepting_and_keeps_the_original_repository_intact() {
    let env = Env::new().await;
    let before = env.original_state();
    let supervisor = env.supervisor(env.config()).await;
    let story = env.story("Buscador").await;
    let story_before = get_story(&env.store, &env.project.id, &story.id)
        .await
        .unwrap();
    let run = env.run(&story).await;
    env.start(&run, TestScenario::Success).await;
    supervisor.wake();

    let detail = env
        .wait_for(&run, &[RunState::Completed, RunState::Failed])
        .await;
    assert_eq!(
        detail.run.state,
        RunState::Completed,
        "{:?}",
        detail.run.end_reason
    );
    assert_eq!(
        detail.run.end_reason.as_deref(),
        Some("Terminó con código 0.")
    );
    // Completar no acepta la historia ni la mueve.
    let story_after = get_story(&env.store, &env.project.id, &story.id)
        .await
        .unwrap();
    assert!(story_after.accepted_at.is_none());
    assert_eq!(story_after.column_id, story_before.column_id);

    let worktree = detail.worktree.unwrap();
    assert!(worktree.path.contains("datos con espacios"));
    let path = PathBuf::from(&worktree.path);
    assert_eq!(
        git(&path, &["rev-parse", "HEAD"]).trim(),
        worktree.base_commit
    );
    assert_eq!(
        git(&path, &["symbolic-ref", "HEAD"]).trim(),
        format!("refs/heads/{}", worktree.branch)
    );
    // El worktree sale del commit, sin los cambios locales del original.
    assert_eq!(
        std::fs::read_to_string(path.join("seguido.txt")).unwrap(),
        "original\n"
    );

    let stdout = env.output(&run, RunOutputStream::Stdout);
    assert!(stdout.contains(&"paso 2 de 3".to_owned()), "{stdout:?}");
    assert_eq!(
        env.output(&run, RunOutputStream::Stderr),
        ["aviso de prueba en stderr"]
    );
    let system = env.output(&run, RunOutputStream::System);
    assert!(
        system[0].contains("Proceso de prueba (sin IA)"),
        "{system:?}"
    );

    assert_eq!(env.original_state(), before);
    assert!(
        env.changes
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.run_ids == [run.id.clone()] && c.story_ids == [story.id.clone()])
    );
    supervisor.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_and_timeout_end_as_failed_with_core_reasons() {
    let env = Env::new().await;
    let mut config = env.config();
    config.timeout_override = Some(Duration::from_secs(1));
    let supervisor = env.supervisor(config).await;

    let failing = env.queued("Falla", TestScenario::Fail, &supervisor).await;
    let detail = env
        .wait_for(&failing, &[RunState::Failed, RunState::Completed])
        .await;
    assert_eq!(detail.run.state, RunState::Failed);
    assert_eq!(
        detail.run.end_reason.as_deref(),
        Some("Terminó con código 3.")
    );

    let hanging = env.queued("Cuelga", TestScenario::Hang, &supervisor).await;
    let started = Instant::now();
    let detail = env
        .wait_for(&hanging, &[RunState::Failed, RunState::Completed])
        .await;
    assert_eq!(
        detail.run.end_reason.as_deref(),
        Some("Superó el tiempo máximo (30 min).")
    );
    assert!(started.elapsed() < Duration::from_secs(20));
    supervisor.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_running_attempt_kills_its_children() {
    let env = Env::new().await;
    let beats = env.data_dir.join("latidos");
    std::fs::create_dir(&beats).unwrap();
    let mut config = env.config();
    config.scenario_args = vec!["--heartbeat".into(), beats.clone().into()];
    let supervisor = env.supervisor(config).await;
    let run = env.queued("Lenta", TestScenario::Slow, &supervisor).await;
    env.wait_for(&run, &[RunState::Running]).await;
    let pids = env.slow_pids(&run).await;
    assert_eq!(pids.len(), 3);
    common::wait_heartbeats_alive(&beats).await;

    let requested = cancel_run(
        &env.store,
        &Actor::Human,
        &CancelRunInput {
            request_id: rid(),
            project_id: env.project.id.clone(),
            run_id: run.id.clone(),
        },
    )
    .await
    .unwrap();
    // Es una petición: el intento sigue en ejecución hasta que el supervisor la confirma.
    assert_eq!(requested.state, RunState::Running);
    assert!(requested.cancel_requested_at.is_some());
    supervisor.wake();

    let detail = env.wait_for(&run, &[RunState::Cancelled]).await;
    assert_eq!(
        detail.run.end_reason.as_deref(),
        Some("Cancelado por una persona.")
    );
    common::assert_heartbeats_stopped(&beats).await;
    #[cfg(unix)]
    for pid in pids {
        assert!(gone(pid).await, "sigue vivo el pid {pid}");
    }
    supervisor.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrency_limit_is_respected_in_fifo_order() {
    let env = Env::new().await;
    env.set_concurrency(1).await;
    let mut config = env.config();
    config.timeout_override = Some(Duration::from_millis(1500));
    let supervisor = env.supervisor(config).await;
    let mut runs = Vec::new();
    for title in ["Primera", "Segunda", "Tercera"] {
        runs.push(env.queued(title, TestScenario::Hang, &supervisor).await);
    }

    // Con las horas de los eventos (una lectura por intento no es una instantánea común):
    // cada uno empieza a prepararse cuando el anterior ya terminó, en orden de petición.
    let mut spans = Vec::new();
    for run in &runs {
        let detail = env.wait_for(run, &[RunState::Failed]).await;
        spans.push((
            Env::reached(&detail, RunState::Preparing),
            Env::reached(&detail, RunState::Failed),
            run.id.clone(),
        ));
    }
    for pair in spans.windows(2) {
        assert!(pair[0].1 <= pair[1].0, "se solapan: {pair:?}");
    }
    let mut by_start = spans.clone();
    by_start.sort();
    let order: Vec<String> = by_start.into_iter().map(|(_, _, id)| id).collect();
    let expected: Vec<String> = runs.iter().map(|r| r.id.clone()).collect();
    assert_eq!(order, expected);
    supervisor.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_reevaluates_runnable_and_fails_with_the_reason() {
    let env = Env::new().await;
    env.set_concurrency(1).await;
    let supervisor = env.supervisor(env.config()).await;
    // Ocupa el único hueco hasta que la prueba lo cancela: el segundo no puede despacharse
    // antes de quedar bloqueado, dure lo que dure cada paso.
    let first = env
        .queued("Ocupa el hueco", TestScenario::Hang, &supervisor)
        .await;
    env.wait_for(&first, &[RunState::Running]).await;

    let story = env.story("Se bloquea en cola").await;
    let second = env.run(&story).await;
    env.start(&second, TestScenario::Success).await;
    set_story_blocker(
        &env.store,
        &Actor::Human,
        &SetBlockerInput {
            project_id: env.project.id.clone(),
            story_id: story.id.clone(),
            reason: Some("Falta una clave".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(env.detail(&second).await.run.state, RunState::Queued);
    env.cancel(&first).await;
    supervisor.wake();
    env.wait_for(&first, &[RunState::Cancelled]).await;

    let detail = env
        .wait_for(&second, &[RunState::Failed, RunState::Completed])
        .await;
    assert_eq!(detail.run.state, RunState::Failed);
    assert!(
        detail
            .run
            .end_reason
            .as_deref()
            .unwrap()
            .starts_with("No se pudo iniciar"),
        "{:?}",
        detail.run.end_reason
    );
    assert!(detail.worktree.is_none());
    supervisor.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_interrupts_running_attempts_and_kills_their_trees() {
    let env = Env::new().await;
    let beats = env.data_dir.join("latidos");
    std::fs::create_dir(&beats).unwrap();
    let mut config = env.config();
    config.scenario_args = vec!["--heartbeat".into(), beats.clone().into()];
    let supervisor = env.supervisor(config).await;
    let run = env.queued("Lenta", TestScenario::Slow, &supervisor).await;
    env.wait_for(&run, &[RunState::Running]).await;
    let pids = env.slow_pids(&run).await;
    common::wait_heartbeats_alive(&beats).await;

    let started = Instant::now();
    supervisor.shutdown().await;
    assert!(started.elapsed() < Duration::from_secs(15));
    let detail = env.detail(&run).await;
    assert_eq!(detail.run.state, RunState::Interrupted);
    assert_eq!(detail.run.end_reason.as_deref(), Some("Se cerró la app."));
    common::assert_heartbeats_stopped(&beats).await;
    #[cfg(unix)]
    for pid in pids {
        assert!(gone(pid).await, "sigue vivo el pid {pid}");
    }
    // El token se soltó: se puede volver a arrancar (p. ej. tras restaurar una copia).
    SupervisorToken::claim(&env.store).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_moved_repository_fails_and_its_unused_worktree_registration_is_discarded() {
    let env = Env::new().await;
    let supervisor = env.supervisor(env.config()).await;
    let story = env.story("Repositorio movido").await;
    let run = env.run(&story).await;
    std::fs::rename(&env.repo, env.repo.with_file_name("movido")).unwrap();
    env.start(&run, TestScenario::Success).await;
    supervisor.wake();

    let detail = env
        .wait_for(&run, &[RunState::Failed, RunState::Completed])
        .await;
    assert_eq!(detail.run.state, RunState::Failed);
    let reason = detail.run.end_reason.unwrap();
    assert!(
        reason.contains("repositorio no está disponible"),
        "{reason}"
    );
    assert!(!reason.contains(env.data_dir.to_str().unwrap()));
    // Se registró antes de tocar el disco, pero `git worktree add` no llegó a ejecutarse: el
    // registro se descarta (ni la rama ni la carpeta son del intento).
    assert!(detail.worktree.is_none());
    let system = env.output(&run, RunOutputStream::System);
    assert!(system.iter().any(|l| l.contains("Detalle")), "{system:?}");
    supervisor.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn start_requests_from_a_previous_session_are_voided_and_never_launched() {
    let env = Env::new().await;
    let story = env.story("De la sesión anterior").await;
    let run = env.run(&story).await;
    env.start(&run, TestScenario::Success).await;

    let supervisor = env.supervisor(env.config()).await;
    supervisor.wake();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let detail = env.detail(&run).await;
    assert_eq!(detail.run.state, RunState::Queued);
    assert!(detail.run.start_requested_at.is_none());
    assert!(detail.worktree.is_none());
    supervisor.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_attempts_really_run_at_the_same_time_with_limit_two() {
    let env = Env::new().await;
    assert_eq!(
        get_run_settings(&env.store).await.unwrap().max_concurrency,
        2
    );
    let mut config = env.config();
    config.timeout_override = Some(Duration::from_secs(2));
    let supervisor = env.supervisor(config).await;
    let a = env.queued("A", TestScenario::Hang, &supervisor).await;
    let b = env.queued("B", TestScenario::Hang, &supervisor).await;
    let a = env.wait_for(&a, &[RunState::Failed]).await;
    let b = env.wait_for(&b, &[RunState::Failed]).await;
    // Solape real: cada uno empezó a ejecutarse antes de que terminara el otro.
    let (a_run, a_end) = (
        Env::reached(&a, RunState::Running),
        Env::reached(&a, RunState::Failed),
    );
    let (b_run, b_end) = (
        Env::reached(&b, RunState::Running),
        Env::reached(&b, RunState::Failed),
    );
    assert!(
        a_run < b_end && b_run < a_end,
        "{a_run} {a_end} {b_run} {b_end}"
    );
    supervisor.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lowering_the_limit_never_stops_running_attempts() {
    let env = Env::new().await;
    let supervisor = env.supervisor(env.config()).await;
    let a = env.queued("A", TestScenario::Hang, &supervisor).await;
    let b = env.queued("B", TestScenario::Hang, &supervisor).await;
    env.wait_for(&a, &[RunState::Running]).await;
    env.wait_for(&b, &[RunState::Running]).await;

    env.set_concurrency(1).await;
    let c = env.queued("C", TestScenario::Success, &supervisor).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(env.detail(&a).await.run.state, RunState::Running);
    assert_eq!(env.detail(&b).await.run.state, RunState::Running);
    assert_eq!(env.detail(&c).await.run.state, RunState::Queued);

    // Con uno menos sigue habiendo 1 en marcha: el límite (1) sigue lleno.
    env.cancel(&a).await;
    supervisor.wake();
    env.wait_for(&a, &[RunState::Cancelled]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(env.detail(&c).await.run.state, RunState::Queued);
    // Al quedar hueco, sale el que esperaba.
    env.cancel(&b).await;
    supervisor.wake();
    env.wait_for(&c, &[RunState::Completed]).await;
    supervisor.shutdown().await;
}

/// Cancelar en `preparing` mata la orden de Git en curso (un `git worktree add` que no
/// termina) en vez de esperar a su límite de tiempo.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_while_preparing_kills_git_instead_of_waiting() {
    let env = Env::new().await;
    let mut config = env.config();
    config.git_program = env.fake_git("sleep 600", "true");
    let supervisor = env.supervisor(config).await;
    let run = env
        .queued("Git lento", TestScenario::Success, &supervisor)
        .await;
    env.wait_for(&run, &[RunState::Preparing]).await;
    // Que llegue a `git worktree add`.
    tokio::time::sleep(Duration::from_millis(800)).await;

    let started = Instant::now();
    env.cancel(&run).await;
    supervisor.wake();
    let detail = env
        .wait_for(&run, &[RunState::Cancelled, RunState::Failed])
        .await;
    assert_eq!(detail.run.state, RunState::Cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    let system = env.output(&run, RunOutputStream::System);
    assert!(
        system.iter().any(|l| l.contains("Se canceló")),
        "{system:?}"
    );
    supervisor.shutdown().await;
}

/// Una petición de cancelación que llega entre crear el worktree y lanzar el proceso (sin que
/// nadie despierte al supervisor) la ve `mark_running`: el proceso muere en seguida.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_cancellation_seen_by_mark_running_kills_the_process_at_once() {
    let env = Env::new().await;
    let mut config = env.config();
    config.git_program = env.fake_git("true", "sleep 2");
    // Sin sondeo en la práctica: solo `mark_running` puede ver la petición.
    config.poll_interval = Duration::from_secs(120);
    let supervisor = env.supervisor(config).await;
    let run = env.queued("Lenta", TestScenario::Slow, &supervisor).await;
    env.wait_for(&run, &[RunState::Preparing]).await;
    let requested = env.cancel(&run).await;
    assert_eq!(requested.state, RunState::Preparing);

    let started = Instant::now();
    let detail = env
        .wait_for(&run, &[RunState::Cancelled, RunState::Failed])
        .await;
    assert_eq!(detail.run.state, RunState::Cancelled);
    assert_eq!(
        detail.run.end_reason.as_deref(),
        Some("Cancelado por una persona.")
    );
    // `slow` dura 10 min: muere en cuanto pasa a `running`.
    assert!(started.elapsed() < Duration::from_secs(15));
    assert!(detail.events.iter().any(|e| matches!(
        e.event,
        RunEvent::StateChanged {
            to: RunState::Running,
            ..
        }
    )));
    supervisor.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn archiving_or_accepting_with_a_live_process_cancels_it_and_kills_the_tree() {
    let env = Env::new().await;
    let beats = env.data_dir.join("latidos");
    std::fs::create_dir(&beats).unwrap();
    let mut config = env.config();
    config.scenario_args = vec!["--heartbeat".into(), beats.clone().into()];
    let supervisor = env.supervisor(config).await;

    // Archivar.
    let archived = env.story("Se archiva").await;
    let run = env.run(&archived).await;
    env.start(&run, TestScenario::Slow).await;
    supervisor.wake();
    env.wait_for(&run, &[RunState::Running]).await;
    common::wait_heartbeats_alive(&beats).await;
    let current = get_story(&env.store, &env.project.id, &archived.id)
        .await
        .unwrap();
    archive_story(
        &env.store,
        &ArchiveStoryInput {
            project_id: env.project.id.clone(),
            id: archived.id.clone(),
            expected_revision: current.revision,
        },
    )
    .await
    .unwrap();
    supervisor.wake();
    let detail = env.wait_for(&run, &[RunState::Cancelled]).await;
    assert_eq!(
        detail.run.end_reason.as_deref(),
        Some("La historia se archivó.")
    );
    common::assert_heartbeats_stopped(&beats).await;

    // Aceptar.
    let accepted = env.story("Se acepta").await;
    let current = get_story(&env.store, &env.project.id, &accepted.id)
        .await
        .unwrap();
    let edited = update_story(
        &env.store,
        &UpdateStoryInput {
            project_id: env.project.id.clone(),
            id: accepted.id.clone(),
            expected_revision: current.revision,
            title: current.title.clone(),
            description: String::new(),
            priority: Priority::High,
            criteria: vec![CriterionDraft {
                id: None,
                text: "Hecho".into(),
            }],
        },
    )
    .await
    .unwrap();
    let run = env.run(&accepted).await;
    env.start(&run, TestScenario::Slow).await;
    supervisor.wake();
    env.wait_for(&run, &[RunState::Running]).await;
    common::wait_heartbeats_alive(&beats).await;
    accept_story(
        &env.store,
        &Actor::Human,
        &AcceptStoryInput {
            project_id: env.project.id.clone(),
            story_id: accepted.id.clone(),
            expected_revision: edited.revision,
            satisfied_criteria: edited.criteria.iter().map(|c| c.id.clone()).collect(),
            column_id: None,
            to_index: None,
        },
    )
    .await
    .unwrap();
    supervisor.wake();
    let detail = env.wait_for(&run, &[RunState::Cancelled]).await;
    assert_eq!(
        detail.run.end_reason.as_deref(),
        Some("La historia se aceptó.")
    );
    common::assert_heartbeats_stopped(&beats).await;
    supervisor.shutdown().await;
}

/// Sin supervisor (p. ej. no se encontró el ejecutable), la recuperación de la sesión anterior
/// se hace igual con el token.
#[tokio::test(flavor = "multi_thread")]
async fn the_previous_session_is_recovered_without_starting_a_supervisor() {
    let env = Env::new().await;
    let story = env.story("Activa").await;
    let active = env.run(&story).await;
    env.start(&active, TestScenario::Success).await;
    let token = SupervisorToken::claim(&env.store).unwrap();
    kanbania_core::features::runs::begin_dispatch(&env.store, &token, &active.id)
        .await
        .unwrap();
    let waiting_story = env.story("En cola").await;
    let waiting = env.run(&waiting_story).await;
    env.start(&waiting, TestScenario::Success).await;

    let sink = env.changes.clone();
    kanbania_runner::supervisor::recover_previous_session(
        &env.store,
        &token,
        &(Arc::new(move |change| sink.lock().unwrap().push(change))
            as kanbania_runner::supervisor::ChangeSink),
    )
    .await
    .unwrap();
    drop(token);
    assert_eq!(env.detail(&active).await.run.state, RunState::Interrupted);
    let waiting = env.detail(&waiting).await.run;
    assert_eq!(waiting.state, RunState::Queued);
    assert!(waiting.start_requested_at.is_none());
    assert_eq!(env.changes.lock().unwrap().len(), 2);
}
