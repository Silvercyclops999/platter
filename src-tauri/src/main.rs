// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The speech helper process: no window, no tray, nothing of the app.
    if let Some(code) = platter_lib::speech_worker(&std::env::args().collect::<Vec<_>>()) {
        std::process::exit(code);
    }
    platter_lib::run()
}
