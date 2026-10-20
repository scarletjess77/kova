// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The same executable is the Kova service (`--service-run`) and its installer.
    if let Some(code) = kova_service::dispatch_cli() {
        std::process::exit(code);
    }
    kova_app::run()
}
