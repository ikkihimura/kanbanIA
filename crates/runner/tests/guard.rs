//! Guardián de los procesos (HU-024): si la app muere de golpe, el árbol del intento termina, y
//! nunca se mata un proceso ajeno.
//!
//! La «app» es esta misma prueba relanzada como proceso aparte ([`app_helper`], con
//! `KANBANIA_GUARD_HELPER`): lanza procesos con el runner real (`RunningProcess` + guardián) y
//! la prueba la mata con `SIGKILL`.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use kanbania_runner::guard::{EXIT_GUARD_ERROR, EXIT_NOT_FOUND, GUARD_FLAG, GuardLauncher};
use kanbania_runner::output::OutputLog;
use kanbania_runner::process::{ProcessOutcome, ProcessSpec, RunningProcess, cancel_pair};
use kanbania_runner::test_process::{FAIL_EXIT_CODE, WRITE_FILE_NAME};
use nix::sys::signal::{Signal, kill, killpg};
use nix::unistd::Pid;

const TEST_PROCESS: &str = env!("CARGO_BIN_EXE_kanbania-test-process");
const HELPER_ENV: &str = "KANBANIA_GUARD_HELPER";
const GRACE: Duration = Duration::from_millis(800);

fn guarded(program: &str, args: &[&str], cwd: &Path) -> ProcessSpec {
    ProcessSpec {
        program: PathBuf::from(program),
        args: args.iter().map(OsString::from).collect(),
        cwd: cwd.to_path_buf(),
        extra_env: Vec::new(),
        timeout: Duration::from_secs(600),
        grace: GRACE,
        guard: Some(GuardLauncher::new(TEST_PROCESS.into())),
    }
}

async fn wait_for_file(path: &Path) -> String {
    let started = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && !text.is_empty()
        {
            return text;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "falta {path:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn pids(text: &str) -> Vec<i32> {
    text.lines().map(|l| l.trim().parse().unwrap()).collect()
}

/// `true` si el pid ya no existe tras esperar como mucho `limit`.
async fn gone_within(pid: i32, limit: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if kill(Pid::from_raw(pid), None).is_err() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// La «app» que la prueba mata. Sin `KANBANIA_GUARD_HELPER` no hace nada.
///
/// Lanza con el guardián:
/// - `slow` (padre + 2 hijos, con latidos en `<dir>/latidos`);
/// - un shell que ignora `SIGTERM` y añade una línea a `<dir>/terco` cada 200 ms (solo
///   `SIGKILL` lo para);
///
/// y, sin guardián y en un grupo propio, un `slow` «ajeno» que no debe morir. Escribe
/// `<dir>/listo` y espera a que la maten.
#[tokio::test(flavor = "multi_thread")]
async fn app_helper() {
    let Some(dir) = std::env::var_os(HELPER_ENV).map(PathBuf::from) else {
        return;
    };
    let beats = dir.join("latidos");
    std::fs::create_dir_all(&beats).unwrap();
    let mut keep = Vec::new();

    let spec = guarded(
        TEST_PROCESS,
        &[
            "slow",
            "--pids",
            dir.join("pids").to_str().unwrap(),
            "--heartbeat",
            beats.to_str().unwrap(),
        ],
        &dir,
    );
    keep.push(
        RunningProcess::spawn(spec, OutputLog::create(&dir.join("salida-1")).unwrap()).unwrap(),
    );

    let stubborn = format!(
        "trap '' TERM; echo $$ > '{}'; while :; do echo x >> '{}'; sleep 0.2; done",
        dir.join("terco-pid").display(),
        dir.join("terco").display()
    );
    let spec = guarded("/bin/sh", &["-c", &stubborn], &dir);
    keep.push(
        RunningProcess::spawn(spec, OutputLog::create(&dir.join("salida-2")).unwrap()).unwrap(),
    );

    // Modo protocolo: el programa termina en cuanto su entrada se cierra (al morir la app) y
    // deja un nieto en el grupo, que también debe morir.
    let on_eof = format!(
        "sleep 600 & echo $! > '{}'; read _; exit 0",
        dir.join("nieto").display()
    );
    let mut spec = guarded("/bin/sh", &["-c", &on_eof], &dir);
    spec.extra_env = Vec::new();
    let (process, io) =
        RunningProcess::spawn_protocol(spec, OutputLog::create(&dir.join("salida-3")).unwrap())
            .unwrap();
    keep.push(process);
    std::mem::forget(io);

    // Proceso ajeno: mismo usuario, mismo padre, otro grupo.
    use std::os::unix::process::CommandExt;
    let foreign = Command::new(TEST_PROCESS)
        .args(["slow", "--pids"])
        .arg(dir.join("ajeno"))
        .process_group(0)
        .spawn()
        .unwrap();
    std::mem::forget(foreign);

    // Los guardianes deben estar ya vigilando: el de cada proceso es su raíz.
    let roots: Vec<String> = keep.iter().map(|p| p.pid().unwrap().to_string()).collect();
    std::fs::write(dir.join("guardianes"), roots.join("\n")).unwrap();
    std::fs::write(dir.join("listo"), "1").unwrap();
    std::mem::forget(keep);
    tokio::time::sleep(Duration::from_secs(600)).await;
}

/// Si la prueba falla a medias, no deja vivos la «app» ni el proceso ajeno.
/// Posee el `Child` de la «app»: una vez recogido, `kill` ya no envía nada (su pid podría ser de
/// otro proceso).
struct Cleanup {
    app: std::process::Child,
    foreign: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = self.app.kill();
        let _ = self.app.wait();
        if let Ok(text) = std::fs::read_to_string(&self.foreign)
            && let Some(Ok(leader)) = text.lines().next().map(str::parse::<i32>)
        {
            let _ = killpg(Pid::from_raw(leader), Signal::SIGKILL);
        }
    }
}

/// CA-1: la app muere con `SIGKILL` → el guardián termina todo el árbol (también un proceso
/// que ignora `SIGTERM`), y un proceso de otro grupo sigue vivo.
#[tokio::test(flavor = "multi_thread")]
async fn killing_the_app_terminates_the_tree_but_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().to_path_buf();
    let app = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "app_helper", "--nocapture", "--test-threads=1"])
        .env(HELPER_ENV, &work)
        .spawn()
        .unwrap();
    let mut cleanup = Cleanup {
        app,
        foreign: work.join("ajeno"),
    };

    wait_for_file(&work.join("listo")).await;
    let tree = pids(&wait_for_file(&work.join("pids")).await);
    let guardians = pids(&wait_for_file(&work.join("guardianes")).await);
    let stubborn = pids(&wait_for_file(&work.join("terco-pid")).await);
    let grandchild = pids(&wait_for_file(&work.join("nieto")).await);
    let foreign = pids(&wait_for_file(&work.join("ajeno")).await);
    assert_eq!(tree.len(), 3);
    assert_eq!(guardians.len(), 3);
    common::wait_heartbeats_alive(&work.join("latidos")).await;

    // `Child::kill` = SIGKILL a la app aún sin recoger.
    cleanup.app.kill().unwrap();
    cleanup.app.wait().unwrap();

    // Sondeo cada 100 ms + gracia + margen.
    let limit = GRACE + Duration::from_secs(5);
    for pid in tree
        .iter()
        .chain(&guardians)
        .chain(&stubborn)
        .chain(&grandchild)
    {
        assert!(gone_within(*pid, limit).await, "sigue vivo: {pid}");
    }
    common::assert_heartbeats_stopped(&work.join("latidos")).await;
    let lines = file_len(&work.join("terco"));
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        file_len(&work.join("terco")),
        lines,
        "el proceso terco sigue"
    );

    // El ajeno (otro grupo) sigue vivo; después se limpia.
    for pid in &foreign {
        assert!(
            kill(Pid::from_raw(*pid), None).is_ok(),
            "se mató un ajeno: {pid}"
        );
    }
}

async fn run_guarded(spec: ProcessSpec, dir: &Path) -> ProcessOutcome {
    let process =
        RunningProcess::spawn(spec, OutputLog::create(&dir.join("salida")).unwrap()).unwrap();
    let (_cancel, token) = cancel_pair();
    process.wait(token).await.0.outcome
}

/// Con guardián, el informe sigue siendo el del programa: código, señal y «no encontrado».
#[tokio::test(flavor = "multi_thread")]
async fn the_guard_reports_how_the_program_ended() {
    let dir = tempfile::tempdir().unwrap();
    let ok = run_guarded(guarded(TEST_PROCESS, &["success"], dir.path()), dir.path()).await;
    assert_eq!(ok, ProcessOutcome::Exited { code: 0 });
    let failed = run_guarded(guarded(TEST_PROCESS, &["fail"], dir.path()), dir.path()).await;
    assert_eq!(
        failed,
        ProcessOutcome::Exited {
            code: FAIL_EXIT_CODE
        }
    );
    let signaled = run_guarded(
        guarded("/bin/sh", &["-c", "kill -9 $$"], dir.path()),
        dir.path(),
    )
    .await;
    assert_eq!(signaled, ProcessOutcome::Signaled { signal: 9 });
    // Las señales de fallo no se reproducen en el guardián (macOS lo atribuiría a la app), ni
    // `SIGPIPE` (Rust la ignora): 128 + señal.
    let crashed = run_guarded(
        guarded("/bin/sh", &["-c", "kill -SEGV $$"], dir.path()),
        dir.path(),
    )
    .await;
    assert_eq!(crashed, ProcessOutcome::Exited { code: 128 + 11 });
    let piped = run_guarded(
        guarded("/bin/sh", &["-c", "kill -PIPE $$"], dir.path()),
        dir.path(),
    )
    .await;
    assert_eq!(piped, ProcessOutcome::Exited { code: 128 + 13 });
    let missing = run_guarded(
        guarded("/no/existe/kanbania-programa", &[], dir.path()),
        dir.path(),
    )
    .await;
    assert_eq!(
        missing,
        ProcessOutcome::Exited {
            code: EXIT_NOT_FOUND
        }
    );
}

/// Cancelar con guardián: el programa recibe `SIGTERM` y el árbol entero termina.
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_guarded_process_terminates_the_tree() {
    let dir = tempfile::tempdir().unwrap();
    let beats = dir.path().join("latidos");
    std::fs::create_dir(&beats).unwrap();
    let spec = guarded(
        TEST_PROCESS,
        &["slow", "--heartbeat", beats.to_str().unwrap()],
        dir.path(),
    );
    let process =
        RunningProcess::spawn(spec, OutputLog::create(&dir.path().join("salida")).unwrap())
            .unwrap();
    let (cancel, token) = cancel_pair();
    let waiting = tokio::spawn(process.wait(token));
    common::wait_heartbeats_alive(&beats).await;
    cancel.cancel();
    let report = waiting.await.unwrap().0;
    assert_eq!(report.outcome, ProcessOutcome::Cancelled);
    common::assert_heartbeats_stopped(&beats).await;
}

/// Si la app ya no es su padre al arrancar (murió antes), el guardián no lanza nada.
#[test]
fn the_guard_launches_nothing_if_the_app_is_already_gone() {
    let dir = tempfile::tempdir().unwrap();
    // El padre del guardián será esta prueba, no el pid indicado.
    let not_my_parent = std::os::unix::process::parent_id().to_string();
    let status = Command::new(TEST_PROCESS)
        .args([
            GUARD_FLAG,
            &not_my_parent,
            "500",
            "--",
            TEST_PROCESS,
            "write",
        ])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(EXIT_GUARD_ERROR));
    assert!(!dir.path().join(WRITE_FILE_NAME).exists());

    // Con el padre correcto, sí.
    let status = Command::new(TEST_PROCESS)
        .args([
            GUARD_FLAG,
            &std::process::id().to_string(),
            "500",
            "--",
            TEST_PROCESS,
            "write",
        ])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0));
    assert!(dir.path().join(WRITE_FILE_NAME).exists());
}
