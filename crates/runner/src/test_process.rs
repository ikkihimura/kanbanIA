//! Proceso de prueba incluido: «Proceso de prueba (sin IA)». No hace trabajo de ningún agente;
//! sirve para comprobar el supervisor (salida, fallo, límite de tiempo, cancelación, hijos y
//! cambios en el worktree) hasta que HU-019 traiga un ejecutor real.
//!
//! Lo ejecutan el binario `kanbania-test-process` y la app con el primer argumento reservado
//! [`RESERVED_FLAG`]. Los hijos se lanzan con el mismo ejecutable y ese mismo argumento, así que
//! funcionan desde ambos.
//!
//! Escenarios (`run(&["slow", "--pids", ruta])`, por ejemplo):
//! - `success`: líneas en stdout y stderr; termina 0.
//! - `fail`: un mensaje en stderr; termina 3.
//! - `slow [--pids <archivo>] [--seconds <n>]`: crea 2 hijos que también duermen, imprime cada
//!   segundo y dura `n` segundos (600 por defecto). Imprime `pids: <propio> <hijo> <hijo>` y,
//!   con `--pids`, los escribe también (uno por línea) en el archivo antes de empezar. Con
//!   `--heartbeat <carpeta>`, él (`padre`) y sus hijos (`hijo-0`, `hijo-1`) reescriben allí un
//!   contador cada 200 ms: si deja de cambiar, el proceso ya no corre (prueba portable).
//! - `write`: crea o amplía `kanbania-prueba.txt` en la carpeta de trabajo; termina 0.
//! - `flood [--megabytes <n>]`: unos `n` MiB (6 por defecto) de salida, con líneas muy largas.
//! - `hang [--seconds <n>]`: no imprime nada y duerme (600 s por defecto).
//! - `env`: imprime los nombres (nunca los valores) de sus variables de entorno.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Primer argumento que reserva la app para ejecutar el proceso de prueba.
pub const RESERVED_FLAG: &str = "--kanbania-test-process";
/// Archivo que crea o amplía el escenario `write`.
pub const WRITE_FILE_NAME: &str = "kanbania-prueba.txt";
/// Cada cuánto escriben su latido `slow` y sus hijos con `--heartbeat`.
pub const HEARTBEAT: Duration = Duration::from_millis(200);
/// Prefijo de la línea de `slow` con su pid y el de sus hijos.
pub const PIDS_PREFIX: &str = "pids: ";
/// Código de salida del escenario `fail`.
pub const FAIL_EXIT_CODE: i32 = 3;

const USAGE: &str = "uso: (success | fail | slow [--pids <archivo>] [--heartbeat <carpeta>] [--seconds <n>] | write | flood [--megabytes <n>] | hang [--seconds <n>] | env)";

/// Ejecuta el escenario y devuelve el código de salida. Acepta (y quita) [`RESERVED_FLAG`] como
/// primer argumento.
pub fn run(args: &[String]) -> i32 {
    let args = match args.first() {
        Some(first) if first == RESERVED_FLAG => &args[1..],
        _ => args,
    };
    let Some((scenario, rest)) = args.split_first() else {
        err(USAGE);
        return 2;
    };
    match scenario.as_str() {
        "success" => success(),
        "fail" => fail(),
        "slow" => slow(rest),
        "write" => write(),
        "flood" => flood(rest),
        "hang" => sleep_quietly(rest),
        "env" => env_names(),
        "child" => child(rest),
        _ => {
            err(USAGE);
            2
        }
    }
}

fn success() -> i32 {
    out("Proceso de prueba (sin IA): inicio");
    for step in 1..=3 {
        out(&format!("paso {step} de 3"));
    }
    err("aviso de prueba en stderr");
    out("Proceso de prueba (sin IA): fin");
    0
}

fn fail() -> i32 {
    out("Proceso de prueba (sin IA): va a fallar");
    err("error simulado del proceso de prueba");
    FAIL_EXIT_CODE
}

fn slow(args: &[String]) -> i32 {
    let seconds = option_u64(args, "--seconds").unwrap_or(600);
    let pids_file = option(args, "--pids");
    let heartbeat = option(args, "--heartbeat").map(std::path::PathBuf::from);
    let Ok(exe) = std::env::current_exe() else {
        err("no se pudo localizar el ejecutable");
        return 1;
    };
    let mut children = Vec::new();
    for index in 0..2 {
        // Los hijos heredan stdout/stderr: también mantienen abiertas las tuberías.
        let mut command = Command::new(&exe);
        command
            .args([RESERVED_FLAG, "child", "--seconds", &seconds.to_string()])
            .stdin(Stdio::null());
        if let Some(dir) = &heartbeat {
            command
                .arg("--heartbeat")
                .arg(dir.join(format!("hijo-{index}")));
        }
        let spawned = command.spawn();
        match spawned {
            Ok(child) => children.push(child),
            Err(_) => {
                err("no se pudo crear un proceso hijo");
                for mut child in children {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                return 1;
            }
        }
    }
    if let Some(path) = pids_file {
        let mut pids = format!("{}\n", std::process::id());
        for child in &children {
            pids.push_str(&format!("{}\n", child.id()));
        }
        // Escritura atómica: quien lee nunca ve el archivo a medias.
        let temporary = format!("{path}.tmp");
        if std::fs::write(&temporary, pids).is_err() || std::fs::rename(&temporary, path).is_err() {
            err("no se pudo escribir el archivo de pids");
        }
    }
    // Pids en la salida, para comprobar desde fuera que la cancelación mata a todos.
    let child_pids: Vec<String> = children.iter().map(|c| c.id().to_string()).collect();
    out(&format!(
        "{PIDS_PREFIX}{} {}",
        std::process::id(),
        child_pids.join(" ")
    ));
    out("Proceso de prueba (sin IA): 2 hijos en marcha");
    let parent_beat = heartbeat.as_ref().map(|dir| dir.join("padre"));
    let mut beat: u64 = 0;
    for second in 1..=seconds {
        for _ in 0..5 {
            if let Some(path) = &parent_beat {
                beat += 1;
                let _ = std::fs::write(path, beat.to_string());
            }
            std::thread::sleep(HEARTBEAT);
        }
        out(&format!("progreso {second}/{seconds}"));
    }
    for mut child in children {
        let _ = child.wait();
    }
    0
}

fn write() -> i32 {
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(WRITE_FILE_NAME)
        .and_then(|mut file| file.write_all(b"cambio del proceso de prueba (sin IA)\n"));
    match result {
        Ok(()) => {
            out("archivo de prueba escrito en la carpeta de trabajo");
            0
        }
        Err(_) => {
            err("no se pudo escribir el archivo de prueba");
            1
        }
    }
}

fn flood(args: &[String]) -> i32 {
    let total = option_u64(args, "--megabytes").unwrap_or(6) * 1024 * 1024;
    let stdout = std::io::stdout();
    let mut stdout = std::io::BufWriter::new(stdout.lock());
    let long = "L".repeat(10 * 1024);
    let mut written: u64 = 0;
    let mut line: u64 = 0;
    while written < total {
        line += 1;
        let text = if line.is_multiple_of(1000) {
            format!("{line} {long}\n")
        } else {
            format!("linea {line} {}\n", "x".repeat(90))
        };
        if stdout.write_all(text.as_bytes()).is_err() {
            return 1;
        }
        written += text.len() as u64;
        if line.is_multiple_of(500) {
            err(&format!("stderr {line}"));
        }
    }
    if stdout.flush().is_err() {
        return 1;
    }
    0
}

/// Hijo de `slow`: duerme y, con `--heartbeat <archivo>`, reescribe el archivo cada 200 ms.
fn child(args: &[String]) -> i32 {
    let Some(path) = option(args, "--heartbeat") else {
        return sleep_quietly(args);
    };
    let seconds = option_u64(args, "--seconds").unwrap_or(600);
    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    let mut beat: u64 = 0;
    while std::time::Instant::now() < deadline {
        beat += 1;
        let _ = std::fs::write(path, beat.to_string());
        std::thread::sleep(HEARTBEAT);
    }
    0
}

fn sleep_quietly(args: &[String]) -> i32 {
    let seconds = option_u64(args, "--seconds").unwrap_or(600);
    std::thread::sleep(Duration::from_secs(seconds));
    0
}

fn env_names() -> i32 {
    let mut names: Vec<String> = std::env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        out(&name);
    }
    0
}

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|at| args.get(at + 1))
        .map(String::as_str)
}

fn option_u64(args: &[String], name: &str) -> Option<u64> {
    option(args, name).and_then(|value| value.parse().ok())
}

/// Escribe una línea ignorando errores (una tubería cerrada no debe abortar el proceso, a
/// diferencia de `println!`).
fn out(line: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

fn err(line: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}
