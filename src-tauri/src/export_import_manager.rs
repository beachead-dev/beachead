//! Export/Import Manager: handles encrypted configuration export and import
//! with AES-256-GCM encryption and Argon2id key derivation.
//!
//! File format: [salt: 16 bytes][nonce: 12 bytes][ciphertext: variable]
//!
//! Security:
//! - Passwords and derived keys are zeroized from memory after use
//! - Secret values (API keys, bearer tokens) are never included in exports
//! - Only the list of configured secret service names is exported

use std::collections::HashMap;
use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::db::Database;
use crate::db_ops;
use crate::error::OrchestratorError;
use crate::sbx::PolicyRule;
use crate::types::{AdditionalWorkspace, AgentType, Persona, PersonaId};

// --- Constants ---

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32; // AES-256

// --- Export/Import Types ---

/// Preview of a persona in the import data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaPreview {
    pub id: String,
    pub name: String,
    pub agent_type_id: String,
    pub workspace_path: String,
    pub memory_enabled: bool,
}

/// Preview of an agent type in the import data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPreview {
    pub id: String,
    pub name: String,
    pub is_builtin: bool,
}

/// Warning about a workspace path that doesn't exist on the current host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceWarning {
    pub persona_name: String,
    pub workspace_path: String,
}

/// A conflict detected during import preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Conflict {
    PersonaNameConflict {
        imported_name: String,
        existing_id: String,
    },
}

/// User-provided resolutions for import conflicts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolutions {
    pub persona_resolutions: HashMap<String, ConflictAction>,
}

/// Action to take for a conflicting persona.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum ConflictAction {
    Rename { new_name: String },
    Skip,
    Overwrite,
}

/// Preview of what an import will do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportPreview {
    pub personas: Vec<PersonaPreview>,
    pub agents: Vec<AgentPreview>,
    pub policies: Vec<PolicyRule>,
    pub missing_secrets: Vec<String>,
    pub conflicts: Vec<Conflict>,
    pub invalid_workspaces: Vec<WorkspaceWarning>,
}

/// Summary of a completed import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSummary {
    pub personas_imported: usize,
    pub agents_imported: usize,
    pub personas_skipped: usize,
    pub secrets_needing_configuration: Vec<String>,
}

/// The serialized export payload (before encryption).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExportPayload {
    pub version: u32,
    pub personas: Vec<Persona>,
    pub agents: Vec<AgentType>,
    pub policies: Vec<PolicyRule>,
    pub configured_secret_services: Vec<String>,
    pub mcp_container_configs: Vec<McpContainerExport>,
    pub shared_memory_assignments: Vec<SharedMemoryAssignmentExport>,
    #[serde(default)]
    pub additional_workspaces: Vec<AdditionalWorkspace>,
}

/// MCP container config for export (excludes bearer_token).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct McpContainerExport {
    pub id: String,
    pub persona_id: Option<String>,
    pub shared_memory_id: Option<String>,
    pub port: i64,
    pub volume_name: String,
    pub status: String,
}

/// Shared memory assignment for export.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SharedMemoryAssignmentExport {
    pub persona_id: String,
    pub shared_memory_id: String,
}

// --- Manager ---

/// Manages encrypted configuration export and import.
pub struct ExportImportManager {
    db: Arc<Database>,
}

impl ExportImportManager {
    /// Create a new ExportImportManager.
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Export all configuration data to an encrypted byte vector.
    ///
    /// Serializes personas, agents, policies, MCP entries, MCP container configs,
    /// and shared memory assignments to JSON, then encrypts with AES-256-GCM
    /// using a key derived from the password via Argon2id.
    ///
    /// Security: password and derived key are zeroized from memory after use.
    pub fn export(&self, password: &str) -> Result<Vec<u8>, OrchestratorError> {
        // Collect all data from the database
        let payload = self.build_export_payload()?;

        // Serialize to JSON
        let plaintext = serde_json::to_vec(&payload)
            .map_err(|e| OrchestratorError::Internal(format!("Failed to serialize export: {}", e)))?;

        // Encrypt
        let encrypted = encrypt_data(&plaintext, password)?;

        Ok(encrypted)
    }

    /// Decrypt and preview import data without applying changes.
    ///
    /// Detects persona name conflicts and flags non-existent workspace paths.
    pub fn preview_import(
        &self,
        data: &[u8],
        password: &str,
    ) -> Result<ImportPreview, OrchestratorError> {
        let payload = decrypt_and_deserialize(data, password)?;

        // Build previews
        let personas: Vec<PersonaPreview> = payload
            .personas
            .iter()
            .map(|p| PersonaPreview {
                id: p.id.0.clone(),
                name: p.name.clone(),
                agent_type_id: p.agent_type_id.0.clone(),
                workspace_path: p.workspace_path.to_string_lossy().to_string(),
                memory_enabled: p.memory_enabled,
            })
            .collect();

        let agents: Vec<AgentPreview> = payload
            .agents
            .iter()
            .map(|a| AgentPreview {
                id: a.id.0.clone(),
                name: a.name.clone(),
                is_builtin: a.is_builtin,
            })
            .collect();

        // Detect persona name conflicts
        let conflicts = self.detect_conflicts(&payload)?;

        // Flag non-existent workspace paths
        let invalid_workspaces = self.detect_invalid_workspaces(&payload);

        Ok(ImportPreview {
            personas,
            agents,
            policies: payload.policies.clone(),
            missing_secrets: payload.configured_secret_services.clone(),
            conflicts,
            invalid_workspaces,
        })
    }

    /// Import configuration data, applying conflict resolutions.
    ///
    /// Returns a summary including which secrets need to be configured.
    pub fn import(
        &self,
        data: &[u8],
        password: &str,
        resolutions: &ConflictResolutions,
    ) -> Result<ImportSummary, OrchestratorError> {
        let payload = decrypt_and_deserialize(data, password)?;

        let mut personas_imported = 0;
        let mut agents_imported = 0;
        let mut personas_skipped = 0;

        // Import agents (skip built-ins that already exist)
        self.db.with_conn(|conn| {
            for agent in &payload.agents {
                if agent.is_builtin {
                    // Skip built-in agents; they should already exist
                    continue;
                }
                // Check if agent already exists by name
                let exists = db_ops::agent_type_name_exists(conn, &agent.name, None)?;
                if !exists {
                    db_ops::insert_agent_type(conn, agent)?;
                    agents_imported += 1;
                }
            }
            Ok(())
        })?;

        // Import personas with conflict resolution
        self.db.with_conn(|conn| {
            for persona in &payload.personas {
                let name_exists =
                    db_ops::persona_name_exists(conn, &persona.name, None)?;

                if name_exists {
                    // Check if there's a resolution for this persona
                    match resolutions.persona_resolutions.get(&persona.name) {
                        Some(ConflictAction::Skip) => {
                            personas_skipped += 1;
                            continue;
                        }
                        Some(ConflictAction::Rename { new_name }) => {
                            let mut renamed = persona.clone();
                            renamed.name = new_name.clone();
                            renamed.id = PersonaId::new();
                            db_ops::insert_persona(conn, &renamed)?;
                            // Import MCP servers for this persona
                            for mcp in &renamed.mcp_servers {
                                db_ops::insert_persona_mcp_server(conn, mcp)?;
                            }
                            // Import additional workspaces with the new persona_id
                            self.import_additional_workspaces_for_persona(
                                conn,
                                &payload.additional_workspaces,
                                &persona.id,
                                &renamed.id,
                            )?;
                            personas_imported += 1;
                        }
                        Some(ConflictAction::Overwrite) => {
                            // Find existing persona with same name and delete it
                            let existing_personas = db_ops::list_personas(conn)?;
                            if let Some(existing) =
                                existing_personas.iter().find(|p| p.name == persona.name)
                            {
                                // Delete sessions referencing this persona (no cascade)
                                conn.execute(
                                    "DELETE FROM sessions WHERE persona_id = ?1",
                                    rusqlite::params![existing.id.0],
                                ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
                                // Delete the existing persona (MCP servers and additional_workspaces cascade)
                                conn.execute(
                                    "DELETE FROM personas WHERE id = ?1",
                                    rusqlite::params![existing.id.0],
                                ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
                            }
                            // Insert the imported persona with a new ID
                            let mut imported = persona.clone();
                            imported.id = PersonaId::new();
                            db_ops::insert_persona(conn, &imported)?;
                            for mcp in &imported.mcp_servers {
                                db_ops::insert_persona_mcp_server(conn, mcp)?;
                            }
                            // Import additional workspaces with the new persona_id
                            self.import_additional_workspaces_for_persona(
                                conn,
                                &payload.additional_workspaces,
                                &persona.id,
                                &imported.id,
                            )?;
                            personas_imported += 1;
                        }
                        None => {
                            // No resolution provided, skip
                            personas_skipped += 1;
                        }
                    }
                } else {
                    // No conflict, import directly
                    db_ops::insert_persona(conn, persona)?;
                    for mcp in &persona.mcp_servers {
                        db_ops::insert_persona_mcp_server(conn, mcp)?;
                    }
                    // Import additional workspaces with the original persona_id
                    self.import_additional_workspaces_for_persona(
                        conn,
                        &payload.additional_workspaces,
                        &persona.id,
                        &persona.id,
                    )?;
                    personas_imported += 1;
                }
            }
            Ok(())
        })?;

        Ok(ImportSummary {
            personas_imported,
            agents_imported,
            personas_skipped,
            secrets_needing_configuration: payload.configured_secret_services,
        })
    }

    // --- Private helpers ---

    fn build_export_payload(&self) -> Result<ExportPayload, OrchestratorError> {
        self.db.with_conn(|conn| {
            let agents = db_ops::list_agent_types(conn)?;
            let personas = db_ops::list_personas(conn)?;

            // Collect configured secret service names from agent metadata
            let mut configured_secrets: Vec<String> = Vec::new();
            for agent in &agents {
                for secret in &agent.metadata.required_secrets {
                    if !configured_secrets.contains(secret) {
                        configured_secrets.push(secret.clone());
                    }
                }
            }

            // Read MCP container configs (excluding bearer_token)
            let mcp_container_configs = self.read_mcp_containers(conn)?;

            // Read shared memory assignments
            let shared_memory_assignments = self.read_shared_memory_assignments(conn)?;

            // Collect all additional workspaces across all personas
            let mut all_additional_workspaces: Vec<AdditionalWorkspace> = Vec::new();
            for persona in &personas {
                all_additional_workspaces.extend(persona.additional_workspaces.clone());
            }

            Ok(ExportPayload {
                version: 1,
                personas,
                agents,
                policies: Vec::new(), // Policies are managed via sbx CLI, not stored in DB
                configured_secret_services: configured_secrets,
                mcp_container_configs,
                shared_memory_assignments,
                additional_workspaces: all_additional_workspaces,
            })
        })
    }

    fn read_mcp_containers(
        &self,
        conn: &rusqlite::Connection,
    ) -> Result<Vec<McpContainerExport>, OrchestratorError> {
        let mut stmt = conn
            .prepare(
                "SELECT id, persona_id, shared_memory_id, port, volume_name, status FROM mcp_containers",
            )
            .map_err(|e| OrchestratorError::Database(format!("Failed to query mcp_containers: {}", e)))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(McpContainerExport {
                    id: row.get(0)?,
                    persona_id: row.get(1)?,
                    shared_memory_id: row.get(2)?,
                    port: row.get(3)?,
                    volume_name: row.get(4)?,
                    status: row.get(5)?,
                })
            })
            .map_err(|e| OrchestratorError::Database(format!("Failed to read mcp_containers: {}", e)))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(
                row.map_err(|e| {
                    OrchestratorError::Database(format!("Failed to parse mcp_container row: {}", e))
                })?,
            );
        }
        Ok(results)
    }

    fn read_shared_memory_assignments(
        &self,
        conn: &rusqlite::Connection,
    ) -> Result<Vec<SharedMemoryAssignmentExport>, OrchestratorError> {
        let mut stmt = conn
            .prepare("SELECT persona_id, shared_memory_id FROM persona_shared_memory")
            .map_err(|e| {
                OrchestratorError::Database(format!(
                    "Failed to query persona_shared_memory: {}",
                    e
                ))
            })?;

        let rows = stmt
            .query_map([], |row| {
                Ok(SharedMemoryAssignmentExport {
                    persona_id: row.get(0)?,
                    shared_memory_id: row.get(1)?,
                })
            })
            .map_err(|e| {
                OrchestratorError::Database(format!(
                    "Failed to read persona_shared_memory: {}",
                    e
                ))
            })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| {
                OrchestratorError::Database(format!(
                    "Failed to parse shared_memory_assignment row: {}",
                    e
                ))
            })?);
        }
        Ok(results)
    }

    fn detect_conflicts(&self, payload: &ExportPayload) -> Result<Vec<Conflict>, OrchestratorError> {
        self.db.with_conn(|conn| {
            let mut conflicts = Vec::new();
            let existing_personas = db_ops::list_personas(conn)?;

            for imported_persona in &payload.personas {
                if let Some(existing) = existing_personas
                    .iter()
                    .find(|p| p.name == imported_persona.name)
                {
                    conflicts.push(Conflict::PersonaNameConflict {
                        imported_name: imported_persona.name.clone(),
                        existing_id: existing.id.0.clone(),
                    });
                }
            }

            Ok(conflicts)
        })
    }

    fn detect_invalid_workspaces(&self, payload: &ExportPayload) -> Vec<WorkspaceWarning> {
        let mut warnings: Vec<WorkspaceWarning> = payload
            .personas
            .iter()
            .filter(|p| !p.workspace_path.exists())
            .map(|p| WorkspaceWarning {
                persona_name: p.name.clone(),
                workspace_path: p.workspace_path.to_string_lossy().to_string(),
            })
            .collect();

        // Also check additional workspace paths
        for persona in &payload.personas {
            for ws in &persona.additional_workspaces {
                if !ws.path.exists() {
                    warnings.push(WorkspaceWarning {
                        persona_name: persona.name.clone(),
                        workspace_path: ws.path.to_string_lossy().to_string(),
                    });
                }
            }
        }

        warnings
    }

    /// Import additional workspaces for a persona, mapping from the original persona_id
    /// to the target persona_id (which may differ due to rename/overwrite generating a new ID).
    fn import_additional_workspaces_for_persona(
        &self,
        conn: &rusqlite::Connection,
        all_workspaces: &[AdditionalWorkspace],
        original_persona_id: &PersonaId,
        target_persona_id: &PersonaId,
    ) -> Result<(), OrchestratorError> {
        for ws in all_workspaces
            .iter()
            .filter(|w| w.persona_id == *original_persona_id)
        {
            let mut imported_ws = ws.clone();
            imported_ws.id = Uuid::new_v4().to_string();
            imported_ws.persona_id = target_persona_id.clone();
            db_ops::insert_additional_workspace(conn, &imported_ws)?;
        }
        Ok(())
    }
}

// --- Encryption/Decryption helpers ---

/// Encrypt plaintext with AES-256-GCM using a key derived from password via Argon2id.
///
/// Returns: [salt: 16 bytes][nonce: 12 bytes][ciphertext]
pub(crate) fn encrypt_data(plaintext: &[u8], password: &str) -> Result<Vec<u8>, OrchestratorError> {
    // Generate random salt and nonce
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut salt);
    rand::rng().fill_bytes(&mut nonce_bytes);

    // Derive key from password using Argon2id
    let mut key = derive_key(password, &salt)?;

    // Encrypt with AES-256-GCM
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| OrchestratorError::Internal(format!("Failed to create cipher: {}", e)))?;

    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| OrchestratorError::Internal(format!("Encryption failed: {}", e)))?;

    // Zeroize the key from memory
    key.zeroize();

    // Assemble output: salt || nonce || ciphertext
    let mut output = Vec::with_capacity(SALT_LEN + NONCE_LEN + ciphertext.len());
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    Ok(output)
}

/// Decrypt data that was encrypted with encrypt_data.
pub(crate) fn decrypt_data(data: &[u8], password: &str) -> Result<Vec<u8>, OrchestratorError> {
    let min_len = SALT_LEN + NONCE_LEN + 1; // At least 1 byte of ciphertext
    if data.len() < min_len {
        return Err(OrchestratorError::DecryptionFailed(
            "Data too short to be a valid encrypted export".to_string(),
        ));
    }

    // Extract salt, nonce, and ciphertext
    let salt = &data[..SALT_LEN];
    let nonce_bytes = &data[SALT_LEN..SALT_LEN + NONCE_LEN];
    let ciphertext = &data[SALT_LEN + NONCE_LEN..];

    // Derive key from password
    let mut key = derive_key(password, salt)?;

    // Decrypt
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| OrchestratorError::Internal(format!("Failed to create cipher: {}", e)))?;

    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|_| {
        OrchestratorError::DecryptionFailed(
            "Decryption failed: incorrect password or corrupted data".to_string(),
        )
    })?;

    // Zeroize the key from memory
    key.zeroize();

    Ok(plaintext)
}

/// Derive a 256-bit key from a password and salt using Argon2id.
fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], OrchestratorError> {
    let mut key = [0u8; KEY_LEN];
    let argon2 = Argon2::default();

    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| {
            OrchestratorError::Internal(format!("Key derivation failed: {}", e))
        })?;

    Ok(key)
}

/// Decrypt and deserialize the export payload.
fn decrypt_and_deserialize(
    data: &[u8],
    password: &str,
) -> Result<ExportPayload, OrchestratorError> {
    let plaintext = decrypt_data(data, password)?;

    let payload: ExportPayload = serde_json::from_slice(&plaintext).map_err(|e| {
        OrchestratorError::DecryptionFailed(format!(
            "Failed to deserialize decrypted data: {}",
            e
        ))
    })?;

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let plaintext = b"Hello, World! This is a test payload.";
        let password = "test-password-123";

        let encrypted = encrypt_data(plaintext, password).unwrap();

        // Verify format: salt + nonce + ciphertext
        assert!(encrypted.len() > SALT_LEN + NONCE_LEN);

        let decrypted = decrypt_data(&encrypted, password).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_decrypt_wrong_password_fails() {
        let plaintext = b"Secret data";
        let password = "correct-password";

        let encrypted = encrypt_data(plaintext, password).unwrap();

        let result = decrypt_data(&encrypted, "wrong-password");
        assert!(result.is_err());
        match result.unwrap_err() {
            OrchestratorError::DecryptionFailed(_) => {}
            other => panic!("Expected DecryptionFailed, got: {:?}", other),
        }
    }

    #[test]
    fn test_decrypt_truncated_data_fails() {
        let result = decrypt_data(&[0u8; 10], "password");
        assert!(result.is_err());
        match result.unwrap_err() {
            OrchestratorError::DecryptionFailed(_) => {}
            other => panic!("Expected DecryptionFailed, got: {:?}", other),
        }
    }

    #[test]
    fn test_decrypt_corrupted_data_fails() {
        let plaintext = b"Test data";
        let password = "password";

        let mut encrypted = encrypt_data(plaintext, password).unwrap();
        // Corrupt the ciphertext
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0xFF;

        let result = decrypt_data(&encrypted, password);
        assert!(result.is_err());
    }

    #[test]
    fn test_different_encryptions_produce_different_output() {
        let plaintext = b"Same data";
        let password = "same-password";

        let encrypted1 = encrypt_data(plaintext, password).unwrap();
        let encrypted2 = encrypt_data(plaintext, password).unwrap();

        // Different salt/nonce means different ciphertext
        assert_ne!(encrypted1, encrypted2);

        // But both decrypt to the same plaintext
        assert_eq!(
            decrypt_data(&encrypted1, password).unwrap(),
            decrypt_data(&encrypted2, password).unwrap()
        );
    }

    #[test]
    fn test_export_import_roundtrip() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let manager = ExportImportManager::new(db.clone());

        // Insert an agent type
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{\"required_secrets\":[\"anthropic\"],\"auth_methods\":[\"api_key\"],\"description\":\"Claude Code\",\"supports_interactive_auth\":true}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            // Insert a persona
            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, agent_cli_args, created_at, updated_at)
                 VALUES ('p1', 'my-persona', 'a1', '/tmp', 0, '[]', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            Ok(())
        }).unwrap();

        // Export
        let password = "export-password";
        let exported = manager.export(password).unwrap();

        // Preview import on a fresh database
        let db2 = Arc::new(Database::open_in_memory().unwrap());
        let manager2 = ExportImportManager::new(db2.clone());

        let preview = manager2.preview_import(&exported, password).unwrap();
        assert_eq!(preview.personas.len(), 1);
        assert_eq!(preview.personas[0].name, "my-persona");
        assert_eq!(preview.agents.len(), 1);
        assert!(preview.conflicts.is_empty());
        assert_eq!(preview.missing_secrets, vec!["anthropic"]);
    }

    #[test]
    fn test_import_detects_persona_name_conflict() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let manager = ExportImportManager::new(db.clone());

        // Insert agent type and persona
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{\"required_secrets\":[],\"auth_methods\":[],\"description\":\"Claude\",\"supports_interactive_auth\":false}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, agent_cli_args, created_at, updated_at)
                 VALUES ('p1', 'conflicting-name', 'a1', '/tmp', 0, '[]', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            Ok(())
        }).unwrap();

        // Export
        let password = "test";
        let exported = manager.export(password).unwrap();

        // Import into same database (will have conflict)
        let preview = manager.preview_import(&exported, password).unwrap();
        assert_eq!(preview.conflicts.len(), 1);
        match &preview.conflicts[0] {
            Conflict::PersonaNameConflict {
                imported_name,
                existing_id,
            } => {
                assert_eq!(imported_name, "conflicting-name");
                assert_eq!(existing_id, "p1");
            }
        }
    }

    #[test]
    fn test_import_with_skip_resolution() {
        let db = Arc::new(Database::open_in_memory().unwrap());

        // Set up source database with data
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{\"required_secrets\":[],\"auth_methods\":[],\"description\":\"Claude\",\"supports_interactive_auth\":false}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, agent_cli_args, created_at, updated_at)
                 VALUES ('p1', 'test-persona', 'a1', '/tmp', 0, '[]', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            Ok(())
        }).unwrap();

        let manager = ExportImportManager::new(db.clone());
        let password = "test";
        let exported = manager.export(password).unwrap();

        // Import with skip resolution for the conflicting persona
        let resolutions = ConflictResolutions {
            persona_resolutions: {
                let mut map = HashMap::new();
                map.insert("test-persona".to_string(), ConflictAction::Skip);
                map
            },
        };

        let summary = manager.import(&exported, password, &resolutions).unwrap();
        assert_eq!(summary.personas_skipped, 1);
        assert_eq!(summary.personas_imported, 0);
    }
}
