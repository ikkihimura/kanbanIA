use serde::Serialize;
use ts_rs::TS;

use crate::contract::CONTRACT_VERSION;

/// Datos que solo conoce el host que ejecuta el núcleo.
#[derive(Debug, Clone, Copy)]
pub struct HostInfo<'a> {
    pub product_name: &'a str,
    pub app_version: &'a str,
}

/// Información de versión expuesta a los transportes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub product_name: String,
    pub app_version: String,
    pub core_version: String,
    pub contract_version: u32,
    pub os: String,
    pub arch: String,
}

pub fn get_app_info(host: HostInfo<'_>) -> AppInfo {
    AppInfo {
        product_name: host.product_name.to_owned(),
        app_version: host.app_version.to_owned(),
        core_version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: CONTRACT_VERSION,
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_host_data_with_core_and_contract_versions() {
        let info = get_app_info(HostInfo {
            product_name: "Kanban IA",
            app_version: "9.8.7",
        });

        assert_eq!(info.product_name, "Kanban IA");
        assert_eq!(info.app_version, "9.8.7");
        assert_eq!(info.core_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(info.contract_version, CONTRACT_VERSION);
        assert_eq!(info.os, std::env::consts::OS);
        assert_eq!(info.arch, std::env::consts::ARCH);
    }

    #[test]
    fn serializes_with_camel_case_keys_for_the_frontend() {
        let info = get_app_info(HostInfo {
            product_name: "Kanban IA",
            app_version: "0.1.0",
        });
        let value = serde_json::to_value(&info).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();

        assert_eq!(
            keys,
            [
                "appVersion",
                "arch",
                "contractVersion",
                "coreVersion",
                "os",
                "productName"
            ]
        );
    }
}
