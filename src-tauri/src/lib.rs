use tauri::Manager;

pub mod commands;
pub mod core;
pub mod storage;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_directory)?;

            let database = storage::Database::open(data_directory.join("serverdesk.sqlite3"))?;
            app.manage(database);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::system::get_app_info,
            commands::preferences::get_preferences,
            commands::preferences::save_preferences
        ])
        .run(tauri::generate_context!())
        .expect("error while running ServerDesk");
}
