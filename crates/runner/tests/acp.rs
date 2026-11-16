//! Contrato del cliente ACP (HU-019) con el agente falso `kanbania-fake-acp`, de extremo a
//! extremo por el supervisor: SQLite temporal real, repositorio Git real, worktree real y el
//! adaptador lanzado desde una ruta con espacios. Comprueba estado final, motivo exacto del
//! núcleo, evento `acp_session`, salida normalizada (`agent`/`err`/`sys`) y lo que nunca debe
//! llegar a la salida (correo de la cuenta, pensamiento, entrada y contenido de herramientas,
//! `_meta`). Los mocks no prueban compatibilidad con el proveedor real: eso es `acp_real.rs`.

#![cfg(feature = "supervisor")]
#![allow(clippy::unwrap_used, clippy::panic)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kanbania_core::features::columns::{ColumnCategory, list_columns};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::relations::{SetStoryAgentInput, set_story_agent};
use kanbania_core::features::repositories::{LinkRepositoryInput, link_repository};
use kanbania_core::features::runs::{
    APPROVAL_TTL_MS, AcpAgent, AcpSessionInfo, ApprovalDecision, ApprovalState, CancelRunInput,
    CreateRunInput, DecideApprovalInput, RunApproval, RunDetail, RunEvent, RunExecutor,
    RunOutputKind, RunOutputLine, RunOutputStream, RunState, RunSummary, StartRunInput,
    SupervisorToken, UpdateRunSettingsInput, cancel_run, create_run, decide_approval, get_run,
    get_run_settings, prompt_digest, start_run, update_run_settings,
};
use kanbania_core::features::stories::{CreateStoryInput, Priority, create_story};
use kanbania_core::features::team::{
    Agent, AgentFields, AvatarSpec, Capability, CreateAgentInput, create_agent,
};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::clock::ManualClock;
use kanbania_core::platform::storage::Store;
use kanbania_runner::acp::{
    AcpClient, AcpLimits, CLAUDE_CODE_PROFILE, CODEX_HOME_NOTE, ISOLATION_NOTE, ProbeError,
    TurnEnd, codex_env, probe_adapter,
};
use kanbania_runner::output::{AgentKind, OutputLog, read_tail};
use kanbania_runner::process::{ProcessSpec, RunningProcess, cancel_pair};
use kanbania_runner::supervisor::{
    ApprovalNotice, ApprovalNotifier, Supervisor, SupervisorConfig, read_run_output,
};

const UI: &str = "ui";
/// Correo ficticio y textos ocultos que envía el agente falso (`src/bin/kanbania-fake-acp`):
/// nunca deben llegar a la salida. Si cambian allí, cámbialos aquí.
const FAKE_EMAIL: &str = "persona.ficticia@example.invalid";
const HIDDEN_MARKERS: [&str; 4] = [
    "PENSAMIENTO-SECRETO",
    "ENTRADA-SECRETA",
    "CONTENIDO-SECRETO",
    "META-SECRETO",
];
const FAKE_ACP: &str = env!("CARGO_BIN_EXE_kanbania-fake-acp");
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

/// Copia el agente falso en una carpeta con espacios (ruta del adaptador con espacios).
fn install_adapter(base: &Path) -> PathBuf {
    let dir = base.join("adaptador con espacios");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(if cfg!(windows) {
        "agente falso.exe"
    } else {
        "agente falso"
    });
    std::fs::copy(FAKE_ACP, &path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

struct Env {
    _dir: tempfile::TempDir,
    base: PathBuf,
    data_dir: PathBuf,
    store: Store,
    project: Project,
    agent: Agent,
    ready_column: String,
    adapter: PathBuf,
}

impl Env {
    async fn new() -> Self {
        Self::build(None).await
    }

    /// Con reloj controlado (caducidad de los permisos).
    async fn with_clock(clock: Arc<ManualClock>) -> Self {
        Self::build(Some(clock)).await
    }

    async fn build(clock: Option<Arc<ManualClock>>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dunce::canonicalize(dir.path()).unwrap();
        let data_dir = base.join("datos con espacios");
        std::fs::create_dir(&data_dir).unwrap();
        let repo = base.join("repositorio con espacios");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("README.md"), "# Prueba\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "inicial"]);
        let adapter = install_adapter(&base);

        let mut store = Store::open(&data_dir).await.unwrap();
        if let Some(clock) = clock {
            store = store.with_clock(clock);
        }
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
                    provider: "Anthropic".into(),
                    model: "claude".into(),
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
        let env = Self {
            _dir: dir,
            base,
            data_dir,
            store,
            project,
            agent,
            ready_column,
            adapter,
        };
        env.set_adapter(Some(env.adapter.to_str().unwrap().to_owned()))
            .await;
        env
    }

    async fn set_adapter(&self, path: Option<String>) {
        self.set_settings(path, false).await;
    }

    async fn set_settings(&self, path: Option<String>, approval_notifications: bool) {
        let current = get_run_settings(&self.store).await.unwrap();
        update_run_settings(
            &self.store,
            &Actor::Human,
            &UpdateRunSettingsInput {
                expected_revision: current.revision,
                max_concurrency: current.max_concurrency,
                timeout_minutes: current.timeout_minutes,
                // El mismo agente falso para los dos: su variante la decide el entorno.
                claude_acp_path: path.clone(),
                codex_acp_path: path,
                approval_notifications,
            },
        )
        .await
        .unwrap();
    }

    /// Espera una petición de permiso pendiente del intento y la decide como persona.
    async fn decide_pending(&self, run: &RunSummary, decision: ApprovalDecision) -> RunApproval {
        let pending = self.wait_pending(run).await;
        decide_approval(
            &self.store,
            &Actor::Human,
            &DecideApprovalInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                approval_id: pending.id,
                decision,
            },
        )
        .await
        .unwrap()
    }

    async fn wait_pending(&self, run: &RunSummary) -> RunApproval {
        let started = Instant::now();
        loop {
            let detail = self.detail(run).await;
            if let Some(pending) = detail
                .approvals
                .into_iter()
                .find(|a| a.state == ApprovalState::Pending)
            {
                assert_eq!(detail.run.state, RunState::WaitingApproval);
                return pending;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "no llegó ninguna petición de permiso ({:?})",
                detail.run.state
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Worktree del intento.
    async fn worktree(&self, run: &RunSummary) -> PathBuf {
        PathBuf::from(self.detail(run).await.worktree.unwrap().path)
    }

    fn config(&self, args: &[&str]) -> SupervisorConfig {
        let mut config = SupervisorConfig::new(self.data_dir.clone(), PathBuf::from(TEST_PROCESS));
        config.poll_interval = Duration::from_millis(100);
        config.grace = Duration::from_secs(1);
        config.acp_args = args.iter().map(OsString::from).collect();
        config.acp_limits = AcpLimits {
            request_timeout: Duration::from_secs(20),
            cancel_wait: Duration::from_secs(2),
            shutdown_cancel_wait: Duration::from_secs(1),
            close_wait: Duration::from_secs(2),
            write_timeout: Duration::from_secs(10),
        };
        config
    }

    async fn supervisor(&self, config: SupervisorConfig) -> Supervisor {
        let token = SupervisorToken::claim(&self.store).unwrap();
        Supervisor::start(self.store.clone(), token, config, Arc::new(|_| {}))
            .await
            .unwrap()
    }

    /// Crea una historia lista con agente, un intento y pide iniciarlo con Claude Code (ACP).
    async fn queued(&self, supervisor: &Supervisor) -> RunSummary {
        self.queued_as(supervisor, RunExecutor::ClaudeAcp).await
    }

    /// Igual, con el ejecutor indicado.
    async fn queued_as(&self, supervisor: &Supervisor, executor: RunExecutor) -> RunSummary {
        let story = create_story(
            &self.store,
            UI,
            &CreateStoryInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                title: "Resumir el README".into(),
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
                executor,
            },
        )
        .await
        .unwrap();
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

    async fn wait_terminal(&self, run: &RunSummary) -> RunDetail {
        self.wait_for(
            run,
            &[
                RunState::Completed,
                RunState::Failed,
                RunState::Cancelled,
                RunState::Interrupted,
            ],
        )
        .await
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

    fn output(&self, run: &RunSummary) -> Vec<RunOutputLine> {
        read_run_output(&self.data_dir, &run.id).unwrap().lines
    }

    fn texts(&self, run: &RunSummary, stream: RunOutputStream) -> Vec<String> {
        self.output(run)
            .into_iter()
            .filter(|l| l.stream == stream)
            .map(|l| l.text)
            .collect()
    }

    fn agent_lines(&self, run: &RunSummary, kind: RunOutputKind) -> Vec<String> {
        self.output(run)
            .into_iter()
            .filter(|l| l.stream == RunOutputStream::Agent && l.kind == Some(kind))
            .map(|l| l.text)
            .collect()
    }

    /// Repositorio vinculado (git real).
    fn repo(&self) -> PathBuf {
        self.base.join("repositorio con espacios")
    }

    /// Añade un archivo al repositorio y hace commit (antes de crear el intento: la
    /// instantánea toma ese HEAD).
    fn commit_file(&self, relative: &str, content: &str) {
        let repo = self.repo();
        let path = repo.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        git(&repo, &["add", "-f", "."]);
        git(&repo, &["commit", "-q", "-m", "añade archivo"]);
    }

    /// Todo el JSONL del intento, en bruto.
    fn raw_output(&self, run: &RunSummary) -> String {
        let dir = self.data_dir.join("runs").join(&run.id);
        let mut all = String::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            all.push_str(&std::fs::read_to_string(entry.unwrap().path()).unwrap());
        }
        all
    }

    /// Ejecuta un escenario hasta un estado terminal.
    async fn scenario(&self, args: &[&str]) -> (RunSummary, RunDetail) {
        self.scenario_as(RunExecutor::ClaudeAcp, args).await
    }

    /// Igual, con el ejecutor indicado.
    async fn scenario_as(&self, executor: RunExecutor, args: &[&str]) -> (RunSummary, RunDetail) {
        let supervisor = self.supervisor(self.config(args)).await;
        let run = self.queued_as(&supervisor, executor).await;
        let detail = self.wait_terminal(&run).await;
        supervisor.shutdown().await;
        (run, detail)
    }
}

const REQUEST_FAILED: &str =
    "El agente devolvió un error al negociar la sesión. Consulta el registro del intento.";

/// Métodos (y notas) que registró el agente falso con `--log`.
fn received(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Espera a que el intento tenga su sesión ACP registrada (el turno ya empezó).
async fn wait_session(env: &Env, run: &RunSummary) {
    let started = Instant::now();
    while sessions(&env.detail(run).await).is_empty() {
        assert!(started.elapsed() < Duration::from_secs(30));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
}

fn sessions(detail: &RunDetail) -> Vec<AcpSessionInfo> {
    detail
        .events
        .iter()
        .filter_map(|e| match &e.event {
            RunEvent::AcpSession(info) => Some(info.clone()),
            _ => None,
        })
        .collect()
}

fn assert_failed(detail: &RunDetail, reason: &str) {
    assert_eq!(detail.run.state, RunState::Failed, "{:?}", detail.run);
    assert_eq!(detail.run.end_reason.as_deref(), Some(reason));
}

fn assert_nothing_hidden(raw: &str) {
    assert!(!raw.contains(FAKE_EMAIL), "el correo llegó a la salida");
    for marker in HIDDEN_MARKERS {
        assert!(!raw.contains(marker), "{marker} llegó a la salida");
    }
}

#[cfg(unix)]
async fn alive_after_grace(pid: i32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    for _ in 0..50 {
        if kill(Pid::from_raw(pid), None).is_err() {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    true
}

#[tokio::test(flavor = "multi_thread")]
async fn full_turn_is_normalized_and_session_recorded() {
    let env = Env::new().await;
    let received = env.base.join("prompt recibido.txt");
    let (run, detail) = env
        .scenario(&["ok", "--prompt-file", received.to_str().unwrap()])
        .await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    assert_eq!(
        detail.run.end_reason.as_deref(),
        Some("El agente terminó su turno.")
    );

    // CA-1: lo declarado, sin `_meta`, con el modo fijado y la huella del texto que recibió
    // el agente (byte a byte).
    let (sha256, chars) = prompt_digest(&std::fs::read_to_string(&received).unwrap());
    let sessions = sessions(&detail);
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0],
        AcpSessionInfo {
            protocol_version: 1,
            agent_name: Some("kanbania-fake-acp".into()),
            agent_title: Some("Agente ACP falso (pruebas)".into()),
            agent_version: Some("0.0.1-prueba".into()),
            capabilities: [
                "loadSession",
                "image",
                "embeddedContext",
                "mcpHttp",
                "modes"
            ]
            .map(String::from)
            .to_vec(),
            mode: Some("default".into()),
            prompt_sha256: Some(sha256),
            prompt_chars: Some(chars),
            model: None,
        }
    );

    // CA-2: salida normalizada con el origen.
    assert_eq!(
        env.agent_lines(&run, RunOutputKind::Message),
        ["He leído el README.", "Fin."]
    );
    assert_eq!(
        env.agent_lines(&run, RunOutputKind::Thought),
        ["(pensando…, 19 caracteres)"]
    );
    assert_eq!(
        env.agent_lines(&run, RunOutputKind::Tool),
        [
            "«Leer README.md» · tipo: read · estado: pendiente",
            "«Leer README.md» · tipo: read · estado: completada"
        ]
    );
    assert_eq!(
        env.agent_lines(&run, RunOutputKind::Plan),
        ["[completada] Leer el README\n[en curso] Resumirlo"]
    );
    assert_eq!(
        env.agent_lines(&run, RunOutputKind::Usage),
        ["Contexto: 1200 de 200000 tokens. Coste acumulado: 0.01 USD."]
    );
    assert!(env.agent_lines(&run, RunOutputKind::Permission).is_empty());
    // Ninguna línea `agent` sin tipo; ninguna otra con tipo.
    for line in env.output(&run) {
        assert_eq!(
            line.kind.is_some(),
            line.stream == RunOutputStream::Agent,
            "{line:?}"
        );
    }
    let stderr = env.texts(&run, RunOutputStream::Stderr);
    assert!(
        stderr.contains(&"[session/create] sesión de prueba".to_owned()),
        "{stderr:?}"
    );
    assert!(
        stderr.iter().any(|l| l.starts_with("prompt recibido: ")),
        "{stderr:?}"
    );
    // stderr del adaptador: correo y prefijos de secretos ocultos.
    assert!(
        stderr.contains(&"[auth] cuenta [oculto] clave [oculto]".to_owned()),
        "{stderr:?}"
    );
    // stdout del adaptador es el canal del protocolo: no se copia.
    assert!(env.texts(&run, RunOutputStream::Stdout).is_empty());

    let system = env.texts(&run, RunOutputStream::System);
    assert_eq!(
        system
            .iter()
            .filter(|l| *l
                == "La app pidió no cargar la configuración de Claude Code (usuario, proyecto ni local).")
            .count(),
        1
    );
    let auth_notes: Vec<_> = system
        .iter()
        .filter(|l| l.contains("«_auth/status_update»"))
        .collect();
    assert_eq!(auth_notes.len(), 1, "{system:?}");
    assert!(system.iter().any(|l| l.contains("«auth/status_update»")));
    assert!(
        system.iter().any(|l| l.contains("«fs/read_text_file»")),
        "{system:?}"
    );
    assert!(
        system
            .iter()
            .any(|l| l.contains("Modo inicial que informa el agente: «auto»"))
    );
    assert!(
        system
            .iter()
            .any(|l| l.starts_with("Fin de la sesión ACP: el agente terminó su turno."))
    );
    assert_nothing_hidden(&env.raw_output(&run));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejection_by_a_person_is_answered_reject_once() {
    let env = Env::new().await;
    let supervisor = env.supervisor(env.config(&["permission"])).await;
    let run = env.queued(&supervisor).await;
    let decided = env.decide_pending(&run, ApprovalDecision::Reject).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    // El agente falso termina con error si no recibe `reject_once` (o `cancelled` sin esa
    // opción): llegar a `completed` prueba la respuesta.
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    assert_eq!(decided.tool_title, "Editar README.md");
    assert_eq!(decided.tool_kind.as_deref(), Some("edit"));
    assert_eq!(detail.approvals.len(), 1, "{:?}", detail.approvals);
    assert_eq!(detail.approvals[0].state, ApprovalState::Rejected);
    assert!(detail.approvals[0].delivered);
    let permissions = env.agent_lines(&run, RunOutputKind::Permission);
    assert_eq!(
        permissions,
        [
            "Permiso pedido para «Editar README.md»: espera la decisión de una persona en la app.",
            "Permiso para «Editar README.md»: rechazado por una persona.",
            "Permiso pedido para «Ejecutar  pruebas»: sin opción «rechazar una vez», respondido como cancelado.",
        ]
    );
    // El U+202E del título se sustituye: nunca llega a la salida.
    let raw = env.raw_output(&run);
    assert!(!raw.contains('\u{202E}') && !raw.to_lowercase().contains("\\u202e"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_allow_once_is_applied_and_the_action_runs() {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["approval", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued(&supervisor).await;
    let pending = env.wait_pending(&run).await;
    // Alcance: el comando que informa el agente.
    assert_eq!(pending.scope.as_deref(), Some("touch nota.txt"));
    assert!(pending.can_allow);
    let worktree = env.worktree(&run).await;
    assert!(!worktree.join("nota.txt").exists());
    env.decide_pending(&run, ApprovalDecision::AllowOnce).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    assert!(received(&log).contains(&"permiso: permitir-1".to_owned()));
    assert!(worktree.join("nota.txt").exists());
    assert_eq!(detail.approvals[0].state, ApprovalState::Allowed);
    assert!(detail.approvals[0].delivered);
    // Pasó por «esperando aprobación» y volvió a «en ejecución».
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
            RunState::WaitingApproval,
            RunState::Running,
            RunState::Completed
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_while_waiting_answers_cancelled_and_nothing_runs() {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["approval", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued(&supervisor).await;
    let pending = env.wait_pending(&run).await;
    let worktree = env.worktree(&run).await;
    env.cancel(&run).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Cancelled, "{:?}", detail.run);
    assert!(received(&log).contains(&"permiso: cancelled".to_owned()));
    assert!(!worktree.join("nota.txt").exists());
    let approval = detail
        .approvals
        .iter()
        .find(|a| a.id == pending.id)
        .unwrap();
    assert_eq!(approval.state, ApprovalState::Cancelled);
}

/// Carrera aprobar/cancelar: la persona permite y cancela antes de que el supervisor entregue
/// la decisión. La aprobación no se aplica nunca.
#[tokio::test(flavor = "multi_thread")]
async fn an_allow_followed_by_a_cancel_is_never_applied() {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["approval", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued(&supervisor).await;
    env.wait_pending(&run).await;
    let worktree = env.worktree(&run).await;
    // Sin `wake` entre medias: la decisión aún no se ha entregado al cancelar.
    env.decide_pending(&run, ApprovalDecision::AllowOnce).await;
    env.cancel(&run).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Cancelled, "{:?}", detail.run);
    assert!(received(&log).contains(&"permiso: cancelled".to_owned()));
    assert!(!worktree.join("nota.txt").exists());
    assert_eq!(detail.approvals[0].state, ApprovalState::Cancelled);
    assert!(!detail.approvals[0].delivered);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unanswered_request_expires_as_a_rejection() {
    let clock = Arc::new(ManualClock::new(1_794_744_000_000));
    let env = Env::with_clock(clock.clone()).await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["approval", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued(&supervisor).await;
    env.wait_pending(&run).await;
    clock.advance(Duration::from_millis(
        u64::try_from(APPROVAL_TTL_MS).unwrap(),
    ));
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    assert!(received(&log).contains(&"permiso: rechazar-1".to_owned()));
    assert_eq!(detail.approvals[0].state, ApprovalState::Expired);
    assert!(
        env.agent_lines(&run, RunOutputKind::Permission)
            .iter()
            .any(|l| l.contains("caducó sin respuesta"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_can_withdraw_a_pending_request() {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["approval_withdraw", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued(&supervisor).await;
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    assert!(received(&log).contains(&"ok: retirada respondida con -32800".to_owned()));
    assert_eq!(detail.approvals.len(), 1);
    assert_eq!(detail.approvals[0].state, ApprovalState::Withdrawn);
    assert_eq!(detail.approvals[0].scope.as_deref(), Some("src/main.rs"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_system_notification_only_when_enabled_and_without_details() {
    let env = Env::new().await;
    let notices: Arc<std::sync::Mutex<Vec<ApprovalNotice>>> = Arc::default();
    let sink = notices.clone();
    let mut config = env.config(&["approval"]);
    config.approval_notifier = Some(ApprovalNotifier(Arc::new(move |notice| {
        sink.lock().unwrap().push(notice);
    })));
    let supervisor = env.supervisor(config).await;
    // Desactivada (por defecto): nada.
    let first = env.queued(&supervisor).await;
    env.decide_pending(&first, ApprovalDecision::Reject).await;
    supervisor.wake();
    env.wait_terminal(&first).await;
    assert!(notices.lock().unwrap().is_empty());
    // Activada: una, solo con el número de la historia.
    env.set_settings(Some(env.adapter.to_str().unwrap().to_owned()), true)
        .await;
    let second = env.queued(&supervisor).await;
    env.decide_pending(&second, ApprovalDecision::Reject).await;
    supervisor.wake();
    env.wait_terminal(&second).await;
    supervisor.shutdown().await;
    let number =
        kanbania_core::features::stories::get_story(&env.store, &env.project.id, &second.story_id)
            .await
            .unwrap()
            .number;
    assert_eq!(
        *notices.lock().unwrap(),
        [ApprovalNotice {
            story_number: number
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn disconnect_mid_turn_fails() {
    let env = Env::new().await;
    let (run, detail) = env.scenario(&["disconnect"]).await;
    assert_failed(&detail, "El agente se desconectó antes de terminar.");
    assert_eq!(sessions(&detail).len(), 1);
    // Lo pendiente se emite igualmente.
    assert_eq!(
        env.agent_lines(&run, RunOutputKind::Message),
        ["Respuesta a medias"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_messages_fail_with_protocol_error() {
    let env = Env::new().await;
    for scenario in ["malformed", "oversize", "bad_response", "cancelled_unasked"] {
        let (run, detail) = env.scenario(&[scenario]).await;
        assert_failed(&detail, "El agente envió una respuesta ACP no válida.");
        assert!(
            !env.texts(&run, RunOutputStream::System).is_empty(),
            "{scenario}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_or_abandoned_mode_fails_closed() {
    let env = Env::new().await;
    let reason = "El agente no permite fijar un modo que pida permiso antes de actuar: no se \
                  ejecuta.";
    let (_, detail) = env.scenario(&["no_modes"]).await;
    assert_failed(&detail, reason);
    assert!(
        sessions(&detail).is_empty(),
        "sin modo no hay sesión registrada"
    );
    let (_, detail) = env.scenario(&["set_mode_error"]).await;
    assert_failed(&detail, reason);
    let (run, detail) = env.scenario(&["config_mode_change"]).await;
    assert_failed(&detail, reason);
    assert!(
        env.texts(&run, RunOutputStream::System)
            .iter()
            .any(|l| l.contains("cambió por su cuenta al modo «acceptEdits»"))
    );
    let (run, detail) = env.scenario(&["mode_change"]).await;
    assert_failed(&detail, reason);
    assert!(
        env.texts(&run, RunOutputStream::System)
            .iter()
            .any(|l| l.contains("cambió por su cuenta al modo «bypassPermissions»"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn version_and_auth_failures() {
    let env = Env::new().await;
    let (_, detail) = env.scenario(&["version2"]).await;
    assert_failed(
        &detail,
        "El agente no habla una versión de ACP compatible (se requiere la 1).",
    );
    assert!(sessions(&detail).is_empty());
    let (run, detail) = env.scenario(&["auth"]).await;
    assert_failed(
        &detail,
        "El agente necesita iniciar sesión: abre Claude Code en una terminal y usa /login.",
    );
    assert_nothing_hidden(&env.raw_output(&run));
    // Anunciar `authMethods` no basta: solo el código -32000 es «inicia sesión».
    let (_, detail) = env.scenario(&["auth_other_error"]).await;
    assert_failed(&detail, REQUEST_FAILED);
}

#[tokio::test(flavor = "multi_thread")]
async fn refusal_and_limits_fail() {
    let env = Env::new().await;
    let (_, detail) = env.scenario(&["refusal"]).await;
    assert_failed(&detail, "El agente rechazó la tarea.");
    let (_, detail) = env.scenario(&["max_tokens"]).await;
    assert_failed(
        &detail,
        "El agente alcanzó su límite de tokens o de turnos.",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_with_agent_response() {
    let env = Env::new().await;
    let supervisor = env.supervisor(env.config(&["cancel"])).await;
    let run = env.queued(&supervisor).await;
    env.wait_for(&run, &[RunState::Running]).await;
    // Espera a que el turno esté en curso (su primer mensaje solo se emite al terminar el
    // grupo: se espera al evento de sesión).
    let started = Instant::now();
    while sessions(&env.detail(&run).await).is_empty() {
        assert!(started.elapsed() < Duration::from_secs(30));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    env.cancel(&run).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Cancelled, "{:?}", detail.run);
    let system = env.texts(&run, RunOutputStream::System);
    assert!(
        system
            .iter()
            .any(|l| l == "El agente respondió a la cancelación (stopReason: cancelled)."),
        "{system:?}"
    );
    assert_eq!(
        env.agent_lines(&run, RunOutputKind::Message),
        ["Trabajando…"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_without_agent_response_kills_the_tree() {
    let env = Env::new().await;
    let supervisor = env.supervisor(env.config(&["ignore_cancel"])).await;
    let run = env.queued(&supervisor).await;
    let started = Instant::now();
    while sessions(&env.detail(&run).await).is_empty() {
        assert!(started.elapsed() < Duration::from_secs(30));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let cancelled_at = Instant::now();
    env.cancel(&run).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    let elapsed = cancelled_at.elapsed();
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Cancelled, "{:?}", detail.run);
    // Espera la respuesta (2 s en esta prueba) y después termina el árbol.
    assert!(elapsed >= Duration::from_secs(2), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(20), "{elapsed:?}");
    assert!(
        env.texts(&run, RunOutputStream::System)
            .iter()
            .any(|l| l.contains("no respondió a la cancelación a tiempo"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_cancels_the_turn() {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let mut config = env.config(&["cancel", "--log", log.to_str().unwrap()]);
    config.timeout_override = Some(Duration::from_secs(4));
    let supervisor = env.supervisor(config).await;
    let run = env.queued(&supervisor).await;
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_failed(&detail, "Superó el tiempo máximo (30 min).");
    assert_eq!(sessions(&detail).len(), 1);
    // Se pidió cancelar el turno (lo dice la app y lo recibió el agente).
    assert!(
        env.texts(&run, RunOutputStream::System)
            .iter()
            .any(|l| l.starts_with("Se pidió al agente cancelar el turno (session/cancel)"))
    );
    assert!(received(&log).contains(&"session/cancel".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn flood_output_is_bounded() {
    let env = Env::new().await;
    let (run, detail) = env.scenario(&["flood"]).await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    let view = read_run_output(&env.data_dir, &run.id).unwrap();
    assert!(view.lines.len() <= 500);
    assert!(view.earlier_dropped, "la rotación debió descartar salida");
    let files = std::fs::read_dir(env.data_dir.join("runs").join(&run.id))
        .unwrap()
        .count();
    assert!(files <= 4, "{files} archivos");
    for line in view.lines {
        assert!(line.text.len() <= 4096);
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn child_processes_die_with_the_tree() {
    let env = Env::new().await;
    let pids = env.base.join("pids del hijo.txt");
    let (_, detail) = env
        .scenario(&["child", "--pids", pids.to_str().unwrap()])
        .await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    let pid: i32 = std::fs::read_to_string(&pids)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(!alive_after_grace(pid).await, "el hijo {pid} sigue vivo");
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_adapter_fails_without_running() {
    let env = Env::new().await;
    // La ruta era válida al guardarla y desaparece antes de despachar.
    std::fs::remove_file(&env.adapter).unwrap();
    let (_, detail) = env.scenario(&["ok"]).await;
    assert_failed(
        &detail,
        "No se encontró el adaptador de Claude Code configurado o no es ejecutable.",
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn adapter_without_interpreter_is_missing() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new().await;
    // Como un script npm sin `node` en el PATH: sale con 127 sin hablar ACP.
    let script = env
        .base
        .join("adaptador con espacios")
        .join("sin interprete");
    std::fs::write(
        &script,
        "#!/bin/sh\necho 'env: node: No such file or directory' >&2\nexit 127\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    env.set_adapter(Some(script.to_str().unwrap().to_owned()))
        .await;
    let (run, detail) = env.scenario(&[]).await;
    assert_failed(
        &detail,
        "No se encontró el adaptador de Claude Code configurado o no es ejecutable.",
    );
    assert_eq!(
        env.texts(&run, RunOutputStream::Stderr),
        ["env: node: No such file or directory"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_reports_initialize_only() {
    let env = Env::new().await;
    let cwd = env.base.clone();
    let declared = probe_adapter(
        env.adapter.clone(),
        vec!["ok".into()],
        cwd.clone(),
        Vec::new(),
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    assert_eq!(declared.protocol_version, 1);
    assert_eq!(declared.name.as_deref(), Some("kanbania-fake-acp"));
    assert_eq!(
        declared.capabilities,
        ["loadSession", "image", "embeddedContext", "mcpHttp"]
    );
    let version2 = probe_adapter(
        env.adapter.clone(),
        vec!["version2".into()],
        cwd.clone(),
        Vec::new(),
        Duration::from_secs(20),
    )
    .await;
    assert_eq!(version2, Err(ProbeError::VersionUnsupported));
    let missing = probe_adapter(
        env.base.join("no existe"),
        Vec::new(),
        cwd.clone(),
        Vec::new(),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(missing, Err(ProbeError::NotFound));
    // Sin argumentos el agente falso imprime el uso y termina: desconexión.
    let quits = probe_adapter(
        env.adapter.clone(),
        Vec::new(),
        cwd,
        Vec::new(),
        Duration::from_secs(20),
    )
    .await;
    assert_eq!(quits, Err(ProbeError::Disconnected));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_before_the_session_kills_without_session_cancel() {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["slow_init", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued(&supervisor).await;
    env.wait_for(&run, &[RunState::Running]).await;
    // El falso tarda 30 s en responder a `initialize`.
    let started = Instant::now();
    while !received(&log).contains(&"initialize".to_owned()) {
        assert!(started.elapsed() < Duration::from_secs(30));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    env.cancel(&run).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Cancelled, "{:?}", detail.run);
    assert!(started.elapsed() < Duration::from_secs(20));
    assert!(sessions(&detail).is_empty());
    // Sin sesión no se negocia nada: el árbol se mata directamente.
    assert_eq!(received(&log), ["arranque", "initialize"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn negotiation_without_response_fails() {
    let env = Env::new().await;
    let mut config = env.config(&["hang_init"]);
    config.acp_limits.request_timeout = Duration::from_secs(1);
    let supervisor = env.supervisor(config).await;
    let run = env.queued(&supervisor).await;
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_failed(&detail, REQUEST_FAILED);
    assert!(
        env.texts(&run, RunOutputStream::System)
            .iter()
            .any(|l| l == "El agente no respondió a «initialize» en 1 s.")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn permission_during_cancellation_is_answered_cancelled() {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["permission_on_cancel", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued(&supervisor).await;
    wait_session(&env, &run).await;
    env.cancel(&run).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Cancelled, "{:?}", detail.run);
    let log = received(&log);
    assert!(log.contains(&"ok: permiso cancelado".to_owned()), "{log:?}");
    assert_eq!(
        env.agent_lines(&run, RunOutputKind::Permission),
        [
            "Permiso pedido para «Borrar todo»: respondido como cancelado (se estaba cancelando el turno)."
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_during_a_turn_interrupts() {
    let env = Env::new().await;
    let supervisor = env.supervisor(env.config(&["cancel"])).await;
    let run = env.queued(&supervisor).await;
    wait_session(&env, &run).await;
    let started = Instant::now();
    supervisor.shutdown().await;
    // Margen de HU-018: gracia + 10 s.
    assert!(
        started.elapsed() < Duration::from_secs(11),
        "{:?}",
        started.elapsed()
    );
    let detail = env.detail(&run).await;
    assert_eq!(detail.run.state, RunState::Interrupted, "{:?}", detail.run);
    assert_eq!(detail.run.end_reason.as_deref(), Some("Se cerró la app."));
}

#[tokio::test(flavor = "multi_thread")]
async fn response_to_an_unknown_id_is_a_protocol_error() {
    let env = Env::new().await;
    let (_, detail) = env.scenario(&["unknown_id"]).await;
    assert_failed(&detail, "El agente envió una respuesta ACP no válida.");
}

/// Decisión: las actualizaciones de otra sesión se ignoran (una nota `sys` única), también un
/// cambio de modo; un cambio de modo **sin** `sessionId` (campo obligatorio) falla cerrado.
#[tokio::test(flavor = "multi_thread")]
async fn updates_of_other_sessions_are_ignored_but_mode_without_session_fails() {
    let env = Env::new().await;
    let (run, detail) = env.scenario(&["other_session"]).await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    assert_eq!(env.agent_lines(&run, RunOutputKind::Message), ["Hecho."]);
    assert!(!env.raw_output(&run).contains("INTRUSO"));
    let notes: Vec<_> = env
        .texts(&run, RunOutputStream::System)
        .into_iter()
        .filter(|l| l.contains("sesión que no es la del intento"))
        .collect();
    assert_eq!(notes.len(), 1);

    let (_, detail) = env.scenario(&["mode_no_session"]).await;
    assert_failed(&detail, "El agente envió una respuesta ACP no válida.");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn adapter_without_execute_permission_fails_to_spawn() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new().await;
    // Era ejecutable al guardarlo; después pierde el permiso.
    std::fs::set_permissions(&env.adapter, std::fs::Permissions::from_mode(0o644)).unwrap();
    let (_, detail) = env.scenario(&["ok"]).await;
    assert_failed(
        &detail,
        "No se pudo lanzar el adaptador de Claude Code configurado. Consulta el registro del \
         intento.",
    );
}

/// `PermissionPolicy::RejectAll` (por defecto, la de las pruebas reales sin persona): rechaza en
/// el acto con `reject_once` (o `cancelled` sin esa opción), sin preguntar.
#[tokio::test(flavor = "multi_thread")]
async fn reject_all_policy_answers_without_asking() {
    let env = Env::new().await;
    let repo = env.base.join("repositorio con espacios");
    let output_dir = env.base.join("salida directa");
    let log = OutputLog::create(&output_dir).unwrap();
    let spec = ProcessSpec {
        program: env.adapter.clone(),
        args: vec!["permission".into()],
        cwd: repo.clone(),
        extra_env: Vec::new(),
        timeout: Duration::from_secs(60),
        grace: Duration::from_secs(1),
    };
    let (process, io) = RunningProcess::spawn_protocol(spec, log).unwrap();
    let deadline = process.deadline();
    let limits = AcpLimits::default();
    let mut client = AcpClient::new(io, limits, None);
    let (_keep, mut token) = cancel_pair();
    let declared = client.initialize(&mut token, deadline).await.unwrap();
    client
        .open_session(&repo, &declared, &CLAUDE_CODE_PROFILE, &mut token, deadline)
        .await
        .unwrap();
    // El agente falso termina con error si no recibe `reject_once` y después `cancelled`.
    let ended = client.prompt("Haz algo", &mut token, deadline).await;
    client.close_input();
    drop(client);
    let (report, log) = process.close(limits.close_wait).await;
    drop(log);
    assert_eq!(ended, Ok(TurnEnd::EndTurn), "{report:?}");
    let permissions: Vec<String> = read_tail(&output_dir, 100)
        .unwrap()
        .lines
        .into_iter()
        .filter(|l| l.kind == Some(AgentKind::Permission))
        .map(|l| l.text)
        .collect();
    assert_eq!(permissions.len(), 2, "{permissions:?}");
    assert!(permissions[0].contains("rechazado una vez por la política de la app"));
    assert!(permissions[1].contains("respondido como cancelado"));
}

/// Dos intentos a la vez: cada decisión llega a la sesión de su intento.
#[tokio::test(flavor = "multi_thread")]
async fn decisions_reach_the_right_session_with_two_runs() {
    let env = Env::new().await;
    let supervisor = env.supervisor(env.config(&["approval"])).await;
    let first = env.queued(&supervisor).await;
    let second = env.queued(&supervisor).await;
    env.wait_pending(&first).await;
    env.wait_pending(&second).await;
    let (first_tree, second_tree) = (env.worktree(&first).await, env.worktree(&second).await);
    env.decide_pending(&second, ApprovalDecision::Reject).await;
    env.decide_pending(&first, ApprovalDecision::AllowOnce)
        .await;
    supervisor.wake();
    let first_detail = env.wait_terminal(&first).await;
    let second_detail = env.wait_terminal(&second).await;
    supervisor.shutdown().await;
    assert_eq!(first_detail.run.state, RunState::Completed);
    assert_eq!(second_detail.run.state, RunState::Completed);
    assert!(first_tree.join("nota.txt").exists());
    assert!(!second_tree.join("nota.txt").exists());
    assert_eq!(first_detail.approvals[0].state, ApprovalState::Allowed);
    assert_eq!(second_detail.approvals[0].state, ApprovalState::Rejected);
}

/// La caducidad no depende de que alguien despierte al supervisor (ni del reloj monotónico:
/// equipo suspendido): el broker revisa sus pendientes cada `poll_interval`.
#[tokio::test(flavor = "multi_thread")]
async fn a_request_expires_without_any_wake() {
    let clock = Arc::new(ManualClock::new(1_794_744_000_000));
    let env = Env::with_clock(clock.clone()).await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["approval", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued(&supervisor).await;
    env.wait_pending(&run).await;
    clock.advance(Duration::from_millis(
        u64::try_from(APPROVAL_TTL_MS).unwrap(),
    ));
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    assert!(received(&log).contains(&"permiso: rechazar-1".to_owned()));
    assert_eq!(detail.approvals[0].state, ApprovalState::Expired);
}

/// Si el tiempo máximo corta el turno con una petición pendiente, el cliente responde
/// «cancelado» por su cuenta y la petición queda cerrada en la base: una decisión posterior no
/// puede constar como entregada.
#[tokio::test(flavor = "multi_thread")]
async fn a_timeout_while_waiting_closes_the_request() {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let mut config = env.config(&["approval", "--log", log.to_str().unwrap()]);
    config.timeout_override = Some(Duration::from_secs(3));
    let supervisor = env.supervisor(config).await;
    let run = env.queued(&supervisor).await;
    let pending = env.wait_pending(&run).await;
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Failed, "{:?}", detail.run);
    assert!(received(&log).contains(&"permiso: cancelled".to_owned()));
    let approval = detail
        .approvals
        .iter()
        .find(|a| a.id == pending.id)
        .unwrap();
    assert_eq!(approval.state, ApprovalState::Cancelled);
    assert!(!approval.delivered);
}

// ---------------------------------------------------------------------------------------------
// Suite contractual compartida (HU-021, CA-3): los mismos casos con el perfil de Claude Code y
// con el de Codex, cada uno con la variante del agente falso que imita su adaptador (la de Codex
// se activa por el entorno que fija el supervisor: `CODEX_HOME`). Prueba el contrato de la app
// con cada perfil, no la compatibilidad con los proveedores reales (eso es `acp_real.rs`).
// ---------------------------------------------------------------------------------------------

/// Lo que cambia entre perfiles en la suite compartida.
#[derive(Debug, Clone, Copy)]
struct Flavor {
    executor: RunExecutor,
    agent: AcpAgent,
    /// Modo que fija la app (perfil).
    mode: &'static str,
    /// `optionId` que la app debe elegir para permitir una vez y para rechazar.
    allow: &'static str,
    reject: &'static str,
    /// Modelo que declara la variante del agente falso.
    model: Option<&'static str>,
    /// Modo al que se escapa la variante en `mode_change` / `config_mode_change`.
    escapes: [&'static str; 2],
    auth_reason: &'static str,
    missing_reason: &'static str,
}

const CLAUDE: Flavor = Flavor {
    executor: RunExecutor::ClaudeAcp,
    agent: AcpAgent::ClaudeCode,
    mode: "default",
    allow: "permitir-1",
    reject: "rechazar-1",
    model: None,
    escapes: ["bypassPermissions", "acceptEdits"],
    auth_reason: "El agente necesita iniciar sesión: abre Claude Code en una terminal y usa /login.",
    missing_reason: "No se encontró el adaptador de Claude Code configurado o no es ejecutable.",
};

const CODEX: Flavor = Flavor {
    executor: RunExecutor::CodexAcp,
    agent: AcpAgent::Codex,
    mode: "read-only",
    allow: "allow_once",
    // `cancel` va antes en las opciones del agente falso: la app debe preferir `decline`.
    reject: "decline",
    model: Some("gpt-falso-1[medium]"),
    escapes: ["agent-full-access", "agent"],
    auth_reason: "Codex necesita iniciar sesión en la carpeta de Codex de la app: usa el comando \
                  que muestra Sistema → Ejecuciones.",
    missing_reason: "No se encontró el adaptador de Codex configurado o no es ejecutable.",
};

const MODE_REASON: &str =
    "El agente no permite fijar un modo que pida permiso antes de actuar: no se ejecuta.";

async fn contract_ends_turn(f: Flavor) {
    let env = Env::new().await;
    let (run, detail) = env.scenario_as(f.executor, &["ok"]).await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    assert_eq!(
        detail.run.end_reason.as_deref(),
        Some("El agente terminó su turno.")
    );
    let sessions = sessions(&detail);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].mode.as_deref(), Some(f.mode));
    assert_eq!(sessions[0].model.as_deref(), f.model);
    assert_eq!(
        env.agent_lines(&run, RunOutputKind::Message),
        ["He leído el README.", "Fin."]
    );
    let system = env.texts(&run, RunOutputStream::System);
    let claude_note = system.iter().any(|l| l == ISOLATION_NOTE);
    let codex_note = system.iter().any(|l| l == CODEX_HOME_NOTE);
    assert_eq!(
        (claude_note, codex_note),
        (f.agent == AcpAgent::ClaudeCode, f.agent == AcpAgent::Codex),
        "{system:?}"
    );
    assert!(system[0].starts_with(&format!("{} vía ACP.", f.agent.label())));
    let raw = env.raw_output(&run);
    assert_nothing_hidden(&raw);
    // Ninguna ruta del directorio de datos en las líneas de la app (tampoco `CODEX_HOME`).
    assert!(
        !system.iter().any(|l| l.contains("codex-home")),
        "{system:?}"
    );
}

async fn contract_allow_once_runs_the_action(f: Flavor) {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["approval", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued_as(&supervisor, f.executor).await;
    let pending = env.wait_pending(&run).await;
    assert!(pending.can_allow);
    assert_eq!(pending.scope.as_deref(), Some("touch nota.txt"));
    let worktree = env.worktree(&run).await;
    env.decide_pending(&run, ApprovalDecision::AllowOnce).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    assert!(received(&log).contains(&format!("permiso: {}", f.allow)));
    assert!(worktree.join("nota.txt").exists());
    assert_eq!(detail.approvals[0].state, ApprovalState::Allowed);
}

async fn contract_reject_keeps_the_turn(f: Flavor) {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["approval", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued_as(&supervisor, f.executor).await;
    env.wait_pending(&run).await;
    let worktree = env.worktree(&run).await;
    env.decide_pending(&run, ApprovalDecision::Reject).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    assert_eq!(
        detail.run.end_reason.as_deref(),
        Some("El agente terminó su turno.")
    );
    assert!(
        received(&log).contains(&format!("permiso: {}", f.reject)),
        "{:?}",
        received(&log)
    );
    assert!(!worktree.join("nota.txt").exists());
    assert_eq!(detail.approvals[0].state, ApprovalState::Rejected);
}

async fn contract_cancel_while_waiting(f: Flavor) {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["approval", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued_as(&supervisor, f.executor).await;
    env.wait_pending(&run).await;
    let worktree = env.worktree(&run).await;
    env.cancel(&run).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Cancelled, "{:?}", detail.run);
    assert!(received(&log).contains(&"permiso: cancelled".to_owned()));
    assert!(!worktree.join("nota.txt").exists());
    assert_eq!(detail.approvals[0].state, ApprovalState::Cancelled);
}

async fn contract_mode_is_enforced(f: Flavor) {
    let env = Env::new().await;
    for (scenario, escaped) in [
        ("mode_change", f.escapes[0]),
        ("config_mode_change", f.escapes[1]),
    ] {
        let (run, detail) = env.scenario_as(f.executor, &[scenario]).await;
        assert_failed(&detail, MODE_REASON);
        assert!(
            env.texts(&run, RunOutputStream::System)
                .iter()
                .any(|l| l.contains(&format!("cambió por su cuenta al modo «{escaped}»"))),
            "{scenario}"
        );
    }
    let (_, detail) = env.scenario_as(f.executor, &["no_modes"]).await;
    assert_failed(&detail, MODE_REASON);
}

async fn contract_auth_required(f: Flavor) {
    let env = Env::new().await;
    let (run, detail) = env.scenario_as(f.executor, &["auth"]).await;
    assert_failed(&detail, f.auth_reason);
    assert!(sessions(&detail).is_empty());
    assert_nothing_hidden(&env.raw_output(&run));
    let (_, detail) = env.scenario_as(f.executor, &["auth_other_error"]).await;
    assert_failed(&detail, REQUEST_FAILED);
}

async fn contract_unrequested_cancelled_is_a_protocol_error(f: Flavor) {
    let env = Env::new().await;
    let (_, detail) = env.scenario_as(f.executor, &["cancelled_unasked"]).await;
    assert_failed(&detail, "El agente envió una respuesta ACP no válida.");
}

/// Responder `cancelled` (sin opción de rechazo) no cuenta como rechazo que termina el turno:
/// un `stopReason: cancelled` posterior sigue siendo error de protocolo.
async fn contract_cancelled_answer_is_not_a_turn_ending_rejection(f: Flavor) {
    let env = Env::new().await;
    let (_, detail) = env
        .scenario_as(f.executor, &["cancelled_after_cancelled_answer"])
        .await;
    assert_failed(&detail, "El agente envió una respuesta ACP no válida.");
}

async fn contract_missing_adapter(f: Flavor) {
    let env = Env::new().await;
    std::fs::remove_file(&env.adapter).unwrap();
    let (_, detail) = env.scenario_as(f.executor, &["ok"]).await;
    assert_failed(&detail, f.missing_reason);
}

/// Un módulo por perfil con los mismos casos.
macro_rules! shared_contract {
    ($($case:ident),* $(,)?) => {
        mod shared_claude_code {
            $(
                #[tokio::test(flavor = "multi_thread")]
                async fn $case() {
                    super::$case(super::CLAUDE).await;
                }
            )*
        }
        mod shared_codex {
            $(
                #[tokio::test(flavor = "multi_thread")]
                async fn $case() {
                    super::$case(super::CODEX).await;
                }
            )*
        }
    };
}

shared_contract!(
    contract_ends_turn,
    contract_allow_once_runs_the_action,
    contract_reject_keeps_the_turn,
    contract_cancel_while_waiting,
    contract_mode_is_enforced,
    contract_auth_required,
    contract_unrequested_cancelled_is_a_protocol_error,
    contract_missing_adapter,
    contract_cancelled_answer_is_not_a_turn_ending_rejection,
);

// ---------------------------------------------------------------------------------------------
// Propio de Codex (HU-021).
// ---------------------------------------------------------------------------------------------

const CONFIG_IN_REPOSITORY: &str = "El repositorio contiene configuración de Codex (.codex/), que \
                                    podría saltarse los permisos o lanzar programas: la app no \
                                    ejecuta Codex en él.";

/// `.codex/` en el repositorio: Codex la cargaría al abrir la sesión (servidores MCP que se
/// lanzan antes de cualquier permiso). No se lanza el adaptador (su registro ni existe). A
/// Claude Code no le afecta.
#[tokio::test(flavor = "multi_thread")]
async fn codex_config_in_the_repository_is_refused_before_spawning() {
    let env = Env::new().await;
    env.commit_file(
        ".codex/config.toml",
        "[mcp_servers.testigo]\ncommand = \"/no/existe/servidor\"\n",
    );
    let log = env.base.join("registro del falso.txt");
    let (run, detail) = env
        .scenario_as(
            RunExecutor::CodexAcp,
            &["ok", "--log", log.to_str().unwrap()],
        )
        .await;
    assert_failed(&detail, CONFIG_IN_REPOSITORY);
    assert!(!log.exists(), "se lanzó el adaptador: {:?}", received(&log));
    assert!(sessions(&detail).is_empty());
    assert!(
        env.texts(&run, RunOutputStream::System)
            .iter()
            .any(|l| l.contains("contiene configuración del agente (.codex)"))
    );
    // El mismo repositorio con Claude Code sí se ejecuta.
    let (_, detail) = env.scenario_as(RunExecutor::ClaudeAcp, &["ok"]).await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
}

/// `.codex` en cualquier forma (aquí, un enlace roto) también se rechaza.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn codex_config_as_a_dangling_link_is_refused() {
    let env = Env::new().await;
    let repo = env.repo();
    std::os::unix::fs::symlink("/no/existe/codex", repo.join(".codex")).unwrap();
    git(&repo, &["add", ".codex"]);
    git(&repo, &["commit", "-q", "-m", "enlace"]);
    let log = env.base.join("registro del falso.txt");
    let (_, detail) = env
        .scenario_as(
            RunExecutor::CodexAcp,
            &["ok", "--log", log.to_str().unwrap()],
        )
        .await;
    assert_failed(&detail, CONFIG_IN_REPOSITORY);
    assert!(!log.exists());
}

/// Entorno de Codex: su carpeta propia (0700) en el directorio de datos, modo inicial, sin
/// navegador y `CODEX_CONFIG`; nada más que la lista de adaptadores. El agente falso además
/// termina con error si recibe una variable prohibida o un valor distinto.
#[tokio::test(flavor = "multi_thread")]
async fn codex_gets_its_own_home_and_a_minimal_environment() {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let (_, detail) = env
        .scenario_as(
            RunExecutor::CodexAcp,
            &["ok", "--log", log.to_str().unwrap()],
        )
        .await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    let lines = received(&log);
    let home = env.data_dir.join("codex-home");
    assert!(
        lines.contains(&format!("codex_home: {}", home.display())),
        "{lines:?}"
    );
    // `HOME` es la carpeta personal de Codex dentro de su carpeta, nunca la del usuario (el
    // agente falso también comprueba `USERPROFILE` en Windows y `remote_plugin: false`).
    let personal = home.join("home");
    assert!(
        lines.contains(&format!("home: {}", personal.display())),
        "{lines:?}"
    );
    assert_ne!(
        Some(personal.as_os_str()),
        std::env::var_os("HOME").as_deref()
    );
    assert!(personal.is_dir());
    assert!(home.is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&home).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
    let names: Vec<String> = lines
        .iter()
        .find_map(|l| l.strip_prefix("entorno: "))
        .unwrap()
        .split(',')
        .map(str::to_owned)
        .collect();
    for required in [
        "CODEX_HOME",
        "HOME",
        "INITIAL_AGENT_MODE",
        "NO_BROWSER",
        "CODEX_CONFIG",
    ] {
        assert!(names.contains(&required.to_owned()), "{names:?}");
    }
    let mut allowed: Vec<&str> = kanbania_runner::adapter_env_names();
    allowed.extend([
        "CODEX_HOME",
        "INITIAL_AGENT_MODE",
        "NO_BROWSER",
        "CODEX_CONFIG",
    ]);
    for name in &names {
        // macOS añade `__CF_USER_TEXT_ENCODING` a todo proceso.
        assert!(
            allowed.contains(&name.as_str()) || name.starts_with("__CF"),
            "variable no permitida: {name}"
        );
        assert!(!kanbania_runner::is_adapter_forbidden(
            std::ffi::OsStr::new(name)
        ));
    }
}

/// Permisos adicionales de Codex: sus únicos `allow_once` valen para todo el turno; la app no
/// los elige nunca (la persona solo puede rechazar) y rechaza con `reject_permissions`.
#[tokio::test(flavor = "multi_thread")]
async fn codex_turn_wide_allow_options_are_never_chosen() {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["permissions_turn", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued_as(&supervisor, RunExecutor::CodexAcp).await;
    let pending = env.wait_pending(&run).await;
    assert!(!pending.can_allow);
    env.decide_pending(&run, ApprovalDecision::Reject).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    assert!(received(&log).contains(&"permiso: reject_permissions".to_owned()));
    assert!(
        env.agent_lines(&run, RunOutputKind::Permission)[0]
            .contains("la app solo puede rechazarlo")
    );
}

/// Ediciones de Codex: la única opción de rechazo (`cancel`) termina el turno. El agente
/// responde `stopReason: cancelled` sin que la app cancelara: `completed` con su propio motivo,
/// no error de protocolo.
#[tokio::test(flavor = "multi_thread")]
async fn codex_edit_rejection_stops_the_turn() {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["edit_reject", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued_as(&supervisor, RunExecutor::CodexAcp).await;
    let pending = env.wait_pending(&run).await;
    assert!(pending.can_allow);
    assert_eq!(pending.scope.as_deref(), Some("nota.txt"));
    let worktree = env.worktree(&run).await;
    env.decide_pending(&run, ApprovalDecision::Reject).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    assert_eq!(
        detail.run.end_reason.as_deref(),
        Some("El agente detuvo su turno tras un permiso rechazado.")
    );
    assert!(received(&log).contains(&"permiso: cancel".to_owned()));
    assert!(!worktree.join("nota.txt").exists());
    let permissions = env.agent_lines(&run, RunOutputKind::Permission);
    assert!(
        permissions[0].contains("el agente termina su turno"),
        "{permissions:?}"
    );
}

/// Igual al caducar: se responde el rechazo (`cancel`) y el turno termina.
#[tokio::test(flavor = "multi_thread")]
async fn codex_expired_edit_stops_the_turn() {
    let clock = Arc::new(ManualClock::new(1_794_744_000_000));
    let env = Env::with_clock(clock.clone()).await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["edit_reject", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued_as(&supervisor, RunExecutor::CodexAcp).await;
    env.wait_pending(&run).await;
    clock.advance(Duration::from_millis(
        u64::try_from(APPROVAL_TTL_MS).unwrap(),
    ));
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(
        detail.run.end_reason.as_deref(),
        Some("El agente detuvo su turno tras un permiso rechazado.")
    );
    assert!(received(&log).contains(&"permiso: cancel".to_owned()));
    assert_eq!(detail.approvals[0].state, ApprovalState::Expired);
}

/// «Comprobar adaptador» con el entorno de Codex: el agente falso responde como Codex.
#[tokio::test(flavor = "multi_thread")]
async fn probe_with_the_codex_environment() {
    let env = Env::new().await;
    let home = env.data_dir.join("codex-home");
    kanbania_runner::acp::ensure_codex_home(&home).unwrap();
    let declared = probe_adapter(
        env.adapter.clone(),
        vec!["ok".into()],
        env.base.clone(),
        codex_env(&home),
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    assert_eq!(
        declared.title.as_deref(),
        Some("Agente ACP falso tipo Codex (pruebas)")
    );
    assert_eq!(declared.auth_methods, 2);
}

/// Carrera: la persona rechaza una edición de Codex (`cancel`, que terminaría el turno) y
/// cancela el intento antes de que se entregue el rechazo. Determinista: cancelar cierra la
/// decisión sin entregar (núcleo), así que el agente solo recibe `session/cancel` y `cancelled`.
#[tokio::test(flavor = "multi_thread")]
async fn codex_rejection_followed_by_a_cancel_ends_cancelled() {
    let env = Env::new().await;
    let log = env.base.join("registro del falso.txt");
    let supervisor = env
        .supervisor(env.config(&["edit_reject", "--log", log.to_str().unwrap()]))
        .await;
    let run = env.queued_as(&supervisor, RunExecutor::CodexAcp).await;
    env.wait_pending(&run).await;
    // Sin `wake` entre medias: el rechazo aún no se ha entregado al cancelar.
    env.decide_pending(&run, ApprovalDecision::Reject).await;
    env.cancel(&run).await;
    supervisor.wake();
    let detail = env.wait_terminal(&run).await;
    supervisor.shutdown().await;
    assert_eq!(detail.run.state, RunState::Cancelled, "{:?}", detail.run);
    assert!(received(&log).contains(&"permiso: cancelled".to_owned()));
    assert!(!received(&log).contains(&"permiso: cancel".to_owned()));
    assert!(!detail.approvals[0].delivered);
}

const CONFIG_IN_CODEX_HOME: &str = "La carpeta de Codex de la app contiene configuración \
                                    (config.toml, reglas, AGENTS.md…) que podría saltarse los \
                                    permisos: bórrala o vuelve a crear la carpeta e inicia sesión \
                                    de nuevo.";

/// Configuración en la carpeta de Codex de la app: cada entrada de la lista (como archivo, como
/// carpeta o como enlace roto) y skills propias impiden lanzar el adaptador. Las skills del
/// sistema que crea Codex solo (`skills/.system`) y `auth.json` no.
#[tokio::test(flavor = "multi_thread")]
async fn codex_plugin_caches_are_cleared_before_each_launch() {
    // Codex 2.5.0 descarga los plugins remotos de la cuenta en su carpeta en cada sesión: la
    // app los borra antes de lanzar (sin seguir enlaces) y el intento sigue adelante.
    let env = Env::new().await;
    let home = env.data_dir.join("codex-home");
    kanbania_runner::acp::ensure_codex_home(&home).unwrap();
    std::fs::write(home.join("auth.json"), "{}").unwrap();
    let plugin = home.join("plugins/cache/openai-curated-remote/sites");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join("SKILL.md"), "x").unwrap();
    std::fs::create_dir_all(home.join("cache")).unwrap();
    let outside = env.base.join("fuera de codex-home");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("testigo.txt"), "x").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, home.join("cache/remote_plugin_catalog")).unwrap();
    #[cfg(not(unix))]
    std::fs::create_dir_all(home.join("cache/remote_plugin_catalog")).unwrap();

    let log = env.base.join("registro del falso (plugins).txt");
    let args = ["ok", "--log", log.to_str().unwrap()];
    let (_, detail) = env.scenario_as(RunExecutor::CodexAcp, &args).await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    for name in kanbania_runner::acp::CODEX_PLUGIN_CACHES {
        assert!(
            std::fs::symlink_metadata(home.join(name)).is_err(),
            "{name} sigue ahí"
        );
    }
    assert!(outside.join("testigo.txt").exists(), "se siguió el enlace");
    assert!(home.join("auth.json").exists(), "se borró la sesión");
}

#[tokio::test(flavor = "multi_thread")]
async fn codex_home_config_is_refused_before_spawning() {
    let env = Env::new().await;
    let home = env.data_dir.join("codex-home");
    kanbania_runner::acp::ensure_codex_home(&home).unwrap();
    std::fs::create_dir_all(home.join("skills").join(".system").join("imagegen")).unwrap();
    std::fs::write(home.join("auth.json"), "{}").unwrap();
    let log = env.base.join("registro del falso.txt");
    let args = ["ok", "--log", log.to_str().unwrap()];
    let (_, detail) = env.scenario_as(RunExecutor::CodexAcp, &args).await;
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    std::fs::remove_file(&log).unwrap();

    type Create = Box<dyn Fn(&Path)>;
    let mut cases: Vec<(String, Create)> = Vec::new();
    // También `home/.agents` y `home/.codex` en la carpeta personal de Codex.
    for name in kanbania_runner::acp::CODEX_HOME_DENYLIST
        .iter()
        .chain(kanbania_runner::acp::CODEX_PERSONAL_DENYLIST)
    {
        cases.push((
            format!("{name} (archivo)"),
            Box::new(move |home: &Path| std::fs::write(home.join(name), "x").unwrap()),
        ));
        cases.push((
            format!("{name} (carpeta)"),
            Box::new(move |home: &Path| std::fs::create_dir(home.join(name)).unwrap()),
        ));
        #[cfg(unix)]
        cases.push((
            format!("{name} (enlace roto)"),
            Box::new(move |home: &Path| {
                std::os::unix::fs::symlink("/no/existe", home.join(name)).unwrap();
            }),
        ));
    }
    cases.push((
        "skills propia".into(),
        Box::new(|home: &Path| std::fs::create_dir(home.join("skills").join("mia")).unwrap()),
    ));
    for (case, create) in cases {
        let snapshot: Vec<_> = std::fs::read_dir(&home)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        create(&home);
        let (run, detail) = env.scenario_as(RunExecutor::CodexAcp, &args).await;
        assert_failed(&detail, CONFIG_IN_CODEX_HOME);
        assert!(!log.exists(), "{case}: se lanzó el adaptador");
        assert!(
            !env.texts(&run, RunOutputStream::System)
                .iter()
                .any(|l| l.contains(home.to_str().unwrap())),
            "{case}: ruta en la salida"
        );
        // Deja la carpeta como estaba.
        for entry in std::fs::read_dir(&home).unwrap() {
            let entry = entry.unwrap();
            if !snapshot.contains(&entry.file_name()) {
                let path = entry.path();
                if std::fs::symlink_metadata(&path).unwrap().is_dir() {
                    std::fs::remove_dir_all(&path).unwrap();
                } else {
                    std::fs::remove_file(&path).unwrap();
                }
            }
        }
        let user_skill = home.join("skills").join("mia");
        if user_skill.exists() {
            std::fs::remove_dir(&user_skill).unwrap();
        }
        for name in kanbania_runner::acp::CODEX_PERSONAL_DENYLIST {
            let path = home.join(name);
            match std::fs::symlink_metadata(&path) {
                Ok(m) if m.is_dir() => std::fs::remove_dir_all(&path).unwrap(),
                Ok(_) => std::fs::remove_file(&path).unwrap(),
                Err(_) => {}
            }
        }
    }
}
