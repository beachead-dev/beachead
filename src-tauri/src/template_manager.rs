//! Template Manager: wraps `sbx template` CLI commands for template management.
//!
//! Provides a thin abstraction over SbxCli template methods, handling input
//! validation and delegating all operations to the sbx CLI.

use std::path::Path;
use std::sync::Arc;

use crate::error::OrchestratorError;
use crate::sbx::{SbxCli, TemplateInfo};

/// Manages sandbox templates via the `sbx template` CLI.
///
/// This struct delegates to `SbxCli` template methods and adds input validation.
pub struct TemplateManager {
    sbx: Arc<SbxCli>,
}

impl TemplateManager {
    /// Create a new TemplateManager wrapping the given SbxCli instance.
    pub fn new(sbx: Arc<SbxCli>) -> Self {
        Self { sbx }
    }

    /// List all saved templates.
    ///
    /// Invokes `sbx template ls` and returns parsed `Vec<TemplateInfo>`.
    pub async fn list(&self) -> Result<Vec<TemplateInfo>, OrchestratorError> {
        self.sbx.template_ls().await
    }

    /// Save a sandbox as a template with an optional tar export.
    ///
    /// Invokes `sbx template save <sandbox_id> <tag> [--output <file.tar>]`.
    ///
    /// # Arguments
    /// * `sandbox_id` - The sandbox to save as a template
    /// * `tag` - The tag name for the template
    /// * `output_tar` - Optional path to export the template as a tar file
    pub async fn save(
        &self,
        sandbox_id: &str,
        tag: &str,
        output_tar: Option<&Path>,
    ) -> Result<(), OrchestratorError> {
        if sandbox_id.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Sandbox ID cannot be empty".to_string(),
            ));
        }
        if tag.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Template tag cannot be empty".to_string(),
            ));
        }

        self.sbx.template_save(sandbox_id, tag, output_tar).await
    }

    /// Load a template from a tar file.
    ///
    /// Invokes `sbx template load <file.tar>`.
    ///
    /// # Arguments
    /// * `tar_path` - Path to the tar file to load
    pub async fn load(&self, tar_path: &Path) -> Result<(), OrchestratorError> {
        if !tar_path.exists() {
            return Err(OrchestratorError::Validation(format!(
                "Tar file does not exist: {}",
                tar_path.display()
            )));
        }

        self.sbx.template_load(tar_path).await
    }

    /// Remove a template by tag.
    ///
    /// Invokes `sbx template rm <tag>`.
    ///
    /// # Arguments
    /// * `tag` - The tag of the template to remove
    pub async fn remove(&self, tag: &str) -> Result<(), OrchestratorError> {
        if tag.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Template tag cannot be empty".to_string(),
            ));
        }

        self.sbx.template_rm(tag).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a TemplateManager with a mock sbx binary (a shell script).
    fn create_test_manager(script_content: &str) -> (TemplateManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("sbx");

        #[cfg(unix)]
        {
            use std::fs;
            use std::io::Write;
            use std::os::unix::fs::PermissionsExt;
            let mut file = fs::File::create(&script_path).unwrap();
            file.write_all(script_content.as_bytes()).unwrap();
            file.sync_all().unwrap();
            drop(file);
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
            // Brief yield to ensure kernel releases write lock on the file
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        #[cfg(windows)]
        {
            let script_path = dir.path().join("sbx.bat");
            std::fs::write(&script_path, script_content).unwrap();
        }

        let sbx = Arc::new(SbxCli::with_path(script_path));
        let manager = TemplateManager::new(sbx);
        (manager, dir)
    }

    #[tokio::test]
    async fn test_list_parses_json_output() {
        let script = r#"#!/bin/sh
if [ "$1" = "template" ] && [ "$2" = "ls" ]; then
    echo '[{"tag":"my-template","size":"1.2GB","created":"2024-01-15"}]'
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let templates = mgr.list().await.unwrap();

        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].tag, "my-template");
        assert_eq!(templates[0].size, Some("1.2GB".to_string()));
        assert_eq!(templates[0].created, Some("2024-01-15".to_string()));
    }

    #[tokio::test]
    async fn test_list_empty() {
        let script = r#"#!/bin/sh
if [ "$1" = "template" ] && [ "$2" = "ls" ]; then
    echo '[]'
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let templates = mgr.list().await.unwrap();
        assert!(templates.is_empty());
    }

    #[tokio::test]
    async fn test_list_cli_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "template" ] && [ "$2" = "ls" ]; then
    echo "error: not logged in" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.list().await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_save_success() {
        let script = r#"#!/bin/sh
if [ "$1" = "template" ] && [ "$2" = "save" ] && [ "$3" = "my-sandbox" ] && [ "$4" = "my-tag" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.save("my-sandbox", "my-tag", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_save_with_output_tar() {
        let script = r#"#!/bin/sh
if [ "$1" = "template" ] && [ "$2" = "save" ] && [ "$3" = "my-sandbox" ] && [ "$4" = "my-tag" ] && [ "$5" = "--output" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, dir) = create_test_manager(script);
        let tar_path = dir.path().join("output.tar");
        let result = mgr.save("my-sandbox", "my-tag", Some(&tar_path)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_save_empty_sandbox_id_fails() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.save("", "my-tag", None).await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_save_whitespace_sandbox_id_fails() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.save("   ", "my-tag", None).await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_save_empty_tag_fails() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.save("my-sandbox", "", None).await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_save_whitespace_tag_fails() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.save("my-sandbox", "   ", None).await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_save_cli_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "template" ] && [ "$2" = "save" ]; then
    echo "error: sandbox not found" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.save("nonexistent", "my-tag", None).await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_load_success() {
        let script = r#"#!/bin/sh
if [ "$1" = "template" ] && [ "$2" = "load" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, dir) = create_test_manager(script);
        // Create a fake tar file so the path exists
        let tar_path = dir.path().join("template.tar");
        std::fs::write(&tar_path, b"fake tar content").unwrap();

        let result = mgr.load(&tar_path).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_load_nonexistent_path_fails() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.load(Path::new("/nonexistent/path/template.tar")).await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_load_cli_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "template" ] && [ "$2" = "load" ]; then
    echo "error: invalid tar file" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, dir) = create_test_manager(script);
        let tar_path = dir.path().join("bad.tar");
        std::fs::write(&tar_path, b"bad content").unwrap();

        let result = mgr.load(&tar_path).await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_remove_success() {
        let script = r#"#!/bin/sh
if [ "$1" = "template" ] && [ "$2" = "rm" ] && [ "$3" = "my-template" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.remove("my-template").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_remove_empty_tag_fails() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.remove("").await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_remove_whitespace_tag_fails() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.remove("   ").await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_remove_cli_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "template" ] && [ "$2" = "rm" ]; then
    echo "error: template not found" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.remove("nonexistent").await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }
}
