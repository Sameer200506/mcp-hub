//! Universal MCP Manager — library entry point.
//!
//! Local-first desktop application that keeps a central registry of MCP server
//! definitions and synchronizes them into the configuration files of coding
//! agent harnesses (OpenCode, Cline, Google Antigravity and user-configured
//! generic JSON profiles).
//!
//! The frontend never touches the filesystem: every read and every mutation is
//! performed by the Rust commands in [`commands`].

pub mod adapters;
pub mod commands;
pub mod database;
pub mod errors;
pub mod models;
pub mod security;
pub mod services;
pub mod state;

use std::path::{Path, PathBuf};

use tauri::Manager;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::database::Db;
use crate::models::AppSettings;
use crate::state::AppState;

/// Settings are persisted in SQLite under this key.
const SETTINGS_KEY: &str = "settings";
/// Database file name inside the application data directory.
const DATABASE_FILE: &str = "registry.sqlite3";
/// Base name of the rotating log file.
const LOG_FILE: &str = "universal-mcp-manager.log";

/// Start the desktop application.
pub fn run() {
    let data_dir = database::default_data_dir();
    if let Err(error) = std::fs::create_dir_all(&data_dir) {
        eprintln!(
            "Universal MCP Manager could not create {}: {error}",
            data_dir.display()
        );
    }

    let db_path = data_dir.join(DATABASE_FILE);
    let db = match Db::open(&db_path) {
        Ok(db) => db,
        Err(error) => {
            eprintln!(
                "Universal MCP Manager could not open its registry database at {}: {}",
                db_path.display(),
                error.message
            );
            std::process::exit(1);
        }
    };

    let settings = match db.get_setting_json::<AppSettings>(SETTINGS_KEY) {
        Ok(Some(stored)) => stored,
        Ok(None) => {
            let defaults = AppSettings::defaults();
            if let Err(error) = db.set_setting_json(SETTINGS_KEY, &defaults) {
                eprintln!("Could not persist the default settings: {}", error.message);
            }
            defaults
        }
        Err(error) => {
            eprintln!(
                "The stored settings could not be read ({}); starting from defaults.",
                error.message
            );
            AppSettings::defaults()
        }
    };

    let log_file = data_dir.join("logs").join(LOG_FILE);
    let log_guard = init_logging(&data_dir, &settings.logging_level);
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        database = %db_path.display(),
        "Universal MCP Manager starting"
    );

    let mut app_state = AppState::new(db, settings, log_file);
    app_state.log_guard = log_guard;

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .setup(|app| {
            let handle = app.handle().clone();
            let state = app.state::<AppState>();
            if let Ok(mut slot) = state.refs.app.lock() {
                *slot = Some(handle);
            }
            if let Err(error) = state.refs.restart_watcher() {
                tracing::warn!(
                    message = %error.message,
                    "the configuration file watcher could not be started"
                );
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // registry
            commands::list_mcp_servers,
            commands::get_mcp_server,
            commands::create_mcp_server,
            commands::update_mcp_server,
            commands::delete_mcp_server,
            commands::duplicate_mcp_server,
            commands::set_mcp_enabled,
            commands::validate_mcp_config,
            commands::preview_definition,
            commands::test_mcp_connection,
            // secrets
            commands::list_secret_keys,
            commands::set_secret,
            commands::delete_secret,
            commands::has_secret,
            // harnesses
            commands::list_harnesses,
            commands::detect_harnesses,
            commands::get_harness_config,
            commands::get_harness_config_at_path,
            commands::list_harness_entries,
            commands::open_harness_config_dir,
            // synchronization
            commands::preview_sync,
            commands::sync_one,
            commands::sync_all,
            commands::get_sync_history,
            // backups
            commands::list_backups,
            commands::restore_backup,
            // import / export
            commands::export_registry,
            commands::import_registry,
            // settings & diagnostics
            commands::get_app_settings,
            commands::update_app_settings,
            commands::reset_app_settings,
            commands::get_dashboard_stats,
            commands::get_log_file_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Universal MCP Manager");
}

/// Configure logging to a daily-rotating file inside the data directory.
///
/// Returns the non-blocking writer guard, which must be kept alive for the
/// process lifetime or buffered lines are lost.
fn init_logging(data_dir: &Path, level: &str) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("universal_mcp_manager_lib={level},tauri=warn,notify=warn")));

    let log_dir: PathBuf = data_dir.join("logs");
    match std::fs::create_dir_all(&log_dir) {
        Ok(()) => {
            let appender = tracing_appender::rolling::daily(&log_dir, LOG_FILE);
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let subscriber = tracing_subscriber::registry().with(filter).with(
                tracing_subscriber::fmt::layer()
                    .with_writer(writer)
                    .with_ansi(false),
            );
            if subscriber.try_init().is_err() {
                return None;
            }
            Some(guard)
        }
        Err(_) => {
            // Fall back to stderr: the app still runs, it just cannot keep a log.
            let subscriber = tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr));
            let _ = subscriber.try_init();
            None
        }
    }
}
