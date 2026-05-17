use regex::Regex;
use std::path::Path;
use tokio::time::{timeout, Duration};

use crate::git_cli::GitCli;

/// Timeout for the entire secret scan operation.
const SCAN_TIMEOUT_SECS: u64 = 30;

/// A pattern used to detect potential secrets in file names or content.
pub struct SecretPattern {
    name: &'static str,
    regex: Regex,
    /// true = match against filename only, false = match against file content.
    file_only: bool,
}

impl SecretPattern {
    /// Returns the pattern name.
    pub fn name(&self) -> &str {
        self.name
    }

    /// Returns a reference to the compiled regex.
    pub fn regex(&self) -> &Regex {
        &self.regex
    }

    /// Returns whether this pattern matches filenames only (vs content).
    pub fn file_only(&self) -> bool {
        self.file_only
    }
}

/// A finding from the secret scanner indicating a potential secret was detected.
#[derive(Debug, Clone, PartialEq)]
pub struct SecretFinding {
    pub file_path: String,
    pub pattern_name: String,
}

/// Scans commits for potential secrets before pushing to remote.
pub struct SecretScanner {
    patterns: Vec<SecretPattern>,
}

impl Default for SecretScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretScanner {
    /// Create a new SecretScanner with default detection patterns.
    pub fn new() -> Self {
        Self {
            patterns: vec![
                SecretPattern {
                    name: "env file",
                    regex: Regex::new(r"(?:^|/)\.env(\..+)?$").unwrap(),
                    file_only: true,
                },
                SecretPattern {
                    name: "private key file",
                    regex: Regex::new(r"\.(pem|key|p12|pfx)$").unwrap(),
                    file_only: true,
                },
                SecretPattern {
                    name: "private key content",
                    regex: Regex::new(r"-----BEGIN .* PRIVATE KEY-----").unwrap(),
                    file_only: false,
                },
                SecretPattern {
                    name: "AWS access key",
                    regex: Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
                    file_only: false,
                },
                SecretPattern {
                    name: "GitHub token",
                    regex: Regex::new(r"gh[pso]_[A-Za-z0-9_]{36,}").unwrap(),
                    file_only: false,
                },
                SecretPattern {
                    name: "GitLab token",
                    regex: Regex::new(r"glpat-[A-Za-z0-9\-_]{20,}").unwrap(),
                    file_only: false,
                },
            ],
        }
    }

    /// Returns a reference to the internal patterns list (for testing).
    pub fn patterns_ref(&self) -> &[SecretPattern] {
        &self.patterns
    }

    /// Scan commits for secrets. Returns Ok(vec![]) if clean, Err(findings) if secrets detected.
    ///
    /// The scan:
    /// 1. Gets the list of changed files in the given commits
    /// 2. Identifies binary files (skipped)
    /// 3. Checks filenames against file-only patterns
    /// 4. Checks file content against content patterns
    /// 5. Applies a 30-second timeout on the entire operation
    pub async fn scan_commits(
        &self,
        mirror: &Path,
        commit_shas: &[String],
        git: &GitCli,
    ) -> Result<Vec<SecretFinding>, Vec<SecretFinding>> {
        if commit_shas.is_empty() {
            return Ok(vec![]);
        }

        let scan_future = self.scan_commits_inner(mirror, commit_shas, git);

        match timeout(Duration::from_secs(SCAN_TIMEOUT_SECS), scan_future).await {
            Ok(result) => result,
            Err(_elapsed) => {
                // Timeout — treat as blocking (return error with a timeout finding)
                Err(vec![SecretFinding {
                    file_path: String::new(),
                    pattern_name: "scan timeout exceeded (30s)".to_string(),
                }])
            }
        }
    }

    async fn scan_commits_inner(
        &self,
        mirror: &Path,
        commit_shas: &[String],
        git: &GitCli,
    ) -> Result<Vec<SecretFinding>, Vec<SecretFinding>> {
        // Build a commit range for the diff. We use the combined diff of all selected commits.
        // Get the list of changed files with numstat to identify binary files.

        // Get changed files with numstat (binary files show "-\t-\t" prefix)
        let mut numstat_args: Vec<&str> = vec!["diff", "--numstat"];
        // Use the first commit's parent as the base, diff to the last commit
        let first_sha = &commit_shas[0];
        let last_sha = &commit_shas[commit_shas.len() - 1];
        let parent_ref = format!("{}^", first_sha);

        // Try using parent of first commit as base
        numstat_args.push(&parent_ref);
        numstat_args.push(last_sha.as_str());

        let numstat_output = match git.exec(mirror, &numstat_args, None, false).await {
            Ok(output) => output,
            Err(_) => {
                // If parent doesn't exist (initial commit), diff against empty tree
                let empty_tree = "4b825dc642cb6eb9a060e54bf899d15f3c338fb9";
                let fallback_args = vec!["diff", "--numstat", empty_tree, last_sha.as_str()];
                match git.exec(mirror, &fallback_args, None, false).await {
                    Ok(output) => output,
                    Err(_) => return Ok(vec![]), // Can't get diff, skip scan
                }
            }
        };

        // Parse numstat output to identify binary files and changed file paths
        let mut binary_files: Vec<String> = Vec::new();
        let mut changed_files: Vec<String> = Vec::new();

        for line in numstat_output.stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // Binary files show as: -\t-\tfilename
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() == 3 {
                let file_path = parts[2].to_string();
                if parts[0] == "-" && parts[1] == "-" {
                    binary_files.push(file_path);
                } else {
                    changed_files.push(file_path);
                }
            }
        }

        let mut findings: Vec<SecretFinding> = Vec::new();

        // Check filenames (including binary files) against file-only patterns
        let all_files: Vec<&String> = changed_files.iter().chain(binary_files.iter()).collect();
        for file_path in &all_files {
            for pattern in &self.patterns {
                if pattern.file_only && pattern.regex.is_match(file_path) {
                    findings.push(SecretFinding {
                        file_path: file_path.to_string(),
                        pattern_name: pattern.name.to_string(),
                    });
                }
            }
        }

        // Check content of non-binary changed files against content patterns
        let content_patterns: Vec<&SecretPattern> =
            self.patterns.iter().filter(|p| !p.file_only).collect();

        if !content_patterns.is_empty() {
            for file_path in &changed_files {
                // Get file content using git show
                let blob_ref = format!("{}:{}", last_sha, file_path);
                let show_args = vec!["show", &blob_ref];

                let content = match git.exec(mirror, &show_args, None, false).await {
                    Ok(output) => output.stdout,
                    Err(_) => continue, // Skip files we can't read
                };

                for pattern in &content_patterns {
                    if pattern.regex.is_match(&content) {
                        findings.push(SecretFinding {
                            file_path: file_path.to_string(),
                            pattern_name: pattern.name.to_string(),
                        });
                    }
                }
            }
        }

        if findings.is_empty() {
            Ok(vec![])
        } else {
            Err(findings)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_file_pattern() {
        let scanner = SecretScanner::new();
        let env_pattern = &scanner.patterns[0];
        assert!(env_pattern.regex.is_match(".env"));
        assert!(env_pattern.regex.is_match(".env.local"));
        assert!(env_pattern.regex.is_match(".env.production"));
        assert!(env_pattern.regex.is_match("config/.env"));
        assert!(env_pattern.regex.is_match("config/.env.test"));
        assert!(!env_pattern.regex.is_match("env"));
        assert!(!env_pattern.regex.is_match("myenv.txt"));
        assert!(!env_pattern.regex.is_match(".environment"));
    }

    #[test]
    fn test_private_key_file_pattern() {
        let scanner = SecretScanner::new();
        let key_pattern = &scanner.patterns[1];
        assert!(key_pattern.regex.is_match("server.pem"));
        assert!(key_pattern.regex.is_match("id_rsa.key"));
        assert!(key_pattern.regex.is_match("cert.p12"));
        assert!(key_pattern.regex.is_match("keystore.pfx"));
        assert!(key_pattern.regex.is_match("path/to/file.pem"));
        assert!(!key_pattern.regex.is_match("readme.txt"));
        assert!(!key_pattern.regex.is_match("key.txt"));
    }

    #[test]
    fn test_private_key_content_pattern() {
        let scanner = SecretScanner::new();
        let content_pattern = &scanner.patterns[2];
        assert!(content_pattern
            .regex
            .is_match("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(content_pattern
            .regex
            .is_match("-----BEGIN EC PRIVATE KEY-----"));
        assert!(content_pattern
            .regex
            .is_match("some text -----BEGIN RSA PRIVATE KEY----- more text"));
        assert!(!content_pattern.regex.is_match("-----BEGIN PUBLIC KEY-----"));
        assert!(!content_pattern
            .regex
            .is_match("-----BEGIN CERTIFICATE-----"));
    }

    #[test]
    fn test_aws_key_pattern() {
        let scanner = SecretScanner::new();
        let aws_pattern = &scanner.patterns[3];
        assert!(aws_pattern.regex.is_match("AKIAIOSFODNN7EXAMPLE"));
        assert!(aws_pattern.regex.is_match("key=AKIAIOSFODNN7EXAMPLE"));
        assert!(!aws_pattern.regex.is_match("AKIA")); // Too short
        assert!(!aws_pattern.regex.is_match("AKIAiosfodnn7example")); // Lowercase
    }

    #[test]
    fn test_github_token_pattern() {
        let scanner = SecretScanner::new();
        let gh_pattern = &scanner.patterns[4];
        // ghp_ (personal access token)
        assert!(gh_pattern
            .regex
            .is_match("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij"));
        // ghs_ (server-to-server token)
        assert!(gh_pattern
            .regex
            .is_match("ghs_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij"));
        // gho_ (OAuth token)
        assert!(gh_pattern
            .regex
            .is_match("gho_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij"));
        // Too short
        assert!(!gh_pattern.regex.is_match("ghp_short"));
    }

    #[test]
    fn test_gitlab_token_pattern() {
        let scanner = SecretScanner::new();
        let gl_pattern = &scanner.patterns[5];
        assert!(gl_pattern
            .regex
            .is_match("glpat-xxxxxxxxxxxxxxxxxxxx"));
        assert!(gl_pattern
            .regex
            .is_match("glpat-Ab3_Cd5-Ef7_Gh9-Ij1_Kl"));
        assert!(!gl_pattern.regex.is_match("glpat-short")); // Too short (< 20 chars after prefix)
    }

    #[test]
    fn test_scanner_new_has_all_patterns() {
        let scanner = SecretScanner::new();
        assert_eq!(scanner.patterns.len(), 6);
        assert_eq!(scanner.patterns[0].name, "env file");
        assert_eq!(scanner.patterns[1].name, "private key file");
        assert_eq!(scanner.patterns[2].name, "private key content");
        assert_eq!(scanner.patterns[3].name, "AWS access key");
        assert_eq!(scanner.patterns[4].name, "GitHub token");
        assert_eq!(scanner.patterns[5].name, "GitLab token");
    }

    #[test]
    fn test_file_only_flags() {
        let scanner = SecretScanner::new();
        assert!(scanner.patterns[0].file_only); // env file
        assert!(scanner.patterns[1].file_only); // private key file
        assert!(!scanner.patterns[2].file_only); // private key content
        assert!(!scanner.patterns[3].file_only); // AWS access key
        assert!(!scanner.patterns[4].file_only); // GitHub token
        assert!(!scanner.patterns[5].file_only); // GitLab token
    }

    #[tokio::test]
    async fn test_scan_commits_empty_shas() {
        let scanner = SecretScanner::new();
        let git = GitCli::new("git".to_string());
        let result = scanner
            .scan_commits(Path::new("/tmp"), &[], &git)
            .await;
        assert_eq!(result, Ok(vec![]));
    }
}
