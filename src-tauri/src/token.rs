//! Bearer token generation and validation for MCP container authentication.
//!
//! Generates cryptographically random tokens with at least 32 bytes (256 bits)
//! of entropy, encoded as URL-safe base64 (no padding) for safe use in HTTP
//! Authorization headers.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use rusqlite::params;

use crate::db::Database;
use crate::error::OrchestratorError;

/// Number of random bytes used for token generation (256 bits of entropy).
const TOKEN_BYTE_LENGTH: usize = 32;

/// Generate a cryptographically random bearer token.
///
/// Produces 32 bytes of random data from the OS CSPRNG, encoded as URL-safe
/// base64 without padding. The resulting string is safe for use in HTTP
/// Authorization headers.
pub fn generate_bearer_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTE_LENGTH];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Check that a token does not already exist in the `mcp_containers` table.
///
/// Returns `Ok(true)` if the token is unique (no existing record has it),
/// `Ok(false)` if a record with this token already exists.
pub fn validate_token_uniqueness(db: &Database, token: &str) -> Result<bool, OrchestratorError> {
    db.with_conn(|conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM mcp_containers WHERE bearer_token = ?1",
                params![token],
                |row| row.get(0),
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
        Ok(count == 0)
    })
}

/// Generate a bearer token that is guaranteed unique across all mcp_containers records.
///
/// Uses a retry cap of 10 attempts. With 256-bit tokens, collision is astronomically
/// unlikely — this cap exists to satisfy static analysis and prevent theoretical infinite loops.
pub fn generate_unique_bearer_token(db: &Database) -> Result<String, OrchestratorError> {
    for _ in 0..10 {
        let token = generate_bearer_token();
        if validate_token_uniqueness(db, &token)? {
            return Ok(token);
        }
    }
    Err(OrchestratorError::Internal(
        "token uniqueness check failed after 10 attempts".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_generate_bearer_token_length() {
        let token = generate_bearer_token();
        // 32 bytes encoded as base64url (no padding) = 43 characters
        assert_eq!(token.len(), 43);
    }

    #[test]
    fn test_generate_bearer_token_is_valid_base64url() {
        let token = generate_bearer_token();
        let decoded = URL_SAFE_NO_PAD.decode(&token).expect("should be valid base64url");
        assert_eq!(decoded.len(), TOKEN_BYTE_LENGTH);
    }

    #[test]
    fn test_generate_bearer_token_uniqueness() {
        // Generate multiple tokens and verify they are all distinct
        let tokens: HashSet<String> = (0..100).map(|_| generate_bearer_token()).collect();
        assert_eq!(tokens.len(), 100);
    }

    #[test]
    fn test_validate_token_uniqueness_empty_table() {
        let db = Database::open_in_memory().expect("Failed to open in-memory database");
        let token = generate_bearer_token();
        let is_unique = validate_token_uniqueness(&db, &token).unwrap();
        assert!(is_unique);
    }

    #[test]
    fn test_validate_token_uniqueness_existing_token() {
        let db = Database::open_in_memory().expect("Failed to open in-memory database");

        // Insert prerequisite data
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p1', 'test-persona', 'a1', '/tmp', 1, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO mcp_containers (id, persona_id, port, bearer_token, volume_name, status, created_at, updated_at)
                 VALUES ('mc1', 'p1', 9200, 'existing-token-value', 'vol-1', 'running', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            Ok(())
        }).unwrap();

        // Existing token should not be unique
        let is_unique = validate_token_uniqueness(&db, "existing-token-value").unwrap();
        assert!(!is_unique);

        // New token should be unique
        let new_token = generate_bearer_token();
        let is_unique = validate_token_uniqueness(&db, &new_token).unwrap();
        assert!(is_unique);
    }

    #[test]
    fn test_generate_unique_bearer_token() {
        let db = Database::open_in_memory().expect("Failed to open in-memory database");
        let token = generate_unique_bearer_token(&db).unwrap();
        // Should be a valid base64url token
        let decoded = URL_SAFE_NO_PAD.decode(&token).expect("should be valid base64url");
        assert_eq!(decoded.len(), TOKEN_BYTE_LENGTH);
    }
}
