use std::sync::Arc;

use tauri::Manager;

use crate::{
    commands::connections::TauriConnectionEventSink,
    core::{
        credentials::{CredentialStore, OsCredentialStore},
        ssh::{ConnectionManager, ConnectionTimeouts, RusshTransport},
    },
};

pub mod commands;
pub mod core;
pub mod storage;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_directory)?;

            let database = storage::Database::open(data_directory.join("serverdesk.sqlite3"))?;
            app.manage(database);
            app.manage(Arc::new(OsCredentialStore) as Arc<dyn CredentialStore>);
            app.manage(Arc::new(ConnectionManager::new(
                Arc::new(RusshTransport::new()),
                Arc::new(TauriConnectionEventSink::new(app.handle().clone())),
                ConnectionTimeouts::default(),
            )));

            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. })
                && let Some(manager) = window.app_handle().try_state::<Arc<ConnectionManager>>()
            {
                tauri::async_runtime::block_on(manager.disconnect_all());
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
            commands::remote::run_remote_command,
            commands::identity::get_system_identity,
            commands::monitoring::get_monitoring_snapshot,
            commands::known_hosts::list_known_hosts,
            commands::known_hosts::trust_host_key,
            commands::known_hosts::reject_host_key,
            commands::known_hosts::delete_known_host
        ])
        .build(tauri::generate_context!())
        .expect("error while building ServerDesk");

    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit)
            && let Some(manager) = app_handle.try_state::<Arc<ConnectionManager>>()
        {
            tauri::async_runtime::block_on(manager.disconnect_all());
        }
    });
}
