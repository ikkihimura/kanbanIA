//! Binario del proceso de prueba («Proceso de prueba (sin IA)») para las pruebas automáticas.
//! La app usa la misma lógica con el argumento reservado `--kanbania-test-process`. También
//! atiende el guardián (`--kanbania-guard`, HU-024), como la app.

fn main() {
    let raw: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if raw
        .first()
        .is_some_and(|arg| arg == kanbania_runner::guard::GUARD_FLAG)
    {
        std::process::exit(kanbania_runner::guard::run(&raw));
    }
    // `args_os` + conversión con pérdida: `args()` abortaría con un argumento no Unicode.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    std::process::exit(kanbania_runner::test_process::run(&args));
}
