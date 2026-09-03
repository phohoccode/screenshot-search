pub mod commands;
pub mod db;
pub mod errors;
pub mod filesystem;
pub mod indexing;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::check_database,
            commands::list_folders,
            commands::add_folder,
            commands::remove_folder,
            commands::scan_folder,
            commands::pick_folder,
            commands::get_total_screenshot_count,
        ])
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to resolve app data directory");

            log::info!(
                "App data directory: {}",
                app_data_dir.display()
            );

            // Initialize database
            let database = db::connection::initialize(&app_data_dir)
                .expect("Failed to initialize database");

            app.manage(database);

            log::info!("Screenshot Search initialized successfully");

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
