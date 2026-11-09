// Evita una consola adicional en Windows en release. No eliminar.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    kanbania_desktop_lib::run();
}
