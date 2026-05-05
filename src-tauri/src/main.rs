// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod error;
mod server;
mod types;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let handle = app.handle().clone();
            // Start the Axum HTTP server alongside the Tauri app
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server::start_server(handle).await {
                    eprintln!("Failed to start Axum server: {}", e);
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
