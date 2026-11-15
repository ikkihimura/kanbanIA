// Evita una consola adicional en Windows en release. No eliminar.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use kanbania_runner::test_process;

fn main() {
    // Proceso de prueba (sin IA) que lanza el supervisor de ejecuciones (HU-018) con este mismo
    // ejecutable: se atiende antes de iniciar Tauri y no abre ventana ni datos.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == test_process::RESERVED_FLAG)
    {
        let args: Vec<String> = std::env::args_os()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        std::process::exit(test_process::run(&args));
    }
    kanbania_desktop_lib::run();
}
