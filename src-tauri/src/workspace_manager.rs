use std::fs;
use std::path::{Path, PathBuf};

use crate::error::OrchestratorError;

/// Workspace Manager handles workspace path validation and file upload operations.
///
/// SECURITY:
/// - Validates uploaded filenames against path traversal attacks (`../`, absolute paths, null bytes).
/// - Canonicalizes output paths and verifies they remain under the uploads directory.
/// - Uses platform-independent path handling for cross-platform support.
pub struct WorkspaceManager;

impl WorkspaceManager {
    /// Validate that a workspace path exists and is absolute.
    ///
    /// Requirements: 7.1, 7.3, 9.2
    pub fn validate_path(path: &Path) -> Result<(), OrchestratorError> {
        if !path.is_absolute() {
            return Err(OrchestratorError::WorkspaceNotFound(format!(
                "Workspace path must be absolute: {}",
                path.display()
            )));
        }

        if !path.exists() {
            return Err(OrchestratorError::WorkspaceNotFound(format!(
                "Workspace path does not exist: {}",
                path.display()
            )));
        }

        Ok(())
    }

    /// Upload a file to the workspace uploads directory.
    ///
    /// Copies content to `<workspace>/.beachead/uploads/<filename>` and returns
    /// the absolute path to the uploaded file.
    ///
    /// SECURITY:
    /// - Rejects filenames containing path traversal sequences (`../`, `..\\`)
    /// - Rejects filenames that are absolute paths
    /// - Rejects filenames containing null bytes
    /// - Canonicalizes the output path and verifies it remains under the uploads dir
    ///
    /// Requirements: 4.9, 7.2
    pub fn upload_to_workspace(
        workspace: &Path,
        filename: &str,
        content: &[u8],
    ) -> Result<PathBuf, OrchestratorError> {
        // SECURITY: Validate filename does not contain path traversal sequences
        Self::validate_filename(filename)?;

        // Ensure the uploads directory exists
        let uploads_dir = workspace.join(".beachead").join("uploads");
        fs::create_dir_all(&uploads_dir).map_err(|e| {
            OrchestratorError::Internal(format!(
                "Failed to create uploads directory {}: {}",
                uploads_dir.display(),
                e
            ))
        })?;

        // Write the file
        let target_path = uploads_dir.join(filename);
        fs::write(&target_path, content).map_err(|e| {
            OrchestratorError::Internal(format!(
                "Failed to write uploaded file {}: {}",
                target_path.display(),
                e
            ))
        })?;

        // SECURITY: Canonicalize the output path and verify it's still under uploads_dir
        let canonical_path = fs::canonicalize(&target_path).map_err(|e| {
            OrchestratorError::Internal(format!(
                "Failed to canonicalize uploaded file path: {}",
                e
            ))
        })?;

        let canonical_uploads = fs::canonicalize(&uploads_dir).map_err(|e| {
            OrchestratorError::Internal(format!(
                "Failed to canonicalize uploads directory: {}",
                e
            ))
        })?;

        if !canonical_path.starts_with(&canonical_uploads) {
            // Remove the file if it escaped the uploads directory
            let _ = fs::remove_file(&target_path);
            return Err(OrchestratorError::Validation(
                "Uploaded file path escapes the uploads directory".to_string(),
            ));
        }

        Ok(canonical_path)
    }

    /// Check if a file path is inside the given workspace directory.
    ///
    /// Used to determine whether to use workspace upload or `sbx cp` for file transfer.
    ///
    /// Requirements: 4.9, 4.11
    pub fn is_path_inside_workspace(file_path: &Path, workspace: &Path) -> bool {
        // Attempt to canonicalize both paths for accurate comparison.
        // If canonicalization fails (e.g., path doesn't exist yet), fall back to
        // starts_with on the raw paths.
        let canonical_file = fs::canonicalize(file_path).unwrap_or_else(|_| file_path.to_path_buf());
        let canonical_workspace =
            fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());

        canonical_file.starts_with(&canonical_workspace)
    }

    /// Validate that a filename is safe for use in file uploads.
    ///
    /// SECURITY: Rejects filenames that:
    /// - Contain path traversal sequences (`../` or `..\\`)
    /// - Are absolute paths (start with `/` or a Windows drive letter)
    /// - Contain null bytes
    /// - Are empty
    /// - Consist only of `.` or `..`
    fn validate_filename(filename: &str) -> Result<(), OrchestratorError> {
        if filename.is_empty() {
            return Err(OrchestratorError::Validation(
                "Filename cannot be empty".to_string(),
            ));
        }

        // Reject null bytes
        if filename.contains('\0') {
            return Err(OrchestratorError::Validation(
                "Filename contains null bytes".to_string(),
            ));
        }

        // Reject path traversal sequences
        if filename.contains("../") || filename.contains("..\\") {
            return Err(OrchestratorError::Validation(
                "Filename contains path traversal sequence".to_string(),
            ));
        }

        // Reject bare `..` (the filename itself is a traversal)
        if filename == ".." || filename == "." {
            return Err(OrchestratorError::Validation(
                "Filename cannot be '.' or '..'".to_string(),
            ));
        }

        // Reject absolute paths (Unix)
        if filename.starts_with('/') {
            return Err(OrchestratorError::Validation(
                "Filename cannot be an absolute path".to_string(),
            ));
        }

        // Reject absolute paths (Windows drive letters like C:\)
        if filename.len() >= 2 {
            let bytes = filename.as_bytes();
            if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
                return Err(OrchestratorError::Validation(
                    "Filename cannot be an absolute path".to_string(),
                ));
            }
        }

        // Reject backslash-based traversal without forward slash
        // e.g., "..\\secret" where the `../` check above wouldn't catch it
        if filename.starts_with("..\\") {
            return Err(OrchestratorError::Validation(
                "Filename contains path traversal sequence".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_validate_path_absolute_exists() {
        let tmp = TempDir::new().unwrap();
        let result = WorkspaceManager::validate_path(tmp.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_path_relative_rejected() {
        let result = WorkspaceManager::validate_path(Path::new("relative/path"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, OrchestratorError::WorkspaceNotFound(_)));
    }

    #[test]
    fn test_validate_path_nonexistent_rejected() {
        let result = WorkspaceManager::validate_path(Path::new("/nonexistent/path/xyz123"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, OrchestratorError::WorkspaceNotFound(_)));
    }

    #[test]
    fn test_validate_filename_valid() {
        assert!(WorkspaceManager::validate_filename("hello.txt").is_ok());
        assert!(WorkspaceManager::validate_filename("my-file_v2.tar.gz").is_ok());
        assert!(WorkspaceManager::validate_filename("document").is_ok());
    }

    #[test]
    fn test_validate_filename_empty() {
        let result = WorkspaceManager::validate_filename("");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_filename_null_bytes() {
        let result = WorkspaceManager::validate_filename("file\0name.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_filename_traversal_unix() {
        assert!(WorkspaceManager::validate_filename("../etc/passwd").is_err());
        assert!(WorkspaceManager::validate_filename("foo/../bar").is_err());
    }

    #[test]
    fn test_validate_filename_traversal_windows() {
        assert!(WorkspaceManager::validate_filename("..\\windows\\system32").is_err());
        assert!(WorkspaceManager::validate_filename("foo\\..\\bar").is_err());
    }

    #[test]
    fn test_validate_filename_absolute_unix() {
        assert!(WorkspaceManager::validate_filename("/etc/passwd").is_err());
    }

    #[test]
    fn test_validate_filename_absolute_windows() {
        assert!(WorkspaceManager::validate_filename("C:\\Windows\\System32").is_err());
        assert!(WorkspaceManager::validate_filename("D:\\file.txt").is_err());
    }

    #[test]
    fn test_validate_filename_dot_dotdot() {
        assert!(WorkspaceManager::validate_filename(".").is_err());
        assert!(WorkspaceManager::validate_filename("..").is_err());
    }

    #[test]
    fn test_upload_to_workspace_success() {
        let tmp = TempDir::new().unwrap();
        let content = b"hello world";

        let result =
            WorkspaceManager::upload_to_workspace(tmp.path(), "test.txt", content);
        assert!(result.is_ok());

        let path = result.unwrap();
        assert!(path.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello world");

        // Verify it's under .beachead/uploads/
        let uploads_dir = tmp.path().join(".beachead").join("uploads");
        assert!(path.starts_with(fs::canonicalize(&uploads_dir).unwrap()));
    }

    #[test]
    fn test_upload_to_workspace_creates_directory() {
        let tmp = TempDir::new().unwrap();
        let uploads_dir = tmp.path().join(".beachead").join("uploads");
        assert!(!uploads_dir.exists());

        let result =
            WorkspaceManager::upload_to_workspace(tmp.path(), "file.bin", b"data");
        assert!(result.is_ok());
        assert!(uploads_dir.exists());
    }

    #[test]
    fn test_upload_to_workspace_rejects_traversal() {
        let tmp = TempDir::new().unwrap();

        let result =
            WorkspaceManager::upload_to_workspace(tmp.path(), "../escape.txt", b"bad");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, OrchestratorError::Validation(_)));
    }

    #[test]
    fn test_upload_to_workspace_rejects_absolute_path() {
        let tmp = TempDir::new().unwrap();

        let result =
            WorkspaceManager::upload_to_workspace(tmp.path(), "/etc/passwd", b"bad");
        assert!(result.is_err());
    }

    #[test]
    fn test_upload_to_workspace_rejects_null_bytes() {
        let tmp = TempDir::new().unwrap();

        let result =
            WorkspaceManager::upload_to_workspace(tmp.path(), "file\0.txt", b"bad");
        assert!(result.is_err());
    }

    #[test]
    fn test_is_path_inside_workspace_true() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("subdir").join("file.txt");

        // Create the file so canonicalize works
        fs::create_dir_all(tmp.path().join("subdir")).unwrap();
        fs::write(&file_path, "test").unwrap();

        assert!(WorkspaceManager::is_path_inside_workspace(
            &file_path,
            tmp.path()
        ));
    }

    #[test]
    fn test_is_path_inside_workspace_false() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();

        let file_path = tmp1.path().join("file.txt");
        fs::write(&file_path, "test").unwrap();

        assert!(!WorkspaceManager::is_path_inside_workspace(
            &file_path,
            tmp2.path()
        ));
    }

    #[test]
    fn test_is_path_inside_workspace_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        // Non-existent file that would be inside workspace if it existed
        let file_path = tmp.path().join("nonexistent.txt");

        // Falls back to starts_with on raw paths
        assert!(WorkspaceManager::is_path_inside_workspace(
            &file_path,
            tmp.path()
        ));
    }
}
