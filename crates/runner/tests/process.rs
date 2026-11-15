//! Proceso supervisado con el proceso de prueba real (`kanbania-test-process`).

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kanbania_runner::output::{
    MAX_FILE_BYTES, MAX_FILES, MAX_TAIL_LINES, OutputLog, Stream, read_tail,
};
use kanbania_runner::process::{
    ProcessOutcome, ProcessReport, ProcessSpec, RunningProcess, SpawnErrorKind, cancel_pair,
};
use kanbania_runner::test_process::{FAIL_EXIT_CODE, WRITE_FILE_NAME};

const TEST_PROCESS: &str = env!("CARGO_BIN_EXE_kanbania-test-process");

fn spec(args: &[&str], cwd: &Path) -> ProcessSpec {
    ProcessSpec {
        program: PathBuf::from(TEST_PROCESS),
        args: args.iter().map(OsString::from).collect(),
        cwd: cwd.to_path_buf(),
        extra_env: Vec::new(),
        timeout: Duration::from_secs(60),
        grace: Duration::from_secs(2),
    }
}

async fn run(spec: ProcessSpec, output_dir: &Path) -> ProcessReport {
    let log = OutputLog::create(output_dir).unwrap();
    let process = RunningProcess::spawn(spec, log).unwrap();
    let (_cancel, token) = cancel_pair();
    process.wait(token).await.0
}

fn texts(dir: &Path, stream: Stream) -> Vec<String> {
    read_tail(dir, MAX_TAIL_LINES)
        .unwrap()
        .lines
        .into_iter()
        .filter(|line| line.stream == stream)
        .map(|line| line.text)
        .collect()
}

/// Espera a que aparezca el archivo de pids (escrito de forma atómica) y lo lee.
async fn read_pids(path: &Path) -> Vec<i32> {
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(path) {
            return text.lines().map(|l| l.parse().unwrap()).collect();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("el proceso de prueba no escribió sus pids");
}

/// `true` si el pid sigue existiendo (incluido zombi). Se deja un margen para que el sistema
/// recoja a los huérfanos.
#[cfg(unix)]
async fn alive_after_grace(pid: i32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    for _ in 0..100 {
        if kill(Pid::from_raw(pid), None).is_err() {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    true
}

#[tokio::test(flavor = "multi_thread")]
async fn captures_stdout_and_stderr_and_exit_code_zero() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("salida");
    let report = run(spec(&["success"], dir.path()), &out).await;
    assert_eq!(report.outcome, ProcessOutcome::Exited { code: 0 });
    let stdout = texts(&out, Stream::Stdout);
    assert_eq!(
        stdout.first().map(String::as_str),
        Some("Proceso de prueba (sin IA): inicio")
    );
    assert!(stdout.contains(&"paso 2 de 3".to_owned()));
    assert_eq!(texts(&out, Stream::Stderr), ["aviso de prueba en stderr"]);
    assert_eq!(report.output.lines, 6);
    assert_eq!(report.output.cut_lines, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn failure_reports_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("salida");
    let report = run(spec(&["fail"], dir.path()), &out).await;
    assert_eq!(
        report.outcome,
        ProcessOutcome::Exited {
            code: FAIL_EXIT_CODE
        }
    );
    assert_eq!(
        texts(&out, Stream::Stderr),
        ["error simulado del proceso de prueba"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_terminates_the_whole_tree() {
    let dir = tempfile::tempdir().unwrap();
    let pids = dir.path().join("pids.txt");
    let beats = dir.path().join("latidos");
    std::fs::create_dir(&beats).unwrap();
    let mut spec = spec(
        &[
            "slow",
            "--pids",
            pids.to_str().unwrap(),
            "--heartbeat",
            beats.to_str().unwrap(),
        ],
        dir.path(),
    );
    spec.timeout = Duration::from_secs(4);
    let started = std::time::Instant::now();
    let output = dir.path().join("salida");
    let running = tokio::spawn(async move { run(spec, &output).await });
    common::wait_heartbeats_alive(&beats).await;
    let report = running.await.unwrap();
    assert_eq!(report.outcome, ProcessOutcome::TimedOut);
    assert!(report.tree_terminated);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
    common::assert_heartbeats_stopped(&beats).await;
    let pids = read_pids(&pids).await;
    assert_eq!(pids.len(), 3);
    #[cfg(unix)]
    for pid in pids {
        assert!(!alive_after_grace(pid).await, "sigue vivo el pid {pid}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hang_times_out_without_output() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("salida");
    let mut spec = spec(&["hang"], dir.path());
    spec.timeout = Duration::from_secs(1);
    let report = run(spec, &out).await;
    assert_eq!(report.outcome, ProcessOutcome::TimedOut);
    assert_eq!(report.output.lines, 0);
    assert!(report.elapsed < Duration::from_secs(10));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_kills_the_process_and_its_children() {
    let dir = tempfile::tempdir().unwrap();
    let pids_file = dir.path().join("pids.txt");
    let beats = dir.path().join("latidos");
    std::fs::create_dir(&beats).unwrap();
    let out = dir.path().join("salida");
    let log = OutputLog::create(&out).unwrap();
    let process = RunningProcess::spawn(
        spec(
            &[
                "slow",
                "--pids",
                pids_file.to_str().unwrap(),
                "--heartbeat",
                beats.to_str().unwrap(),
            ],
            dir.path(),
        ),
        log,
    )
    .unwrap();
    let root_pid = process.pid().unwrap();
    let (cancel, token) = cancel_pair();
    let waiting = tokio::spawn(process.wait(token));

    let pids = read_pids(&pids_file).await;
    assert_eq!(pids.len(), 3);
    assert_eq!(pids[0], i32::try_from(root_pid).unwrap());
    common::wait_heartbeats_alive(&beats).await;
    // Que haya algo de salida antes de cancelar.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    cancel.cancel();
    let (report, log) = waiting.await.unwrap();
    assert_eq!(report.outcome, ProcessOutcome::Cancelled);
    assert!(report.tree_terminated);
    assert!(report.elapsed < Duration::from_secs(20));
    assert!(log.is_some());
    assert!(
        texts(&out, Stream::Stdout)
            .iter()
            .any(|t| t.starts_with("progreso 1/"))
    );
    common::assert_heartbeats_stopped(&beats).await;
    #[cfg(unix)]
    for pid in pids {
        assert!(!alive_after_grace(pid).await, "sigue vivo el pid {pid}");
    }
}

/// Un proceso que ignora `SIGTERM` (y su hijo, que hereda la señal ignorada) muere con
/// `SIGKILL` al terminar la gracia.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn grace_period_escalates_to_sigkill() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("salida");
    let spec = ProcessSpec {
        program: PathBuf::from("/bin/sh"),
        args: vec![
            "-c".into(),
            "trap '' TERM; sleep 600 & echo \"hijo $!\"; wait".into(),
        ],
        cwd: dir.path().to_path_buf(),
        extra_env: Vec::new(),
        timeout: Duration::from_secs(60),
        grace: Duration::from_millis(500),
    };
    let log = OutputLog::create(&out).unwrap();
    let process = RunningProcess::spawn(spec, log).unwrap();
    let root = i32::try_from(process.pid().unwrap()).unwrap();
    let (cancel, token) = cancel_pair();
    let waiting = tokio::spawn(process.wait(token));
    let child = loop {
        if let Some(text) = texts(&out, Stream::Stdout).first() {
            break text.strip_prefix("hijo ").unwrap().parse::<i32>().unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    cancel.cancel();
    let (report, _) = waiting.await.unwrap();
    assert_eq!(report.outcome, ProcessOutcome::Cancelled);
    assert!(report.elapsed < Duration::from_secs(15));
    assert!(!alive_after_grace(root).await);
    assert!(!alive_after_grace(child).await);
}

/// Tras una salida normal no quedan descendientes huérfanos (que además retendrían la tubería).
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn normal_exit_kills_leftover_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("salida");
    let spec = ProcessSpec {
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".into(), "sleep 600 & echo \"hijo $!\"".into()],
        cwd: dir.path().to_path_buf(),
        extra_env: Vec::new(),
        timeout: Duration::from_secs(60),
        grace: Duration::from_millis(500),
    };
    let report = run(spec, &out).await;
    assert_eq!(report.outcome, ProcessOutcome::Exited { code: 0 });
    assert!(report.tree_terminated);
    assert!(report.elapsed < Duration::from_secs(10));
    let child: i32 = texts(&out, Stream::Stdout)[0]
        .strip_prefix("hijo ")
        .unwrap()
        .parse()
        .unwrap();
    assert!(!alive_after_grace(child).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn flood_is_bounded_and_rotated() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("salida");
    let report = run(spec(&["flood", "--megabytes", "6"], dir.path()), &out).await;
    assert_eq!(report.outcome, ProcessOutcome::Exited { code: 0 });
    assert!(report.output.cut_lines > 0, "{:?}", report.output);
    assert!(report.output.cut_bytes > 0);
    assert!(report.output.dropped_files >= 1, "{:?}", report.output);
    assert!(report.output.dropped_bytes > 0);
    assert_eq!(report.output.lost_lines, 0);

    let files: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap())
        .collect();
    assert_eq!(files.len(), MAX_FILES);
    for file in &files {
        assert!(file.metadata().unwrap().len() <= MAX_FILE_BYTES);
    }
    let tail = read_tail(&out, 10_000).unwrap();
    assert_eq!(tail.lines.len(), MAX_TAIL_LINES);
    assert!(tail.earlier_dropped);
    assert_eq!(tail.skipped_corrupt, 0);
    assert!(tail.lines.iter().all(|line| line.text.len() <= 4096));
}

#[tokio::test(flavor = "multi_thread")]
async fn write_in_a_cwd_with_spaces() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("carpeta con espacios");
    std::fs::create_dir(&cwd).unwrap();
    let report = run(spec(&["write"], &cwd), &dir.path().join("salida")).await;
    assert_eq!(report.outcome, ProcessOutcome::Exited { code: 0 });
    let written = std::fs::read_to_string(cwd.join(WRITE_FILE_NAME)).unwrap();
    assert!(written.contains("proceso de prueba"));
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_errors_are_typed_and_return_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("salida");

    let mut missing = spec(&[], dir.path());
    missing.program = dir.path().join("no-existe");
    let failure = RunningProcess::spawn(missing, OutputLog::create(&out).unwrap()).unwrap_err();
    assert_eq!(failure.error.kind, SpawnErrorKind::NotFound);
    assert!(
        !failure
            .error
            .to_string()
            .contains(dir.path().to_str().unwrap())
    );

    let mut relative = spec(&[], dir.path());
    relative.program = PathBuf::from("kanbania-test-process");
    let failure = RunningProcess::spawn(relative, failure.log).unwrap_err();
    assert_eq!(failure.error.kind, SpawnErrorKind::InvalidSpec);

    let mut reserved = spec(&["env"], dir.path());
    reserved.extra_env = vec![("kanbania_agent_token".into(), "x".into())];
    let failure = RunningProcess::spawn(reserved, failure.log).unwrap_err();
    assert_eq!(failure.error.kind, SpawnErrorKind::InvalidSpec);

    let mut no_cwd = spec(&["success"], dir.path());
    no_cwd.cwd = dir.path().join("no-existe");
    let failure = RunningProcess::spawn(no_cwd, failure.log).unwrap_err();
    assert_eq!(failure.error.kind, SpawnErrorKind::InvalidSpec);
}

/// Se vuelve a ejecutar este binario de pruebas con `KANBANIA_*` en el entorno (no se puede
/// cambiar el entorno del propio proceso sin `unsafe`): la sonda comprueba que no llega al hijo.
#[test]
fn environment_is_cleared() {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "env_probe", "--ignored", "--nocapture"])
        .env("KANBANIA_AGENT_TOKEN", "no-debe-llegar")
        .env("OTRA_VARIABLE_DEL_PADRE", "tampoco")
        .status()
        .unwrap();
    assert!(status.success());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "sonda de environment_is_cleared"]
async fn env_probe() {
    assert!(std::env::var_os("KANBANIA_AGENT_TOKEN").is_some());
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("salida");
    let mut spec = spec(&["env"], dir.path());
    spec.extra_env = vec![("EXTRA_PERMITIDA".into(), "1".into())];
    let report = run(spec, &out).await;
    assert_eq!(report.outcome, ProcessOutcome::Exited { code: 0 });
    let names = texts(&out, Stream::Stdout);
    assert!(names.contains(&"EXTRA_PERMITIDA".to_owned()));
    assert!(
        !names
            .iter()
            .any(|n| n.to_ascii_uppercase().starts_with("KANBANIA_")),
        "{names:?}"
    );
    assert!(!names.contains(&"OTRA_VARIABLE_DEL_PADRE".to_owned()));
    #[cfg(unix)]
    for name in &names {
        assert!(
            [
                "PATH",
                "HOME",
                "LANG",
                "LC_ALL",
                "TMPDIR",
                "EXTRA_PERMITIDA",
                "__CF_USER_TEXT_ENCODING"
            ]
            .contains(&name.as_str()),
            "variable inesperada: {name}"
        );
    }
}
