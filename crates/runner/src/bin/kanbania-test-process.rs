//! Binario del proceso de prueba («Proceso de prueba (sin IA)») para las pruebas automáticas.
//! La app usa la misma lógica con el argumento reservado `--kanbania-test-process`.

fn main() {
    // `args_os` + conversión con pérdida: `args()` abortaría con un argumento no Unicode.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    std::process::exit(kanbania_runner::test_process::run(&args));
}
