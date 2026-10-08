//! nucleus desktop app: a Tauri shell over the harness.

pub mod commands;
pub mod core;
pub mod logging;
pub mod settings;

use std::sync::Arc;

use tauri::{Emitter, Manager};

struct LogGuard(#[allow(dead_code)] std::sync::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>);

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
            let logs = logging::LogBuffer::new(2000);
            if let Some(guard) = logging::init(&data_dir, logs.clone()) {
                // Keep the file writer alive for the life of the app.
                app.manage(LogGuard(std::sync::Mutex::new(Some(guard))));
            }
            app.manage(logs);
            tracing::info!(data_dir = %data_dir.display(), "nucleus starting");
            let handle = app.handle().clone();
            let sink: nucleus_harness::EventSink = Arc::new(move |event| {
                if let Err(e) = handle.emit(EVENT_NAME, &event) {
                    tracing::warn!("emitting event failed: {e}");
                }
            });
            app.manage(core::AppCore::new(data_dir, core::AppCore::engine_connector(), sink));
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
            commands::recent_logs,
            commands::restart_sandbox,
            commands::mark_last_turn_wrong,
            commands::propose_skill,
            commands::propose_skill_removal,
            commands::propose_template,
            commands::tools,
            commands::skill_source,
            commands::template_source,
        ])
        .run(tauri::generate_context!())
        .expect("error while running nucleus");
}
