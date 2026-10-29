// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Desktop entry point: a thin wrapper around the library's `run`.

fn main() {
    studyanalyzer_lib::run()
}
