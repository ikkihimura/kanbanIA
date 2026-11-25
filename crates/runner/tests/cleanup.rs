//! Limpieza de ejecuciones (HU-025) de extremo a extremo: SQLite temporal real con reloj
//! controlable, repositorio Git real, worktrees reales creados por el supervisor con el proceso
//! de prueba, y una copia restaurada que deja un worktree huérfano.

#![cfg(feature = "supervisor")]
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kanbania_core::error::CoreError;
use kanbania_core::features::columns::{ColumnCategory, list_columns};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::relations::{SetStoryAgentInput, set_story_agent};
use kanbania_core::features::repositories::{LinkRepositoryInput, link_repository};
use kanbania_core::features::runs::{
    CleanRunInput, CreateRunInput, RUN_OUTPUT_DIR, RemoveOrphanWorktreeInput, RunExecutor,
    RunState, RunSummary, StartRunInput, SupervisorToken, TestScenario, WorktreeCheck,
    WorktreeRemoval, create_run, get_run, start_run,
};
use kanbania_core::features::stories::{CreateStoryInput, Priority, create_story};
use kanbania_core::features::team::{
    Agent, AgentFields, AvatarSpec, Capability, CreateAgentInput, create_agent,
};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::clock::ManualClock;
use kanbania_core::platform::storage::{BackupKind, Store, backup_path, restore_backup_waiting};
use kanbania_runner::cleanup::{clean_run, remove_orphan_worktree, retention_plan};
use kanbania_runner::supervisor::{Supervisor, SupervisorConfig};

const UI: &str = "ui";
const TEST_PROCESS: &str = env!("CARGO_BIN_EXE_kanbania-test-process");
const DAY: Duration = Duration::from_secs(86_400);

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn git(repo: &Path, args: &[&str]) -> (bool, String) {
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
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    git(
        repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .0
}

struct Env {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    repo: PathBuf,
    clock: Arc<ManualClock>,
    store: Store,
    project: Project,
    agent: Agent,
    ready_column: String,
}

impl Env {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dunce::canonicalize(dir.path()).unwrap();
        let data_dir = base.join("datos con espacios");
        std::fs::create_dir(&data_dir).unwrap();
        let repo = base.join("repositorio con espacios");
        std::fs::create_dir(&repo).unwrap();
        assert!(git(&repo, &["init", "-q"]).0);
        std::fs::write(repo.join("seguido.txt"), "original\n").unwrap();
        assert!(git(&repo, &["add", "."]).0);
        assert!(git(&repo, &["commit", "-q", "-m", "inicial"]).0);
        let clock = Arc::new(ManualClock::new(now_ms()));
        let store = Store::open(&data_dir)
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
            clock,
            store,
            project,
            agent,
            ready_column,
        }
    }

    /// Intento en cola (sin pedir el inicio) de una historia nueva lista.
    async fn queued(&self, title: &str) -> RunSummary {
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

    /// Ejecuta un intento del proceso de prueba hasta el final con un supervisor propio.
    async fn finished(&self, title: &str, scenario: TestScenario) -> RunSummary {
        let mut config = SupervisorConfig::new(self.data_dir.clone(), PathBuf::from(TEST_PROCESS));
        config.poll_interval = Duration::from_millis(100);
        config.grace = Duration::from_secs(1);
        let token = SupervisorToken::claim(&self.store).unwrap();
        let supervisor = Supervisor::start(self.store.clone(), token, config, Arc::new(|_| {}))
            .await
            .unwrap();
        let run = self.queued(title).await;
        start_run(
            &self.store,
            &Actor::Human,
            &StartRunInput {
                request_id: rid(),
                project_id: self.project.id.clone(),
                run_id: run.id.clone(),
                repository_id: None,
                executor: RunExecutor::TestProcess { scenario },
            },
        )
        .await
        .unwrap();
        supervisor.wake();
        let started = Instant::now();
        let done = loop {
            let detail = get_run(&self.store, &self.project.id, &run.id)
                .await
                .unwrap();
            if detail.run.state.is_terminal() {
                break detail.run;
            }
            assert!(started.elapsed() < Duration::from_secs(60));
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        supervisor.shutdown().await;
        assert_eq!(done.state, RunState::Completed, "{:?}", done.end_reason);
        done
    }

    fn clean_input(&self, run: &RunSummary) -> CleanRunInput {
        CleanRunInput {
            project_id: self.project.id.clone(),
            run_id: run.id.clone(),
            output: false,
            worktree: false,
            discard_worktree_changes: false,
            branch: false,
        }
    }
}

fn field(error: CoreError) -> String {
    match error {
        CoreError::Validation { field, .. } => field.to_owned(),
        other => format!("{other:?}"),
    }
}

/// CA-1 y CA-2: el plan propone solo lo terminado hace más de los días configurados, mide lo
/// que ocupa y da el estado de cada worktree y rama sin borrar nada; limpiar respeta las
/// comprobaciones de HU-018 y borra ramas solo si se marcan, con el worktree eliminado.
#[tokio::test(flavor = "multi_thread")]
async fn old_runs_are_proposed_and_cleaned_only_as_confirmed() {
    let env = Env::new().await;
    let clean = env.finished("Limpio", TestScenario::Success).await;
    let dirty = env.finished("Con cambios", TestScenario::Write).await;
    // En el de cambios: un commit propio en su rama («trabajo sin integrar») y además un
    // archivo sin seguimiento (eliminarlo exige descartar).
    let dirty_path = PathBuf::from(
        &get_run(&env.store, &env.project.id, &dirty.id)
            .await
            .unwrap()
            .worktree
            .unwrap()
            .path,
    );
    assert!(git(&dirty_path, &["add", "-A"]).0);
    assert!(git(&dirty_path, &["commit", "-q", "-m", "trabajo del agente"]).0);
    std::fs::write(dirty_path.join("sin seguimiento.txt"), "nuevo\n").unwrap();
    let clean_detail = get_run(&env.store, &env.project.id, &clean.id)
        .await
        .unwrap();
    let clean_worktree = PathBuf::from(&clean_detail.worktree.as_ref().unwrap().path);
    let clean_branch = clean_detail.worktree.as_ref().unwrap().branch.clone();

    // Recién terminados: nada que proponer, pero el plan mide lo que ocupa todo.
    let plan = retention_plan(&env.store, &env.data_dir).await.unwrap();
    assert_eq!(plan.retention_days, 30);
    assert!(plan.items.is_empty());
    assert!(plan.runs_usage.bytes > 0 && plan.worktrees_usage.bytes > 0);
    assert!(plan.orphans.is_empty());

    env.clock.advance(31 * DAY);
    // Uno más reciente, que no se propone.
    let recent = env.finished("Reciente", TestScenario::Success).await;
    let plan = retention_plan(&env.store, &env.data_dir).await.unwrap();
    let ids: Vec<&str> = plan.items.iter().map(|i| i.run_id.as_str()).collect();
    assert_eq!(ids, [clean.id.as_str(), dirty.id.as_str()]);
    assert!(!ids.contains(&recent.id.as_str()));
    let item = &plan.items[0];
    assert_eq!(item.story_title, "Limpio");
    assert!(item.output.unwrap().bytes > 0);
    let worktree = item.worktree.as_ref().unwrap();
    assert_eq!(worktree.check, WorktreeCheck::Clean);
    assert!(worktree.usage.unwrap().bytes > 0);
    let branch = item.branch.as_ref().unwrap();
    assert_eq!(branch.name, clean_branch);
    assert_eq!(branch.unintegrated_commits, Some(0));
    assert!(branch.worktree_live);
    assert!(matches!(
        plan.items[1].worktree.as_ref().unwrap().check,
        WorktreeCheck::NeedsForce { .. }
    ));
    assert_eq!(
        plan.items[1].branch.as_ref().unwrap().unintegrated_commits,
        Some(1)
    );
    // Calcular el plan no borra nada.
    assert!(clean_worktree.exists());

    // Rama sin eliminar antes el worktree → rechazado, y nada cambia.
    let mut input = env.clean_input(&clean);
    input.branch = true;
    assert_eq!(
        field(
            clean_run(&env.store, &Actor::Human, &env.data_dir, &input)
                .await
                .unwrap_err()
        ),
        "branch"
    );
    assert!(branch_exists(&env.repo, &clean_branch));

    // El de cambios, sin descartar → «force»; el worktree sigue.
    let mut dirty_input = env.clean_input(&dirty);
    dirty_input.worktree = true;
    assert_eq!(
        field(
            clean_run(&env.store, &Actor::Human, &env.data_dir, &dirty_input)
                .await
                .unwrap_err()
        ),
        "force"
    );
    let dirty_detail = get_run(&env.store, &env.project.id, &dirty.id)
        .await
        .unwrap();
    assert!(Path::new(&dirty_detail.worktree.as_ref().unwrap().path).exists());

    // Limpio: todo lo marcado, en orden (worktree, salida, rama).
    let input = CleanRunInput {
        output: true,
        worktree: true,
        branch: true,
        ..env.clean_input(&clean)
    };
    let result = clean_run(&env.store, &Actor::Human, &env.data_dir, &input)
        .await
        .unwrap();
    assert!(result.output_removed && result.branch_deleted);
    assert_eq!(result.worktree_removal, Some(WorktreeRemoval::Clean));
    assert!(!clean_worktree.exists());
    assert!(!env.data_dir.join(RUN_OUTPUT_DIR).join(&clean.id).exists());
    assert!(!branch_exists(&env.repo, &clean_branch));
    let detail = get_run(&env.store, &env.project.id, &clean.id)
        .await
        .unwrap();
    assert!(detail.output_removed_at.is_some());
    let registered = detail.worktree.unwrap();
    assert_eq!(registered.removal, Some(WorktreeRemoval::Clean));
    assert!(registered.branch_deleted_at.is_some());
    // El intento y sus eventos siguen.
    assert_eq!(detail.run.state, RunState::Completed);
    assert!(!detail.events.is_empty());

    // Descartando: se elimina el worktree; la salida y la rama se conservan (no se marcaron).
    dirty_input.discard_worktree_changes = true;
    let result = clean_run(&env.store, &Actor::Human, &env.data_dir, &dirty_input)
        .await
        .unwrap();
    assert_eq!(result.worktree_removal, Some(WorktreeRemoval::Discarded));
    assert!(!result.output_removed && !result.branch_deleted);
    let dirty_branch = dirty_detail.worktree.unwrap().branch;
    assert!(branch_exists(&env.repo, &dirty_branch));
    assert!(env.data_dir.join(RUN_OUTPUT_DIR).join(&dirty.id).exists());

    // Lo limpio desaparece del plan; del otro queda lo no marcado (salida y rama).
    let plan = retention_plan(&env.store, &env.data_dir).await.unwrap();
    assert_eq!(plan.items.len(), 1);
    let left = &plan.items[0];
    assert_eq!(left.run_id, dirty.id);
    assert!(left.worktree.is_none() && left.output.is_some());
    assert!(!left.branch.as_ref().unwrap().worktree_live);
    // El repositorio original no se ha tocado.
    assert_eq!(
        std::fs::read_to_string(env.repo.join("seguido.txt")).unwrap(),
        "original\n"
    );
}

/// CA-1: un intento activo o con una entrega pendiente nunca se limpia (ni se propone): ni su
/// salida, ni su worktree, ni su rama. Un intento cancelado en la cola no tiene nada que limpiar
/// y no se propone.
#[tokio::test(flavor = "multi_thread")]
async fn active_runs_and_pending_deliveries_are_never_cleaned() {
    use kanbania_core::features::review::{SubmitDeliveryInput, submit_delivery};
    use kanbania_core::features::runs::{CancelRunInput, cancel_run};
    let env = Env::new().await;
    let done = env.finished("Con entrega", TestScenario::Success).await;
    let cancelled = env.queued("Cancelado en cola").await;
    cancel_run(
        &env.store,
        &Actor::Human,
        &CancelRunInput {
            request_id: rid(),
            project_id: env.project.id.clone(),
            run_id: cancelled.id.clone(),
        },
    )
    .await
    .unwrap();
    env.clock.advance(31 * DAY);
    // Activo: un intento en cola (terminó hace mucho otro, pero este sigue vivo).
    let active = env.queued("Activo").await;
    submit_delivery(
        &env.store,
        &Actor::Human,
        &SubmitDeliveryInput {
            request_id: rid(),
            project_id: env.project.id.clone(),
            story_id: done.story_id.clone(),
            summary: "Listo para revisar".into(),
            references: Vec::new(),
            send_to_review: false,
        },
    )
    .await
    .unwrap();
    let plan = retention_plan(&env.store, &env.data_dir).await.unwrap();
    assert!(plan.items.is_empty(), "{:?}", plan.items);

    let all = |run: &RunSummary| CleanRunInput {
        output: true,
        worktree: true,
        branch: true,
        discard_worktree_changes: true,
        ..env.clean_input(run)
    };
    let error = clean_run(&env.store, &Actor::Human, &env.data_dir, &all(&done))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, CoreError::Validation { reason, .. } if reason.contains("entrega pendiente")),
        "{error:?}"
    );
    let error = clean_run(&env.store, &Actor::Human, &env.data_dir, &all(&active))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, CoreError::Validation { reason, .. } if reason.contains("activo")),
        "{error:?}"
    );
    // Nada se tocó.
    let detail = get_run(&env.store, &env.project.id, &done.id)
        .await
        .unwrap();
    let worktree = detail.worktree.unwrap();
    assert!(Path::new(&worktree.path).exists());
    assert!(branch_exists(&env.repo, &worktree.branch));
    assert!(env.data_dir.join(RUN_OUTPUT_DIR).join(&done.id).exists());
    assert!(detail.output_removed_at.is_none());
}

/// Revisión: todas las precondiciones se comprueban antes de borrar nada (la rama con el
/// worktree vivo y sin marcarlo no borra tampoco la salida), y lo que ya no existe en disco
/// (salida o rama borradas a mano) se anota y deja de proponerse.
#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_removed_if_a_step_cannot_run_and_vanished_data_stops_being_proposed() {
    let env = Env::new().await;
    let run = env.finished("Intento", TestScenario::Success).await;
    env.clock.advance(31 * DAY);
    let output_dir = env.data_dir.join(RUN_OUTPUT_DIR).join(&run.id);
    let input = CleanRunInput {
        output: true,
        branch: true,
        ..env.clean_input(&run)
    };
    assert_eq!(
        field(
            clean_run(&env.store, &Actor::Human, &env.data_dir, &input)
                .await
                .unwrap_err()
        ),
        "branch"
    );
    assert!(output_dir.exists(), "la salida no se borró");

    // Se eliminan worktree y rama a la vez (en orden); la salida desaparece «a mano».
    let input = CleanRunInput {
        worktree: true,
        branch: true,
        ..env.clean_input(&run)
    };
    let result = clean_run(&env.store, &Actor::Human, &env.data_dir, &input)
        .await
        .unwrap();
    assert_eq!(result.worktree_removal, Some(WorktreeRemoval::Clean));
    assert!(result.branch_deleted && !result.output_removed);
    std::fs::remove_dir_all(&output_dir).unwrap();
    let plan = retention_plan(&env.store, &env.data_dir).await.unwrap();
    assert!(plan.items.is_empty(), "{:?}", plan.items);
    let detail = get_run(&env.store, &env.project.id, &run.id).await.unwrap();
    assert!(
        detail.output_removed_at.is_some(),
        "anotado como sin salida"
    );

    // Una rama borrada a mano con el worktree ya eliminado también se anota.
    let other = env.finished("Otro", TestScenario::Success).await;
    env.clock.advance(31 * DAY);
    let input = CleanRunInput {
        worktree: true,
        output: true,
        ..env.clean_input(&other)
    };
    clean_run(&env.store, &Actor::Human, &env.data_dir, &input)
        .await
        .unwrap();
    let branch = get_run(&env.store, &env.project.id, &other.id)
        .await
        .unwrap()
        .worktree
        .unwrap()
        .branch;
    assert!(git(&env.repo, &["branch", "-D", &branch]).0);
    let plan = retention_plan(&env.store, &env.data_dir).await.unwrap();
    assert!(plan.items.is_empty(), "{:?}", plan.items);
    let registered = get_run(&env.store, &env.project.id, &other.id)
        .await
        .unwrap()
        .worktree
        .unwrap();
    assert!(registered.branch_deleted_at.is_some());
}

/// CA-2: tras restaurar una copia anterior, el worktree de un intento que la copia no conoce
/// queda huérfano: se detecta y se elimina con las mismas comprobaciones (cambios → descartar
/// explícitamente). Una carpeta que no es un worktree no se toca.
#[tokio::test(flavor = "multi_thread")]
async fn a_restored_copy_leaves_orphan_worktrees_that_can_be_removed() {
    let env = Env::new().await;
    let backup = env.store.create_backup(BackupKind::Manual).await.unwrap();
    let lost = env
        .finished("Después de la copia", TestScenario::Write)
        .await;
    let detail = get_run(&env.store, &env.project.id, &lost.id)
        .await
        .unwrap();
    let orphan = PathBuf::from(&detail.worktree.as_ref().unwrap().path);
    let branch = detail.worktree.unwrap().branch;

    let Env {
        _dir,
        data_dir,
        repo,
        store,
        ..
    } = env;
    drop(store);
    // Como el host: otra prueba en paralelo que lanza un proceso puede retener un instante el
    // descriptor del bloqueo (heredado entre `fork` y `exec`).
    let wait = Duration::from_secs(5);
    restore_backup_waiting(
        &data_dir,
        &backup_path(&data_dir, &backup.id).unwrap(),
        wait,
    )
    .await
    .unwrap();
    let started = Instant::now();
    let store = loop {
        match Store::open(&data_dir).await {
            Err(CoreError::StorageLocked) if started.elapsed() < wait => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            other => break other.unwrap(),
        }
    };
    // Una carpeta que no es un worktree con la forma <proyecto>/<intento>.
    let stray = data_dir.join("worktrees").join("otro").join("carpeta");
    std::fs::create_dir_all(&stray).unwrap();

    let plan = retention_plan(&store, &data_dir).await.unwrap();
    assert!(plan.items.is_empty());
    let orphans: Vec<_> = plan
        .orphans
        .iter()
        .map(|o| (PathBuf::from(&o.path), o.branch.clone(), o.check.clone()))
        .collect();
    assert_eq!(orphans.len(), 2, "{orphans:?}");
    let found = orphans.iter().find(|(path, ..)| *path == orphan).unwrap();
    assert_eq!(found.1.as_deref(), Some(branch.as_str()));
    assert!(matches!(found.2, WorktreeCheck::NeedsForce { .. }));
    let stray_found = orphans.iter().find(|(path, ..)| *path == stray).unwrap();
    assert!(matches!(stray_found.2, WorktreeCheck::Unverifiable { .. }));

    let request = |path: &Path, discard: bool| RemoveOrphanWorktreeInput {
        path: path.to_str().unwrap().into(),
        discard_changes: discard,
    };
    assert_eq!(
        field(
            remove_orphan_worktree(&store, &Actor::Human, &data_dir, &request(&orphan, false))
                .await
                .unwrap_err()
        ),
        "force"
    );
    assert!(orphan.exists());
    assert_eq!(
        field(
            remove_orphan_worktree(&store, &Actor::Human, &data_dir, &request(&stray, true))
                .await
                .unwrap_err()
        ),
        "worktree"
    );
    assert!(stray.exists());
    // Una ruta que no es huérfana (fuera de la raíz) → no encontrada.
    assert!(matches!(
        remove_orphan_worktree(&store, &Actor::Human, &data_dir, &request(&repo, true)).await,
        Err(CoreError::NotFound { .. })
    ));
    assert_eq!(
        remove_orphan_worktree(&store, &Actor::Human, &data_dir, &request(&orphan, true))
            .await
            .unwrap(),
        WorktreeRemoval::Discarded
    );
    assert!(!orphan.exists());
    // La rama se conserva.
    assert!(branch_exists(&repo, &branch));
    let plan = retention_plan(&store, &data_dir).await.unwrap();
    assert_eq!(plan.orphans.len(), 1);
}

/// CA-1: el núcleo exige la antigüedad y una persona aunque la UI no los compruebe: un intento
/// recién terminado no se limpia, y un agente nunca limpia nada (antes de borrar).
#[tokio::test(flavor = "multi_thread")]
async fn recent_runs_and_agents_cannot_clean() {
    let env = Env::new().await;
    let run = env.finished("Reciente", TestScenario::Success).await;
    let input = CleanRunInput {
        output: true,
        worktree: true,
        ..env.clean_input(&run)
    };
    let error = clean_run(&env.store, &Actor::Human, &env.data_dir, &input)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, CoreError::Validation { reason, .. } if reason.contains("hace menos de 30 días")),
        "{error:?}"
    );
    env.clock.advance(31 * DAY);
    let agent = Actor::Agent {
        agent_id: env.agent.id.clone(),
    };
    assert!(matches!(
        clean_run(&env.store, &agent, &env.data_dir, &input).await,
        Err(CoreError::Forbidden { .. })
    ));
    let worktree = get_run(&env.store, &env.project.id, &run.id)
        .await
        .unwrap()
        .worktree
        .unwrap();
    assert!(Path::new(&worktree.path).exists());
    assert!(env.data_dir.join(RUN_OUTPUT_DIR).join(&run.id).exists());
    clean_run(&env.store, &Actor::Human, &env.data_dir, &input)
        .await
        .unwrap();
    assert!(!Path::new(&worktree.path).exists());
}

/// Seguridad: el `.git` de un worktree lo escribe el agente. Si lo reescribe para señalar a un
/// repositorio propio (con un filtro que el Git de la app ejecutaría sin sandbox), la app no
/// ejecuta Git ahí ni lo elimina; y un huérfano de un repositorio no vinculado tampoco se toca.
#[tokio::test(flavor = "multi_thread")]
async fn a_rewritten_git_pointer_never_runs_git_elsewhere() {
    let env = Env::new().await;
    let run = env
        .finished("Puntero reescrito", TestScenario::Success)
        .await;
    env.clock.advance(31 * DAY);
    let worktree = PathBuf::from(
        get_run(&env.store, &env.project.id, &run.id)
            .await
            .unwrap()
            .worktree
            .unwrap()
            .path,
    );
    // Repositorio del agente dentro de su worktree, con un filtro que deja un testigo.
    let witness = env.data_dir.parent().unwrap().join("testigo");
    let rogue = worktree.join("x");
    std::fs::create_dir(&rogue).unwrap();
    assert!(git(&rogue, &["init", "-q"]).0);
    let filter = format!("touch '{}'; cat", witness.display());
    assert!(git(&rogue, &["config", "filter.trap.clean", &filter]).0);
    std::fs::write(worktree.join(".gitattributes"), "* filter=trap\n").unwrap();
    let admin = rogue.join(".git").join("worktrees").join("n");
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::write(
        admin.join("gitdir"),
        format!("{}\n", worktree.join(".git").display()),
    )
    .unwrap();
    std::fs::write(admin.join("commondir"), "../..\n").unwrap();
    std::fs::write(admin.join("HEAD"), "ref: refs/heads/master\n").unwrap();
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", admin.display()),
    )
    .unwrap();

    // Huérfano registrado de verdad, pero en un repositorio que ningún proyecto vincula.
    let foreign = env.data_dir.parent().unwrap().join("ajeno");
    std::fs::create_dir(&foreign).unwrap();
    assert!(git(&foreign, &["init", "-q"]).0);
    assert!(
        git(
            &foreign,
            &["commit", "-q", "--allow-empty", "-m", "inicial"]
        )
        .0
    );
    let stray = env.data_dir.join("worktrees").join("otro").join("ajeno");
    assert!(
        git(
            &foreign,
            &["worktree", "add", "-q", "--detach", stray.to_str().unwrap()]
        )
        .0
    );

    let plan = retention_plan(&env.store, &env.data_dir).await.unwrap();
    let item = plan.items.iter().find(|i| i.run_id == run.id).unwrap();
    let check = &item.worktree.as_ref().unwrap().check;
    assert!(
        matches!(check, WorktreeCheck::Unverifiable { .. }),
        "{check:?}"
    );
    let orphan = plan
        .orphans
        .iter()
        .find(|o| Path::new(&o.path) == stray)
        .unwrap();
    assert!(
        matches!(&orphan.check, WorktreeCheck::Unverifiable { reason } if reason.contains("No se reconoce")),
        "{:?}",
        orphan.check
    );

    let input = CleanRunInput {
        worktree: true,
        discard_worktree_changes: true,
        ..env.clean_input(&run)
    };
    assert_eq!(
        field(
            clean_run(&env.store, &Actor::Human, &env.data_dir, &input)
                .await
                .unwrap_err()
        ),
        "worktree"
    );
    let request = RemoveOrphanWorktreeInput {
        path: stray.to_str().unwrap().into(),
        discard_changes: true,
    };
    assert_eq!(
        field(
            remove_orphan_worktree(&env.store, &Actor::Human, &env.data_dir, &request)
                .await
                .unwrap_err()
        ),
        "worktree"
    );
    assert!(worktree.exists() && stray.exists());
    assert!(!witness.exists(), "Git ejecutó el filtro del agente");
}
