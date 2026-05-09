// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod agent_manager;
pub mod credential_manager;
mod db;
mod db_ops;
pub mod export_import_manager;
pub mod policy_manager;
#[cfg(test)]
mod db_property_tests;
mod error;
pub mod kit_generator;
pub mod mcp_container_manager;
pub mod persona_manager;
pub mod port_allocator;
pub mod pty_bridge;
pub mod routes;
pub mod sbx;
#[cfg(test)]
mod port_allocator_property_tests;
#[cfg(test)]
mod sbx_property_tests;
#[cfg(test)]
mod mcp_container_property_tests;
#[cfg(test)]
mod export_import_property_tests;
#[cfg(test)]
mod token_property_tests;
mod server;
pub mod session_manager;
pub mod system_manager;
pub mod template_manager;
pub mod token;
mod types;
pub mod workspace_manager;

use std::sync::{Arc, OnceLock};
use mcp_container_manager::McpContainerManager;

/// Global reference to the MCP container manager for shutdown cleanup.
static MCP_MANAGER: OnceLock<Arc<McpContainerManager>> = OnceLock::new();

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
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
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                // Stop all MCP containers on app exit
                if let Some(mgr) = MCP_MANAGER.get() {
                    let mgr = mgr.clone();
                    // Use a blocking runtime since we're in the exit handler
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build();
                    if let Ok(rt) = rt {
                        rt.block_on(async {
                            if let Err(e) = mgr.stop_all().await {
                                eprintln!("Warning: failed to stop MCP containers on exit: {}", e);
                            }
                        });
                    }
                }
            }
        });
}
