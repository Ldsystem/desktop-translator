// Hide the extra console window on Windows release builds. Debug keeps a
// console so logs stay visible during development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    desktop_translator_lib::run();
}
