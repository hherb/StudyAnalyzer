//! The Tauri command layer: a thin shell over `studyanalyzer-core`.

/// Builds and runs the app with the dialog and log plugins.
///
/// Panics if Tauri cannot start, which leaves nothing to report to.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_log::Builder::default().build())
        .run(tauri::generate_context!())
        .expect("error while running StudyAnalyzer");
}
