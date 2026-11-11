use std::sync::Arc;

use tauri::Manager;

use crate::{
    commands::{
        connections::TauriConnectionEventSink, files::TauriFileTransferEventSink,
        terminal::TauriTerminalEventSink,
    },
    core::{
        credential_index::CredentialIndex,
        credentials::{CredentialStore, OsCredentialStore},
        files::FileTransferManager,
        ssh::{ConnectionManager, ConnectionTimeouts, RusshTransport, TunnelManager},
        terminal::TerminalManager,
    },
    database::DatabaseConnectionRegistry,
};

async fn release_connections(app: &tauri::AppHandle) {
    if let Some(transfers) = app.try_state::<Arc<FileTransferManager>>() {
        transfers.cancel_all();
    }
    if let Some(databases) = app.try_state::<DatabaseConnectionRegistry>() {
        databases.close_all().await;
    }
    if let Some(tunnels) = app.try_state::<Arc<TunnelManager>>() {
        tunnels.close_all().await;
    }
    if let Some(terminal) = app.try_state::<Arc<TerminalManager>>() {
        terminal.close_all().await;
    }
    if let Some(manager) = app.try_state::<Arc<ConnectionManager>>() {
        manager.disconnect_all().await;
    }
}

/// Closes the main window after startup when the packaging smoke test asks for it.
///
/// This exercises window creation, managed-state setup, database migrations, and
/// a clean application exit without requiring desktop automation tools.
fn schedule_smoke_exit(app: &tauri::App) {
    let app_handle = app.handle().clone();

    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));

        let exit_handle = app_handle.clone();
        if app_handle
            .run_on_main_thread(move || {
                if let Some(window) = exit_handle.get_webview_window("main") {
                    eprintln!("SERVERDESK_SMOKE_WINDOW_PRESENT=1");
                    if let Err(error) = window.close() {
                        eprintln!("SERVERDESK_SMOKE_CLOSE_ERROR={error}");
                        exit_handle.exit(2);
                    }
                } else {
                    eprintln!("SERVERDESK_SMOKE_WINDOW_PRESENT=0");
                    exit_handle.exit(2);
                }
            })
            .is_err()
        {
            eprintln!("SERVERDESK_SMOKE_MAIN_THREAD_ERROR=1");
        }
    });
}

/// Runs the uninstall cleanup from the command line.
///
/// Returns a process exit code. This is called before Tauri starts so the
/// cleanup also works from package-manager uninstall scripts.
pub fn purge_local_data_cli() -> i32 {
    let data_directory = match crate::core::purge::default_data_directory() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("ServerDesk cleanup failed: {}", error.message);
            return 1;
        }
    };

    match crate::core::purge::purge_local_data(&data_directory, &OsCredentialStore) {
        Ok(report) => {
            eprintln!(
                "ServerDesk removed {} credential(s) from the OS credential store.",
                report.credential_count
            );
            0
        }
        Err(error) => {
            eprintln!("ServerDesk cleanup failed: {}", error.message);
            1
        }
    }
}

pub mod commands;
pub mod core;
pub mod database;
pub mod storage;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_directory)?;

            let database = storage::Database::open(data_directory.join("serverdesk.sqlite3"))?;
            let schema_version = database.schema_version()?;
            let credential_index = CredentialIndex::open(&data_directory)?;
            app.manage(database);
            app.manage(credential_index);
            app.manage(Arc::new(OsCredentialStore) as Arc<dyn CredentialStore>);

            if std::env::var_os("SERVERDESK_SMOKE_TEST").is_some() {
                eprintln!("SERVERDESK_SMOKE_DATA_DIR={}", data_directory.display());
                eprintln!("SERVERDESK_SMOKE_SCHEMA_VERSION={schema_version}");
                schedule_smoke_exit(app);
            }
            let connections = Arc::new(ConnectionManager::new(
                Arc::new(RusshTransport::new()),
                Arc::new(TauriConnectionEventSink::new(app.handle().clone())),
                ConnectionTimeouts::default(),
            ));
            let tunnels = TunnelManager::new(Arc::clone(&connections));
            app.manage(DatabaseConnectionRegistry::new(Arc::clone(&tunnels)));
            app.manage(Arc::clone(&connections));
            app.manage(tunnels);
            app.manage(Arc::new(FileTransferManager::new(
                connections,
                Arc::new(TauriFileTransferEventSink::new(app.handle().clone())),
            )));
            app.manage(Arc::new(TerminalManager::new(Arc::new(
                TauriTerminalEventSink::new(app.handle().clone()),
            ))));

            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                tauri::async_runtime::block_on(release_connections(window.app_handle()));
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::system::get_app_info,
            commands::preferences::get_preferences,
            commands::preferences::save_preferences,
            commands::servers::list_servers,
            commands::servers::get_server,
            commands::servers::create_server,
            commands::servers::update_server,
            commands::servers::delete_server,
            commands::credentials::create_credential,
            commands::credentials::store_credential,
            commands::credentials::delete_credential,
            commands::credentials::create_private_key_credential,
            commands::credentials::store_private_key_credential,
            commands::connections::connect_server,
            commands::connections::disconnect_server,
            commands::connections::get_connection_state,
            commands::connections::test_server_connection,
            commands::database::capabilities::get_database_capabilities,
            commands::database::discovery::list_database_databases,
            commands::database::discovery::list_database_schemas,
            commands::database::discovery::list_database_tables,
            commands::database::discovery::list_database_views,
            commands::database::discovery::list_database_sequences,
            commands::database::discovery::get_database_object_counts,
            commands::database::discovery::load_database_explorer_node,
            commands::database::discovery::refresh_database_explorer_node,
            commands::database::profiles::list_database_profiles,
            commands::database::profiles::get_database_profile,
            commands::database::profiles::create_database_profile,
            commands::database::profiles::update_database_profile,
            commands::database::profiles::delete_database_profile,
            commands::database::profiles::test_database_profile,
            commands::database::profiles::test_database_profile_draft,
            commands::database::profiles::connect_database_profile,
            commands::database::profiles::reconnect_database_profile,
            commands::database::profiles::disconnect_database_profile,
            commands::database::profiles::get_database_connection_state,
            commands::database::tunnels::open_database_tunnel,
            commands::database::tunnels::close_database_tunnel,
            commands::database::tunnels::get_database_tunnel_state,
            commands::database::tunnels::list_database_tunnels,
            commands::remote::run_remote_command,
            commands::history::list_operation_history,
            commands::history::get_operation_history_policy,
            commands::history::clear_operation_history,
            commands::identity::get_system_identity,
            commands::monitoring::get_monitoring_snapshot,
            commands::services::get_systemd_services,
            commands::services::get_service_status,
            commands::services::run_service_action,
            commands::projects::list_projects,
            commands::projects::browse_remote_directories,
            commands::projects::scan_remote_projects,
            commands::projects::run_project_artisan_action,
            commands::projects::delete_project,
            commands::queue::get_queue_supervisor_status,
            commands::logs::list_log_sources,
            commands::logs::read_log_tail,
            commands::files::browse_remote_files,
            commands::files::upload_remote_file,
            commands::files::download_remote_file,
            commands::files::cancel_file_transfer,
            commands::files::create_remote_folder,
            commands::files::rename_remote_entry,
            commands::files::delete_remote_entry,
            commands::known_hosts::list_known_hosts,
            commands::known_hosts::trust_host_key,
            commands::known_hosts::reject_host_key,
            commands::known_hosts::delete_known_host,
            commands::terminal::open_terminal,
            commands::terminal::terminal_input,
            commands::terminal::resize_terminal,
            commands::terminal::close_terminal,
            commands::terminal::list_terminal_sessions
        ])
        .build(tauri::generate_context!())
        .expect("error while building ServerDesk");

    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            tauri::async_runtime::block_on(release_connections(app_handle));
        }
    });
}
