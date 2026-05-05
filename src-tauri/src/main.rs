// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod agent_manager;
pub mod credential_manager;
mod db;
mod db_ops;
pub mod policy_manager;
#[cfg(test)]
mod db_property_tests;
mod error;
pub mod kit_generator;
pub mod persona_manager;
pub mod pty_bridge;
pub mod sbx;
#[cfg(test)]
mod sbx_property_tests;
mod server;
pub mod session_manager;
pub mod template_manager;
mod types;
pub mod workspace_manager;

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
