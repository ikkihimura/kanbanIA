use kanbania_core::features::app_info::{self, AppInfo, HostInfo};
use tauri::{AppHandle, Runtime};

#[tauri::command]
pub fn get_app_info<R: Runtime>(app: AppHandle<R>) -> AppInfo {
    let package = app.package_info();
    let app_version = package.version.to_string();

    app_info::get_app_info(HostInfo {
        product_name: &package.name,
        app_version: &app_version,
    })
}
