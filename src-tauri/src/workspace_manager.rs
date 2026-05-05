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
    use proptest::prelude::*;
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

    // =========================================================================
    // Property-Based Tests — Property 10: File upload routing
    // Validates: Requirements 4.9, 4.11
    // =========================================================================

    /// Strategy for generating relative path segments for files inside a workspace.
    fn inside_file_path_strategy() -> impl Strategy<Value = Vec<String>> {
        proptest::collection::vec("[a-z][a-z0-9_]{0,8}".prop_map(|s| s), 1..=4)
    }

    proptest! {
        /// **Validates: Requirements 4.9, 4.11**
        ///
        /// Property: files inside the workspace directory are correctly identified
        /// as inside, meaning they would be routed to `<workspace>/.beachead/uploads/`.
        #[test]
        fn prop_inside_workspace_files_routed_to_uploads(
            segments in inside_file_path_strategy()
        ) {
            let workspace = TempDir::new().unwrap();
            // Build a file path inside the workspace
            let mut file_path = workspace.path().to_path_buf();
            for seg in &segments {
                file_path.push(seg);
            }
            // Create the parent directories and file so canonicalize works
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&file_path, "test content").unwrap();

            let is_inside = WorkspaceManager::is_path_inside_workspace(
                &file_path,
                workspace.path(),
            );
            prop_assert!(
                is_inside,
                "File {:?} should be inside workspace {:?}",
                file_path,
                workspace.path()
            );
        }

        /// **Validates: Requirements 4.9, 4.11**
        ///
        /// Property: files outside the workspace directory are correctly identified
        /// as outside, meaning they would be routed to `sbx cp`.
        #[test]
        fn prop_outside_workspace_files_routed_to_sbx_cp(
            segments in inside_file_path_strategy()
        ) {
            let workspace = TempDir::new().unwrap();
            let other_dir = TempDir::new().unwrap();
            // Build a file path inside other_dir (outside workspace)
            let mut file_path = other_dir.path().to_path_buf();
            for seg in &segments {
                file_path.push(seg);
            }
            // Create the parent directories and file so canonicalize works
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&file_path, "test content").unwrap();

            let is_inside = WorkspaceManager::is_path_inside_workspace(
                &file_path,
                workspace.path(),
            );
            prop_assert!(
                !is_inside,
                "File {:?} should be outside workspace {:?} (routed to sbx cp)",
                file_path,
                workspace.path()
            );
        }
    }

    // =========================================================================
    // Property-Based Tests — Property 20: Workspace path validation
    // Validates: Requirements 7.1, 7.3
    // =========================================================================

    /// Strategy for generating subdirectory names (safe filesystem characters).
    fn subdir_name_strategy() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_-]{0,10}".prop_map(|s| s)
    }

    /// Strategy for generating random absolute paths that are unlikely to exist.
    fn nonexistent_absolute_path_strategy() -> impl Strategy<Value = PathBuf> {
        "[a-z]{4,8}".prop_flat_map(|seg1| {
            "[a-z]{4,8}".prop_map(move |seg2| {
                PathBuf::from(format!(
                    "/nonexistent_beachead_test_{}/{}",
                    seg1, seg2
                ))
            })
        })
    }

    /// Strategy for generating relative paths (no leading slash).
    fn relative_path_strategy() -> impl Strategy<Value = PathBuf> {
        "[a-z]{1,6}(/[a-z]{1,6}){0,3}".prop_map(|s| PathBuf::from(s))
    }

    proptest! {
        /// **Validates: Requirements 7.1, 7.3**
        ///
        /// Property: existing absolute paths are accepted by validate_path.
        /// Creates a tempdir and generates subdirectories within it, then
        /// asserts validate_path returns Ok for each.
        #[test]
        fn prop_existing_paths_accepted(
            subdir in subdir_name_strategy()
        ) {
            let tmp = TempDir::new().unwrap();
            let dir_path = tmp.path().join(&subdir);
            fs::create_dir_all(&dir_path).unwrap();

            let result = WorkspaceManager::validate_path(&dir_path);
            prop_assert!(
                result.is_ok(),
                "Expected existing path {:?} to be accepted, got: {:?}",
                dir_path,
                result
            );
        }

        /// **Validates: Requirements 7.1, 7.3**
        ///
        /// Property: non-existing absolute paths are rejected by validate_path.
        /// Generates random absolute paths that don't exist on the filesystem
        /// and asserts validate_path returns an error.
        #[test]
        fn prop_nonexistent_paths_rejected(
            path in nonexistent_absolute_path_strategy()
        ) {
            // Ensure the path truly doesn't exist
            prop_assume!(!path.exists());

            let result = WorkspaceManager::validate_path(&path);
            prop_assert!(
                result.is_err(),
                "Expected non-existing path {:?} to be rejected",
                path
            );
            match result.unwrap_err() {
                OrchestratorError::WorkspaceNotFound(_) => {}
                other => prop_assert!(
                    false,
                    "Expected WorkspaceNotFound error, got: {:?}",
                    other
                ),
            }
        }

        /// **Validates: Requirements 7.1, 7.3**
        ///
        /// Property: relative paths are always rejected by validate_path,
        /// regardless of whether they happen to resolve to existing directories.
        #[test]
        fn prop_relative_paths_rejected(
            path in relative_path_strategy()
        ) {
            let result = WorkspaceManager::validate_path(&path);
            prop_assert!(
                result.is_err(),
                "Expected relative path {:?} to be rejected",
                path
            );
            match result.unwrap_err() {
                OrchestratorError::WorkspaceNotFound(_) => {}
                other => prop_assert!(
                    false,
                    "Expected WorkspaceNotFound error, got: {:?}",
                    other
                ),
            }
        }
    }
}
