use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::run::{RunState, RunSummary, require_run, validate_id};
use super::runnable::ensure_runnable;
use super::settings::load_settings;
use super::snapshot::{RunSnapshot, SnapshotRepository};
use crate::error::CoreError;
use crate::features::stories::load_story;
use crate::platform::actor::Actor;
use crate::platform::idempotency::MutationKey;
use crate::platform::storage::Store;

const OPERATION: &str = "runs.start";

/// Escenario del proceso de prueba incluido (sin IA) que lanza el supervisor (HU-018). Se
/// pasa como argumento explícito al proceso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TestScenario {
    /// Imprime líneas en stdout y stderr y termina con 0.
    Success,
    /// Termina con código 3.
    Fail,
    /// Crea procesos hijos que duermen e imprime cada segundo (10 min).
    Slow,
    /// Crea o modifica un archivo en el worktree y termina con 0 (deja cambios).
    Write,
    /// Genera mucha salida (prueba la salida acotada).
    Flood,
    /// No imprime y duerme (prueba el tiempo máximo).
    Hang,
}

impl TestScenario {
    pub const ALL: [Self; 6] = [
        Self::Success,
        Self::Fail,
        Self::Slow,
        Self::Write,
        Self::Flood,
        Self::Hang,
    ];

    /// Valor guardado y argumento del proceso de prueba.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Fail => "fail",
            Self::Slow => "slow",
            Self::Write => "write",
            Self::Flood => "flood",
            Self::Hang => "hang",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self, CoreError> {
        Self::ALL
            .into_iter()
            .find(|s| s.as_str() == value)
            .ok_or_else(|| {
                CoreError::Database(sqlx::Error::Decode(
                    format!("escenario desconocido: {value}").into(),
                ))
            })
    }
}

/// Agente real que se lanza por ACP (HU-019: Claude Code; HU-021: Codex). Cada uno tiene su
/// adaptador en «Sistema → Ejecuciones» y su perfil en el runner (modo que pide permiso,
/// aislamiento, entorno). La identidad del agente del equipo (nombre, avatar) no depende de él.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AcpAgent {
    /// Claude Code vía `@agentclientprotocol/claude-agent-acp`.
    ClaudeCode,
    /// Codex vía `@agentclientprotocol/codex-acp`.
    Codex,
}

impl AcpAgent {
    pub const ALL: [Self; 2] = [Self::ClaudeCode, Self::Codex];

    /// Campo de la ruta de su adaptador en `UpdateRunSettingsInput` (errores de validación).
    pub fn settings_field(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claudeAcpPath",
            Self::Codex => "codexAcpPath",
        }
    }

    /// Nombre para mostrar.
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }
}

/// Carpeta de Codex propia de la app (`CODEX_HOME`, HU-021, decisión del propietario) dentro
/// del directorio de datos: Codex no carga la configuración, los servidores MCP, las reglas ni
/// el historial de `~/.codex` del usuario. La persona inicia sesión una vez en ella
/// (`CODEX_HOME=<ruta> codex login`); guarda credenciales: nunca para agentes ni en copias
/// exportadas.
pub const CODEX_HOME_DIR: &str = "codex-home";

/// `<data_dir>/codex-home`.
pub fn codex_home(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join(CODEX_HOME_DIR)
}

/// Estado de la carpeta de Codex de la app para «Sistema → Ejecuciones» (HU-021). Ruta local:
/// solo para la persona, nunca para agentes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CodexHomeStatus {
    /// Ruta absoluta de la carpeta (para el comando `CODEX_HOME=<ruta> codex login`).
    pub path: String,
    /// Hay un `auth.json` (archivo normal) en la carpeta: se inició sesión alguna vez. Solo se
    /// comprueba que exista; nunca se lee. No garantiza que la sesión siga siendo válida.
    pub signed_in: bool,
}

/// Lee el estado de la carpeta de Codex (E/S bloqueante y breve: llámalo fuera del runtime).
pub fn codex_home_status(data_dir: &std::path::Path) -> CodexHomeStatus {
    let home = codex_home(data_dir);
    let signed_in =
        std::fs::symlink_metadata(home.join("auth.json")).is_ok_and(|metadata| metadata.is_file());
    CodexHomeStatus {
        path: home.to_string_lossy().into_owned(),
        signed_in,
    }
}

/// Ejecutor elegido al pedir el inicio (HU-019, HU-021). JSON con etiqueta `kind`:
/// `{ "kind": "testProcess", "scenario": "success" }`, `{ "kind": "claudeAcp" }` o
/// `{ "kind": "codexAcp" }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum RunExecutor {
    /// Proceso de prueba incluido (sin IA): nunca se presenta como trabajo de un agente.
    TestProcess { scenario: TestScenario },
    /// Claude Code vía ACP con el adaptador configurado en «Sistema → Ejecuciones».
    ClaudeAcp,
    /// Codex vía ACP con el adaptador configurado en «Sistema → Ejecuciones» (HU-021).
    CodexAcp,
}

impl RunExecutor {
    /// Valor de la columna `executor` de `run_start_requests`.
    pub(super) fn column(self) -> &'static str {
        match self {
            Self::TestProcess { .. } => "test_process",
            Self::ClaudeAcp => "claude_acp",
            Self::CodexAcp => "codex_acp",
        }
    }

    /// Agente ACP del ejecutor (ninguno para el proceso de prueba).
    pub fn acp_agent(self) -> Option<AcpAgent> {
        match self {
            Self::TestProcess { .. } => None,
            Self::ClaudeAcp => Some(AcpAgent::ClaudeCode),
            Self::CodexAcp => Some(AcpAgent::Codex),
        }
    }

    /// Valor de la columna `scenario` (solo el proceso de prueba).
    pub(super) fn scenario(self) -> Option<TestScenario> {
        match self {
            Self::TestProcess { scenario } => Some(scenario),
            Self::ClaudeAcp | Self::CodexAcp => None,
        }
    }

    /// Lee las columnas `executor` y `scenario` de una petición de inicio.
    pub(super) fn from_columns(executor: &str, scenario: Option<&str>) -> Result<Self, CoreError> {
        match (executor, scenario) {
            ("test_process", Some(scenario)) => Ok(Self::TestProcess {
                scenario: TestScenario::parse(scenario)?,
            }),
            ("claude_acp", None) => Ok(Self::ClaudeAcp),
            ("codex_acp", None) => Ok(Self::CodexAcp),
            _ => Err(CoreError::Database(sqlx::Error::Decode(
                format!("ejecutor desconocido: {executor}").into(),
            ))),
        }
    }
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartRunInput {
    /// Repítelo al reintentar el mismo envío.
    pub request_id: String,
    pub project_id: String,
    pub run_id: String,
    /// Repositorio de la instantánea donde crear el worktree. Obligatorio si la instantánea
    /// tiene varios con commit; si solo hay uno, se usa ese.
    #[ts(optional)]
    pub repository_id: Option<String>,
    pub executor: RunExecutor,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    project_id: &'a str,
    run_id: &'a str,
    repository_id: Option<&'a str>,
    executor: RunExecutor,
}

/// Pide iniciar un intento en cola con el ejecutor elegido: el proceso de prueba (sin IA) o
/// Claude Code vía ACP (HU-019). Solo personas, con
/// `requestId` (reintentar devuelve el resultado original). Una transacción `BEGIN IMMEDIATE`
/// que registra la petición de inicio; el intento sigue `queued` (`startRequestedAt`) hasta
/// que el supervisor tenga hueco y lo despache, momento en que vuelve a evaluar
/// `ensure_runnable`.
///
/// Errores: `claudeAcp`/`codexAcp` sin la ruta de su adaptador configurada → `validation`
/// (`executor`);
/// intento de otro proyecto o inexistente → `not_found`; no está en cola →
/// `invalid_run_transition`; ya tiene una petición pendiente → `validation` (`runId`);
/// la instantánea no tiene ningún repositorio con commit, el indicado no está en ella o no
/// tiene commit, hay varios y no se indicó, o ya no está vinculado al proyecto en la misma ruta
/// → `validation` (`repositoryId`); la historia ya no
/// es ejecutable → los errores de `ensure_runnable` (`validation` o `claimed`).
pub async fn start_run(
    store: &Store,
    actor: &Actor,
    input: &StartRunInput,
) -> Result<RunSummary, CoreError> {
    actor.require_human("iniciar un intento")?;
    validate_id("runId", &input.run_id)?;
    if let Some(repository_id) = &input.repository_id {
        validate_id("repositoryId", repository_id)?;
    }
    let key = MutationKey::new(actor.receipt_key(), OPERATION, &input.request_id)?;
    let payload = Payload {
        project_id: &input.project_id,
        run_id: &input.run_id,
        repository_id: input.repository_id.as_deref(),
        executor: input.executor,
    };
    let (input, clock) = (input.clone(), store.clone());
    store
        .write_idempotent(&key, &payload, move |conn| {
            Box::pin(async move {
                let now = clock.now_ms();
                let run = require_run(conn, &input.project_id, &input.run_id).await?;
                if run.state != RunState::Queued {
                    return Err(CoreError::InvalidRunTransition {
                        from: run.state.label(),
                        to: RunState::Preparing.label(),
                    });
                }
                if run.start_requested_at.is_some() {
                    return Err(CoreError::Validation {
                        field: "runId",
                        reason: format!(
                            "El intento #{} ya tiene una petición de inicio: espera a que \
                             haya hueco o cancélalo.",
                            run.number
                        ),
                    });
                }
                if let Some(agent) = input.executor.acp_agent()
                    && load_settings(conn).await?.adapter_path(agent).is_none()
                {
                    return Err(CoreError::Validation {
                        field: "executor",
                        reason: format!(
                            "Configura la ruta del adaptador de {} en Sistema → Ejecuciones.",
                            agent.label()
                        ),
                    });
                }
                let snapshot = load_snapshot(conn, &run.id).await?;
                let repository = choose_repository(&snapshot, input.repository_id.as_deref())?;
                if !is_repository_linked(conn, &run.project_id, repository).await? {
                    return Err(CoreError::Validation {
                        field: "repositoryId",
                        reason: "Ese repositorio ya no está vinculado al proyecto (o se reubicó): \
                                 prepara un intento nuevo con los repositorios vinculados ahora."
                            .into(),
                    });
                }
                let story = load_story(conn, &run.project_id, &run.story_id)
                    .await?
                    .ok_or(CoreError::NotFound { entity: "Historia" })?;
                ensure_runnable(conn, &story, &run.agent_id, now).await?;
                sqlx::query(
                    "INSERT INTO run_start_requests (run_id, repository_id, executor, scenario, \
                     requested_at_ms) VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&run.id)
                .bind(&repository.id)
                .bind(input.executor.column())
                .bind(input.executor.scenario().map(TestScenario::as_str))
                .bind(now)
                .execute(&mut *conn)
                .await?;
                require_run(conn, &input.project_id, &input.run_id).await
            })
        })
        .await
}

pub(super) async fn load_snapshot(
    conn: &mut SqliteConnection,
    run_id: &str,
) -> Result<RunSnapshot, CoreError> {
    let json: String = sqlx::query_scalar("SELECT snapshot FROM runs WHERE id = ?")
        .bind(run_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(serde_json::from_str(&json)?)
}

/// El repositorio de la instantánea sigue vinculado al proyecto con el mismo id y la misma ruta
/// (decisión del propietario: la instantánea sola no autoriza a crear worktrees en una carpeta
/// que la persona ya desvinculó). Se comprueba al iniciar, al despachar y al registrar el
/// worktree.
pub(super) async fn is_repository_linked(
    conn: &mut SqliteConnection,
    project_id: &str,
    repository: &SnapshotRepository,
) -> Result<bool, CoreError> {
    let linked: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM repository_links WHERE project_id = ? AND id = ? AND path = ?",
    )
    .bind(project_id)
    .bind(&repository.id)
    .bind(&repository.path)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(linked.is_some())
}

/// Repositorio de la instantánea con commit congelado: la referencia explícita del worktree
/// (nunca la rama ni el HEAD actual).
pub(super) fn choose_repository<'a>(
    snapshot: &'a RunSnapshot,
    requested: Option<&str>,
) -> Result<&'a SnapshotRepository, CoreError> {
    let invalid = |reason: &str| CoreError::Validation {
        field: "repositoryId",
        reason: reason.to_owned(),
    };
    let with_commit: Vec<&SnapshotRepository> = snapshot
        .repositories
        .iter()
        .filter(|r| r.head.is_some())
        .collect();
    match requested {
        Some(id) => {
            let repository = snapshot
                .repositories
                .iter()
                .find(|r| r.id == id)
                .ok_or_else(|| invalid("Ese repositorio no está en la instantánea del intento."))?;
            if repository.head.is_none() {
                return Err(invalid(
                    "Ese repositorio no tenía un commit legible al preparar el intento: no hay \
                     referencia desde la que crear el worktree. Prepara un intento nuevo.",
                ));
            }
            Ok(repository)
        }
        None => match with_commit.as_slice() {
            [only] => Ok(only),
            [] => Err(invalid(
                "La instantánea del intento no tiene ningún repositorio con commit: vincula un \
                 repositorio Git con al menos un commit al proyecto y prepara un intento nuevo.",
            )),
            _ => Err(invalid(
                "La instantánea tiene varios repositorios: elige en cuál crear el worktree.",
            )),
        },
    }
}
