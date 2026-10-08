//! nucleus desktop app: a Tauri shell over the harness.

pub mod commands;
pub mod core;
pub mod settings;

use std::sync::Arc;

use tauri::{Emitter, Manager};

/// Event the frontend listens on (`EVENT_NAME` in `src/api/tauri.ts`).
pub const EVENT_NAME: &str = "nucleus://event";

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = match std::env::var_os("NUCLEUS_DATA_DIR") {
                Some(d) => d.into(),
                None => app.path().app_data_dir()?,
            };
            let handle = app.handle().clone();
            let sink: nucleus_harness::EventSink = Arc::new(move |event| {
                if let Err(e) = handle.emit(EVENT_NAME, &event) {
                    tracing::warn!("emitting event failed: {e}");
                }
            });
            app.manage(core::AppCore::new(
                data_dir,
                core::AppCore::engine_connector(),
                sink,
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::init,
            commands::get_state,
            commands::update_settings,
            commands::pick_directory,
            commands::add_workspace,
            commands::remove_workspace,
            commands::configure_workspace,
            commands::available_templates,
            commands::template_status,
            commands::build_templates,
            commands::branches,
            commands::graph,
            commands::diff,
            commands::create_conversation,
            commands::rename_conversation,
            commands::delete_conversation,
            commands::send_message,
            commands::cancel,
            commands::transcript,
            commands::conversation_diff,
            commands::unmerged_commits,
            commands::merge_conversation,
            commands::proposals,
            commands::proposal,
            commands::approve,
            commands::reject,
            commands::history,
            commands::revert,
            commands::skills,
            commands::cleanup_orphans,
            commands::build_agent_image,
        ])
        .run(tauri::generate_context!())
        .expect("error while running nucleus");
}
