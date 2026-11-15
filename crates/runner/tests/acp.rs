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
    AcpSessionInfo, CancelRunInput, CreateRunInput, RunDetail, RunEvent, RunExecutor,
    RunOutputKind, RunOutputLine, RunOutputStream, RunState, RunSummary, StartRunInput,
    SupervisorToken, UpdateRunSettingsInput, cancel_run, create_run, get_run, get_run_settings,
    prompt_digest, start_run, update_run_settings,
};
use kanbania_core::features::stories::{CreateStoryInput, Priority, create_story};
use kanbania_core::features::team::{
    Agent, AgentFields, AvatarSpec, Capability, CreateAgentInput, create_agent,
};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::storage::Store;
use kanbania_runner::acp::{AcpLimits, ProbeError, probe_adapter};
use kanbania_runner::supervisor::{Supervisor, SupervisorConfig, read_run_output};

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
        let current = get_run_settings(&self.store).await.unwrap();
        update_run_settings(
            &self.store,
            &Actor::Human,
            &UpdateRunSettingsInput {
                expected_revision: current.revision,
                max_concurrency: current.max_concurrency,
                timeout_minutes: current.timeout_minutes,
                claude_acp_path: path,
            },
        )
        .await
        .unwrap();
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
                executor: RunExecutor::ClaudeAcp,
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
        let supervisor = self.supervisor(self.config(args)).await;
        let run = self.queued(&supervisor).await;
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
async fn permissions_are_always_rejected_once() {
    let env = Env::new().await;
    let (run, detail) = env.scenario(&["permission"]).await;
    // El agente falso termina con error si no recibe `reject_once` (o `cancelled` sin esa
    // opción): llegar a `completed` prueba la respuesta.
    assert_eq!(detail.run.state, RunState::Completed, "{:?}", detail.run);
    let permissions = env.agent_lines(&run, RunOutputKind::Permission);
    assert_eq!(permissions.len(), 2, "{permissions:?}");
    assert!(
        permissions[0].starts_with("Permiso pedido para «Editar README.md»: rechazado una vez")
    );
    assert!(permissions[1].contains("sin opción «rechazar una vez», respondido como cancelado"));
    // El U+202E del título se sustituye: nunca llega a la salida.
    assert!(permissions[1].starts_with("Permiso pedido para «Ejecutar  pruebas»"));
    let raw = env.raw_output(&run);
    assert!(!raw.contains('\u{202E}') && !raw.to_lowercase().contains("\\u202e"));
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
        Duration::from_secs(20),
    )
    .await;
    assert_eq!(version2, Err(ProbeError::VersionUnsupported));
    let missing = probe_adapter(
        env.base.join("no existe"),
        Vec::new(),
        cwd.clone(),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(missing, Err(ProbeError::NotFound));
    // Sin argumentos el agente falso imprime el uso y termina: desconexión.
    let quits = probe_adapter(
        env.adapter.clone(),
        Vec::new(),
        cwd,
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
    assert_eq!(received(&log), ["initialize"]);
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
