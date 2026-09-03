pub mod commands;
pub mod db;
pub mod errors;
pub mod filesystem;
pub mod indexing;
pub mod ocr;
pub mod search;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .register_uri_scheme_protocol("screenshot", |ctx, req| {
            // Parses screenshot id from request URI
            // Handles both "http://screenshot.localhost/42" and "screenshot://localhost/42"
            let path_str = req.uri().path().trim_start_matches('/');
            let id_token = path_str.split('/').next().unwrap_or(path_str);
            let id_clean = id_token.split('.').next().unwrap_or(id_token);

            if let Ok(id) = id_clean.parse::<i64>() {
                if let Some(database) = ctx.app_handle().try_state::<db::connection::Database>() {
                    if let Ok(conn) = database.conn.lock() {
                        let query_result: Result<(String, String), _> = conn.query_row(
                            "SELECT path, extension FROM screenshots WHERE id = ?1",
                            rusqlite::params![id],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        );

                        if let Ok((file_path, extension)) = query_result {
                            let p = std::path::Path::new(&file_path);
                            if p.exists() && p.is_file() {
                                if let Ok(bytes) = std::fs::read(p) {
                                    let mime = match extension.to_lowercase().as_str() {
                                        "png" => "image/png",
                                        "jpg" | "jpeg" => "image/jpeg",
                                        "webp" => "image/webp",
                                        _ => "application/octet-stream",
                                    };

                                    return tauri::http::Response::builder()
                                        .status(tauri::http::StatusCode::OK)
                                        .header("Content-Type", mime)
                                        .header("Access-Control-Allow-Origin", "*")
                                        .header("Cache-Control", "public, max-age=3600")
                                        .body(bytes)
                                        .unwrap();
                                }
                            }
                        }
                    }
                }
            }

            tauri::http::Response::builder()
                .status(tauri::http::StatusCode::NOT_FOUND)
                .header("Content-Type", "text/plain")
                .body(b"Screenshot not found".to_vec())
                .unwrap()
        })
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
            commands::start_ocr_indexing,
            commands::get_ocr_stats,
            commands::get_ocr_engine_info,
            commands::cancel_ocr_indexing,
            commands::search_screenshots,
            commands::get_screenshot,
            commands::get_screenshot_image,
            commands::open_screenshot,
            commands::reveal_screenshot,
            commands::rebuild_search_index,
            commands::check_search_index_health,
        ])
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to resolve app data directory");

            log::info!("App data directory: {}", app_data_dir.display());

            // Initialize database
            let database =
                db::connection::initialize(&app_data_dir).expect("Failed to initialize database");

            // Perform startup recovery on stale PROCESSING jobs
            if let Ok(conn) = database.conn.lock() {
                let _ = db::screenshots::recover_stale_processing(&conn);
            }

            app.manage(database);
            app.manage(ocr::OcrManager::new());

            log::info!("Screenshot Search initialized successfully");

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
