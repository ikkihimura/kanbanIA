//! Prueba **real** con el adaptador de Claude Code (HU-019, CA-3). Consume la suscripción del
//! propietario: está `#[ignore]` y además solo corre si `KANBANIA_ACP_REAL` contiene la ruta
//! absoluta del adaptador (`claude-agent-acp`). Nunca en CI. Ejecutar con autorización:
//!
//! ```sh
//! KANBANIA_ACP_REAL=/ruta/absoluta/claude-agent-acp \
//!   cargo test -p kanbania-runner --test acp_real -- --ignored --nocapture
//! ```
//!
//! Usa el cliente ACP directamente (mismo proceso, entorno, modo y aislamiento de la
//! configuración que el supervisor) sobre un repositorio Git temporal con un README. Dos
//! pruebas separadas (se pueden ejecutar por nombre):
//! - `real_claude_code_reads_and_summarizes`: prompt que no necesita permisos; comprueba
//!   `end_turn`, al menos un mensaje del agente, el modo `default` y que la carpeta no cambió;
//! - `real_claude_code_permission_is_rejected`: prompt que pide crear un archivo y ejecutar un
//!   comando; comprueba al menos un permiso rechazado, que `nota.txt` no existe, que la carpeta
//!   no cambió, que el turno terminó con un motivo válido y que no quedan procesos;
//! - `real_claude_code_ignores_repository_settings`: el repositorio versiona un
//!   `.claude/settings.json` que permite `Write`/`Edit`/`Bash` y un hook `SessionStart` que
//!   crearía `hook-testigo.txt`, y un `.mcp.json` con un servidor inexistente; con el
//!   aislamiento (`_meta.claudeCode.options.settingSources: []`) el permiso se pide y se
//!   rechaza, y ni `nota.txt` ni `hook-testigo.txt` existen. Limitación: Claude Code puede no
//!   ejecutar hooks de un proyecto no confiable («workspace trust») aun sin aislamiento; la
//!   parte de los permisos `allow` sí distingue ambos casos.
//!
//! La variable `KANBANIA_ACP_REAL` no llega al adaptador (entorno limpio).

#![allow(clippy::unwrap_used, clippy::panic, clippy::print_stdout)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use kanbania_runner::acp::{AcpClient, AcpFailure, AcpLimits, AcpStop, CLAUDE_CODE_PROFILE};
use kanbania_runner::output::{AgentKind, OutputLine, OutputLog, Stream, read_tail};
use kanbania_runner::process::{ProcessSpec, RunningProcess, cancel_pair};

const READ_PROMPT: &str = "Lee el archivo README.md y resume su contenido en una frase. No \
                           edites ni crees archivos y no ejecutes comandos.";
const ISOLATION_PROMPT: &str = "Crea un archivo llamado nota.txt con el texto hola en la carpeta \
                                actual. Si no tienes permiso, dilo y termina.";
const PERMISSION_PROMPT: &str = "Crea un archivo llamado nota.txt con el texto hola en la carpeta \
                                 actual y después ejecuta el comando ls. Si no tienes permiso, \
                                 dilo y termina.";

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
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

/// Estado observable de la carpeta: HEAD, `git status` (incluidos ignorados) y el README.
fn snapshot(repo: &Path) -> (String, String, String) {
    (
        git(repo, &["rev-parse", "HEAD"]),
        git(
            repo,
            &[
                "status",
                "--porcelain",
                "--ignored",
                "--untracked-files=all",
            ],
        ),
        std::fs::read_to_string(repo.join("README.md")).unwrap(),
    )
}

/// Resultado de un turno real.
struct RealTurn {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    before: (String, String, String),
    ended: Result<(), AcpStop>,
    lines: Vec<OutputLine>,
    /// Grupo de procesos del adaptador (Unix: su pid).
    #[cfg_attr(not(unix), allow(dead_code))]
    pid: Option<u32>,
}

/// Lanza el adaptador real sobre un repositorio temporal y ejecuta un turno con `prompt`.
/// `None` si `KANBANIA_ACP_REAL` no está definida.
async fn real_turn(prompt: &str, prepare: impl FnOnce(&Path)) -> Option<RealTurn> {
    let Some(adapter) = std::env::var_os("KANBANIA_ACP_REAL").map(PathBuf::from) else {
        println!("KANBANIA_ACP_REAL no está definida: no se ejecuta nada.");
        return None;
    };
    assert!(
        adapter.is_absolute(),
        "KANBANIA_ACP_REAL debe ser una ruta absoluta"
    );

    let dir = tempfile::tempdir().unwrap();
    let base = dunce::canonicalize(dir.path()).unwrap();
    let repo = base.join("repositorio de prueba");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(
        repo.join("README.md"),
        "# Recetario\n\nUna aplicación de ejemplo para guardar recetas de cocina y \
         consultarlas por ingrediente.\n",
    )
    .unwrap();
    prepare(&repo);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "inicial"]);
    let before = snapshot(&repo);

    let output_dir = base.join("salida");
    let log = OutputLog::create(&output_dir).unwrap();
    let spec = ProcessSpec {
        program: adapter,
        args: Vec::new(),
        cwd: repo.clone(),
        extra_env: Vec::new(),
        timeout: Duration::from_secs(5 * 60),
        grace: Duration::from_secs(5),
    };
    let (process, io) = RunningProcess::spawn_protocol(spec, log).unwrap();
    let pid = process.pid();
    let deadline = process.deadline();
    let limits = AcpLimits::default();
    let mut client = AcpClient::new(io, limits, None);
    let (_keep, mut token) = cancel_pair();

    let declared = client.initialize(&mut token, deadline).await.unwrap();
    println!(
        "agente: {:?} {:?} {:?}; capacidades: {:?}",
        declared.name, declared.title, declared.version, declared.capabilities
    );
    let session = client
        .open_session(&repo, &declared, &CLAUDE_CODE_PROFILE, &mut token, deadline)
        .await
        .unwrap();
    assert_eq!(session.mode, "default");
    let ended = client.prompt(prompt, &mut token, deadline).await;
    client.close_input();
    drop(client);
    let (report, log) = process.close(limits.close_wait).await;
    drop(log);
    println!("turno: {ended:?}; proceso: {:?}", report.outcome);

    let lines = read_tail(&output_dir, 500).unwrap().lines;
    for line in &lines {
        println!("[{:?} {:?}] {}", line.stream, line.kind, line.text);
    }
    assert!(
        !lines
            .iter()
            .any(|l| l.text.contains("cambió por su cuenta al modo")),
        "el agente abandonó el modo default"
    );
    Some(RealTurn {
        _dir: dir,
        repo,
        before,
        ended,
        lines,
        pid,
    })
}

fn has_agent_line(lines: &[OutputLine], kind: AgentKind) -> bool {
    lines
        .iter()
        .any(|l| l.stream == Stream::Agent && l.kind == Some(kind))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "usa el adaptador real de Claude Code: solo con KANBANIA_ACP_REAL y autorización"]
async fn real_claude_code_reads_and_summarizes() {
    let Some(turn) = real_turn(READ_PROMPT, |_| {}).await else {
        return;
    };
    assert_eq!(turn.ended, Ok(()), "el turno no terminó con end_turn");
    assert!(
        has_agent_line(&turn.lines, AgentKind::Message),
        "sin mensajes del agente"
    );
    assert_eq!(snapshot(&turn.repo), turn.before, "la carpeta cambió");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "usa el adaptador real de Claude Code: solo con KANBANIA_ACP_REAL y autorización"]
async fn real_claude_code_permission_is_rejected() {
    let Some(turn) = real_turn(PERMISSION_PROMPT, |_| {}).await else {
        return;
    };
    // Cualquier fin de turno válido del agente; nunca tiempo agotado ni desconexión.
    assert!(
        matches!(
            turn.ended,
            Ok(())
                | Err(AcpStop::Failed(
                    AcpFailure::Refused | AcpFailure::LimitReached
                ))
        ),
        "el turno no terminó con un motivo válido: {:?}",
        turn.ended
    );
    assert!(
        has_agent_line(&turn.lines, AgentKind::Permission),
        "el agente no pidió ningún permiso"
    );
    assert!(
        !turn.repo.join("nota.txt").exists(),
        "se creó nota.txt pese a rechazar el permiso"
    );
    assert_eq!(snapshot(&turn.repo), turn.before, "la carpeta cambió");
    #[cfg(unix)]
    {
        let pid = i32::try_from(turn.pid.unwrap()).unwrap();
        assert!(
            nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), None).is_err(),
            "quedan procesos del adaptador"
        );
    }
}

/// Ajustes de proyecto que, si se cargaran, permitirían escribir sin preguntar y ejecutarían
/// un hook al empezar la sesión (formato de https://code.claude.com/docs/en/hooks: evento →
/// grupo con `matcher` opcional → `hooks` con `type: "command"`; se ejecuta en el `cwd`).
const REPOSITORY_SETTINGS: &str = r#"{
  "permissions": { "allow": ["Write", "Edit", "Bash(ls:*)", "Bash(touch:*)"] },
  "hooks": {
    "SessionStart": [
      { "hooks": [ { "type": "command", "command": "touch hook-testigo.txt" } ] }
    ]
  }
}
"#;
/// Servidor MCP stdio que no existe: si se cargara, el adaptador intentaría lanzarlo.
const REPOSITORY_MCP: &str = r#"{
  "mcpServers": {
    "inexistente": { "command": "/no/existe/servidor-mcp-kanbania", "args": [] }
  }
}
"#;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "usa el adaptador real de Claude Code: solo con KANBANIA_ACP_REAL y autorización"]
async fn real_claude_code_ignores_repository_settings() {
    let Some(turn) = real_turn(ISOLATION_PROMPT, |repo| {
        std::fs::create_dir(repo.join(".claude")).unwrap();
        std::fs::write(
            repo.join(".claude").join("settings.json"),
            REPOSITORY_SETTINGS,
        )
        .unwrap();
        std::fs::write(repo.join(".mcp.json"), REPOSITORY_MCP).unwrap();
    })
    .await
    else {
        return;
    };
    assert!(
        matches!(
            turn.ended,
            Ok(())
                | Err(AcpStop::Failed(
                    AcpFailure::Refused | AcpFailure::LimitReached
                ))
        ),
        "el turno no terminó con un motivo válido: {:?}",
        turn.ended
    );
    assert!(
        turn.lines.iter().any(|l| l.stream == Stream::Agent
            && l.kind == Some(AgentKind::Permission)
            && l.text.contains("rechazado")),
        "no se rechazó ningún permiso: se aplicaron las reglas `allow` del repositorio"
    );
    assert!(
        !turn.repo.join("nota.txt").exists(),
        "se creó nota.txt: se aplicaron las reglas `allow` del repositorio"
    );
    assert!(
        !turn.repo.join("hook-testigo.txt").exists(),
        "se ejecutó el hook SessionStart del repositorio"
    );
    assert_eq!(snapshot(&turn.repo), turn.before, "la carpeta cambió");
    #[cfg(unix)]
    {
        let pid = i32::try_from(turn.pid.unwrap()).unwrap();
        assert!(
            nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), None).is_err(),
            "quedan procesos del adaptador"
        );
    }
}
