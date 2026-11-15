//! Agente ACP falso **solo para pruebas** de contrato (HU-019). Vive solo en este bin (no en la
//! librería): la app y el host nunca lo enlazan ni lo lanzan.

mod scenarios;

fn main() {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    std::process::exit(scenarios::run(&args));
}
