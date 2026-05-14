//! Property-based tests for secret scanner pattern matching.
//!
//! **Validates: Requirements 15.2, 15.5**
//!
//! Properties tested:
//! - Strings containing known secret patterns (AWS keys, GitHub tokens, GitLab tokens,
//!   private key content) are always detected by content patterns
//! - Random alphanumeric strings don't trigger false positives on content patterns
//! - File-only patterns correctly match `.env*`, `.pem`, `.key`, `.p12`, `.pfx` files

use proptest::prelude::*;

use crate::secret_scanner::SecretScanner;

// ─── Generators ────────────────────────────────────────────────────────────────

/// Generate a valid AWS access key (AKIA followed by exactly 16 uppercase alphanumeric chars).
fn aws_key_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(prop_oneof![b'0'..=b'9', b'A'..=b'Z'], 16).prop_map(|chars| {
        let suffix: String = chars.iter().map(|&c| c as char).collect();
        format!("AKIA{}", suffix)
    })
}

/// Generate a valid GitHub token (ghp_, ghs_, or gho_ followed by 36+ alphanumeric/underscore chars).
fn github_token_strategy() -> impl Strategy<Value = String> {
    (
        prop_oneof![Just("ghp_"), Just("ghs_"), Just("gho_")],
        prop::collection::vec(
            prop_oneof![b'a'..=b'z', b'A'..=b'Z', b'0'..=b'9', Just(b'_')],
            36..=50,
        ),
    )
        .prop_map(|(prefix, chars)| {
            let suffix: String = chars.iter().map(|&c| c as char).collect();
            format!("{}{}", prefix, suffix)
        })
}

/// Generate a valid GitLab token (glpat- followed by 20+ alphanumeric/dash/underscore chars).
fn gitlab_token_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![b'a'..=b'z', b'A'..=b'Z', b'0'..=b'9', Just(b'-'), Just(b'_')],
        20..=40,
    )
    .prop_map(|chars| {
        let suffix: String = chars.iter().map(|&c| c as char).collect();
        format!("glpat-{}", suffix)
    })
}

/// Generate private key content with a valid BEGIN header.
fn private_key_content_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("-----BEGIN RSA PRIVATE KEY-----"),
        Just("-----BEGIN EC PRIVATE KEY-----"),
        Just("-----BEGIN DSA PRIVATE KEY-----"),
        Just("-----BEGIN OPENSSH PRIVATE KEY-----"),
        Just("-----BEGIN ENCRYPTED PRIVATE KEY-----"),
    ]
    .prop_map(|s| s.to_string())
}

/// Generate random alphanumeric strings that should NOT match any content pattern.
/// Avoids: AKIA prefix, ghp_/ghs_/gho_ prefix, glpat- prefix, and "-----BEGIN" sequences.
fn safe_alphanumeric_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9]{1,200}".prop_filter("must not accidentally match secret patterns", |s| {
        !s.contains("AKIA")
            && !s.contains("ghp_")
            && !s.contains("ghs_")
            && !s.contains("gho_")
            && !s.contains("glpat-")
            && !s.contains("-----BEGIN")
    })
}

/// Generate filenames that should match file-only patterns (.env*, .pem, .key, .p12, .pfx).
fn secret_filename_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        // .env files
        Just(".env".to_string()),
        "[a-z]{1,10}".prop_map(|s| format!(".env.{}", s)),
        // Private key file extensions
        "[a-z]{1,10}".prop_map(|s| format!("{}.pem", s)),
        "[a-z]{1,10}".prop_map(|s| format!("{}.key", s)),
        "[a-z]{1,10}".prop_map(|s| format!("{}.p12", s)),
        "[a-z]{1,10}".prop_map(|s| format!("{}.pfx", s)),
    ]
}

/// Generate filenames that should NOT match any file-only pattern.
fn safe_filename_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z]{1,15}\\.(rs|ts|js|py|md|txt|json|yaml|toml|html|css)".prop_map(|s| s),
        Just("README.md".to_string()),
        Just("Cargo.toml".to_string()),
        Just("package.json".to_string()),
        Just("main.rs".to_string()),
        Just("index.ts".to_string()),
    ]
    .prop_filter("must not match secret file patterns", |s| {
        !s.ends_with(".pem")
            && !s.ends_with(".key")
            && !s.ends_with(".p12")
            && !s.ends_with(".pfx")
            && !s.contains(".env")
    })
}

// ─── Property 1: Known content patterns are always detected ────────────────────

proptest! {
    /// Strings containing a valid AWS access key pattern are always detected.
    /// **Validates: Requirements 15.2**
    #[test]
    fn aws_keys_always_detected(
        prefix in "[a-zA-Z0-9 =:\"']{0,50}",
        aws_key in aws_key_strategy(),
        suffix in "[a-zA-Z0-9 ]{0,50}",
    ) {
        let scanner = SecretScanner::new();
        let content = format!("{}{}{}", prefix, aws_key, suffix);

        let content_patterns: Vec<_> = scanner.patterns_ref()
            .iter()
            .filter(|p| !p.file_only() && p.name() == "AWS access key")
            .collect();

        prop_assert!(!content_patterns.is_empty(), "AWS access key pattern must exist");

        let matched = content_patterns.iter().any(|p| p.regex().is_match(&content));
        prop_assert!(
            matched,
            "AWS key '{}' in content '{}' should be detected",
            aws_key, content
        );
    }

    /// Strings containing a valid GitHub token pattern are always detected.
    /// **Validates: Requirements 15.2**
    #[test]
    fn github_tokens_always_detected(
        prefix in "[a-zA-Z0-9 =:\"']{0,50}",
        token in github_token_strategy(),
        suffix in "[a-zA-Z0-9 ]{0,50}",
    ) {
        let scanner = SecretScanner::new();
        let content = format!("{}{}{}", prefix, token, suffix);

        let content_patterns: Vec<_> = scanner.patterns_ref()
            .iter()
            .filter(|p| !p.file_only() && p.name() == "GitHub token")
            .collect();

        prop_assert!(!content_patterns.is_empty(), "GitHub token pattern must exist");

        let matched = content_patterns.iter().any(|p| p.regex().is_match(&content));
        prop_assert!(
            matched,
            "GitHub token '{}' in content '{}' should be detected",
            token, content
        );
    }

    /// Strings containing a valid GitLab token pattern are always detected.
    /// **Validates: Requirements 15.2**
    #[test]
    fn gitlab_tokens_always_detected(
        prefix in "[a-zA-Z0-9 =:\"']{0,50}",
        token in gitlab_token_strategy(),
        suffix in "[a-zA-Z0-9 ]{0,50}",
    ) {
        let scanner = SecretScanner::new();
        let content = format!("{}{}{}", prefix, token, suffix);

        let content_patterns: Vec<_> = scanner.patterns_ref()
            .iter()
            .filter(|p| !p.file_only() && p.name() == "GitLab token")
            .collect();

        prop_assert!(!content_patterns.is_empty(), "GitLab token pattern must exist");

        let matched = content_patterns.iter().any(|p| p.regex().is_match(&content));
        prop_assert!(
            matched,
            "GitLab token '{}' in content '{}' should be detected",
            token, content
        );
    }

    /// Strings containing private key content are always detected.
    /// **Validates: Requirements 15.2**
    #[test]
    fn private_key_content_always_detected(
        prefix in "[a-zA-Z0-9 \\n]{0,50}",
        key_header in private_key_content_strategy(),
        suffix in "[a-zA-Z0-9 \\n]{0,50}",
    ) {
        let scanner = SecretScanner::new();
        let content = format!("{}{}{}", prefix, key_header, suffix);

        let content_patterns: Vec<_> = scanner.patterns_ref()
            .iter()
            .filter(|p| !p.file_only() && p.name() == "private key content")
            .collect();

        prop_assert!(!content_patterns.is_empty(), "Private key content pattern must exist");

        let matched = content_patterns.iter().any(|p| p.regex().is_match(&content));
        prop_assert!(
            matched,
            "Private key header '{}' in content '{}' should be detected",
            key_header, content
        );
    }
}

// ─── Property 2: Random alphanumeric strings don't false-positive ──────────────

proptest! {
    /// Random alphanumeric strings (without known secret prefixes) never trigger
    /// content pattern matches.
    /// **Validates: Requirements 15.5**
    #[test]
    fn random_alphanumeric_no_false_positives(
        content in safe_alphanumeric_strategy(),
    ) {
        let scanner = SecretScanner::new();

        let content_patterns: Vec<_> = scanner.patterns_ref()
            .iter()
            .filter(|p| !p.file_only())
            .collect();

        for pattern in &content_patterns {
            prop_assert!(
                !pattern.regex().is_match(&content),
                "Safe alphanumeric string '{}' should not match content pattern '{}'",
                content, pattern.name()
            );
        }
    }
}

// ─── Property 3: File-only patterns correctly match secret filenames ───────────

proptest! {
    /// Filenames matching known secret file patterns (.env*, .pem, .key, .p12, .pfx)
    /// are always detected by file-only patterns.
    /// **Validates: Requirements 15.2**
    #[test]
    fn secret_filenames_always_detected(
        filename in secret_filename_strategy(),
    ) {
        let scanner = SecretScanner::new();

        let file_patterns: Vec<_> = scanner.patterns_ref()
            .iter()
            .filter(|p| p.file_only())
            .collect();

        let matched = file_patterns.iter().any(|p| p.regex().is_match(&filename));
        prop_assert!(
            matched,
            "Secret filename '{}' should be detected by at least one file-only pattern",
            filename
        );
    }

    /// Filenames that don't match secret patterns are not flagged by file-only patterns.
    /// **Validates: Requirements 15.5**
    #[test]
    fn safe_filenames_no_false_positives(
        filename in safe_filename_strategy(),
    ) {
        let scanner = SecretScanner::new();

        let file_patterns: Vec<_> = scanner.patterns_ref()
            .iter()
            .filter(|p| p.file_only())
            .collect();

        for pattern in &file_patterns {
            prop_assert!(
                !pattern.regex().is_match(&filename),
                "Safe filename '{}' should not match file-only pattern '{}'",
                filename, pattern.name()
            );
        }
    }
}
