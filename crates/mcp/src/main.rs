//! `kanbania-mcp`: servidor MCP por stdio para agentes externos (HU-013).
//!
//! stdout es solo del protocolo MCP: los diagnósticos van a stderr, sin tokens ni rutas. Este
//! proceso no abre SQLite: habla con el servicio local de la app abierta (HU-012) y, si está
//! cerrada, las herramientas responden con un error claro.

#![deny(clippy::print_stdout)]

mod config;
mod tools;

use kanbania_local_service::ServiceClient;
use rmcp::ServiceExt;

use config::Config;
use tools::KanbanTools;

#[tokio::main]
async fn main() {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("[kanbania-mcp] {error}");
            std::process::exit(2);
        }
    };
    let tools = KanbanTools::new(ServiceClient::new(&config.data_dir, config.token));
    let service = match tools.serve(rmcp::transport::stdio()).await {
        Ok(service) => service,
        Err(error) => {
            eprintln!("[kanbania-mcp] no se pudo iniciar la sesión MCP: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = service.waiting().await {
        eprintln!("[kanbania-mcp] la sesión MCP terminó con error: {error}");
        std::process::exit(1);
    }
}
