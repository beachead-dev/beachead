use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::error::OrchestratorError;

/// Database connection pool (single-writer via Mutex for SQLite).
/// SQLite does not support true concurrent writes, so we serialize access.
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open (or create) the SQLite database at the given path and run migrations.
    pub fn open(path: &Path) -> Result<Self, OrchestratorError> {
        let conn = Connection::open(path)
            .map_err(|e| OrchestratorError::Database(format!("Failed to open database: {}", e)))?;

        // Enable WAL mode for better read concurrency
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| OrchestratorError::Database(format!("Failed to set WAL mode: {}", e)))?;

        // Enable foreign keys
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| {
                OrchestratorError::Database(format!("Failed to enable foreign keys: {}", e))
            })?;

        let db = Self {
            conn: Mutex::new(conn),
        };

        db.run_migrations()?;

        Ok(db)
    }

    /// Open an in-memory database (useful for testing).
    pub fn open_in_memory() -> Result<Self, OrchestratorError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| OrchestratorError::Database(format!("Failed to open in-memory db: {}", e)))?;

        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| {
                OrchestratorError::Database(format!("Failed to enable foreign keys: {}", e))
            })?;

        let db = Self {
            conn: Mutex::new(conn),
        };

        db.run_migrations()?;

        Ok(db)
    }

    /// Acquire a lock on the database connection and execute a closure.
    pub fn with_conn<F, T>(&self, f: F) -> Result<T, OrchestratorError>
    where
        F: FnOnce(&Connection) -> Result<T, OrchestratorError>,
    {
        let conn = self.conn.lock().map_err(|e| {
            OrchestratorError::Database(format!("Failed to acquire database lock: {}", e))
        })?;
        f(&conn)
    }

    /// Run all pending migrations in order.
    fn run_migrations(&self) -> Result<(), OrchestratorError> {
        let conn = self.conn.lock().map_err(|e| {
            OrchestratorError::Database(format!("Failed to acquire database lock: {}", e))
        })?;

        // Create the schema_version table if it doesn't exist
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )
        .map_err(|e| {
            OrchestratorError::Database(format!("Failed to create schema_version table: {}", e))
        })?;

        let current_version = get_current_version(&conn)?;

        for migration in MIGRATIONS.iter() {
            if migration.version > current_version {
                conn.execute_batch(migration.sql).map_err(|e| {
                    OrchestratorError::Database(format!(
                        "Migration {} ({}) failed: {}",
                        migration.version, migration.description, e
                    ))
                })?;

                conn.execute(
                    "INSERT INTO schema_version (version) VALUES (?1)",
                    params![migration.version],
                )
                .map_err(|e| {
                    OrchestratorError::Database(format!(
                        "Failed to record migration {}: {}",
                        migration.version, e
                    ))
                })?;
            }
        }

        Ok(())
    }
}

/// Get the current schema version from the database.
fn get_current_version(conn: &Connection) -> Result<i64, OrchestratorError> {
    let version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .map_err(|e| {
            OrchestratorError::Database(format!("Failed to query schema version: {}", e))
        })?;
    Ok(version)
}

/// A single migration with a version number, description, and SQL to execute.
struct Migration {
    version: i64,
    description: &'static str,
    sql: &'static str,
}

/// All migrations in order. New migrations are appended to this array.
static MIGRATIONS: &[Migration] = &[
    // Phase 1: Core orchestrator tables
    Migration {
        version: 1,
        description: "Create Phase 1 core tables",
        sql: MIGRATION_001_PHASE1_CORE,
    },
    // Phase 2: MCP containers and port allocations
    Migration {
        version: 2,
        description: "Create Phase 2 MCP container tables",
        sql: MIGRATION_002_PHASE2_MCP,
    },
    // Phase 3: Shared memory
    Migration {
        version: 3,
        description: "Create Phase 3 shared memory tables",
        sql: MIGRATION_003_PHASE3_SHARED_MEMORY,
    },
    // User settings (key-value store for preferences like theme)
    Migration {
        version: 4,
        description: "Create user_settings table",
        sql: MIGRATION_004_USER_SETTINGS,
    },
];

/// Migration 1: Phase 1 core tables (agent_types, personas, persona_mcp_servers, sessions)
const MIGRATION_001_PHASE1_CORE: &str = "
CREATE TABLE agent_types (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    sbx_agent   TEXT,
    kit_ref     TEXT,
    is_builtin  INTEGER NOT NULL DEFAULT 0,
    metadata    TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE personas (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    agent_type_id   TEXT NOT NULL REFERENCES agent_types(id),
    workspace_path  TEXT NOT NULL,
    memory_enabled  INTEGER NOT NULL DEFAULT 0,
    agent_cli_args  TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE TABLE persona_mcp_servers (
    id          TEXT PRIMARY KEY,
    persona_id  TEXT NOT NULL REFERENCES personas(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    url         TEXT NOT NULL,
    description TEXT,
    auth_headers TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    UNIQUE(persona_id, name)
);

CREATE TABLE sessions (
    id              TEXT PRIMARY KEY,
    persona_id      TEXT NOT NULL REFERENCES personas(id),
    sandbox_id      TEXT,
    kit_path        TEXT,
    status          TEXT NOT NULL,
    error_message   TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
";

/// Migration 2: Phase 2 MCP containers and port allocations
const MIGRATION_002_PHASE2_MCP: &str = "
CREATE TABLE mcp_containers (
    id              TEXT PRIMARY KEY,
    persona_id      TEXT REFERENCES personas(id),
    shared_memory_id TEXT,
    container_id    TEXT,
    port            INTEGER NOT NULL,
    bearer_token    TEXT NOT NULL,
    volume_name     TEXT NOT NULL,
    status          TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE TABLE port_allocations (
    port             INTEGER PRIMARY KEY,
    mcp_container_id TEXT NOT NULL REFERENCES mcp_containers(id) ON DELETE CASCADE,
    allocated_at     TEXT NOT NULL
);
";

/// Migration 3: Phase 3 shared memory
const MIGRATION_003_PHASE3_SHARED_MEMORY: &str = "
CREATE TABLE shared_memory (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    description TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE persona_shared_memory (
    persona_id       TEXT NOT NULL REFERENCES personas(id) ON DELETE CASCADE,
    shared_memory_id TEXT NOT NULL REFERENCES shared_memory(id) ON DELETE CASCADE,
    bearer_token     TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    PRIMARY KEY (persona_id, shared_memory_id)
);
";

/// Migration 4: User settings key-value store
const MIGRATION_004_USER_SETTINGS: &str = "
CREATE TABLE user_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory_runs_migrations() {
        let db = Database::open_in_memory().expect("Failed to open in-memory database");

        db.with_conn(|conn| {
            let version = get_current_version(conn)?;
            assert_eq!(version, 4, "All migrations should have been applied");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn test_phase1_tables_exist() {
        let db = Database::open_in_memory().expect("Failed to open in-memory database");

        db.with_conn(|conn| {
            // Verify all Phase 1 tables exist by querying their schema
            let tables = vec![
                "agent_types",
                "personas",
                "persona_mcp_servers",
                "sessions",
            ];

            for table in tables {
                let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                        params![table],
                        |row| row.get(0),
                    )
                    .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                assert_eq!(count, 1, "Table '{}' should exist", table);
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn test_phase2_tables_exist() {
        let db = Database::open_in_memory().expect("Failed to open in-memory database");

        db.with_conn(|conn| {
            let tables = vec!["mcp_containers", "port_allocations"];

            for table in tables {
                let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                        params![table],
                        |row| row.get(0),
                    )
                    .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                assert_eq!(count, 1, "Table '{}' should exist", table);
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn test_phase3_tables_exist() {
        let db = Database::open_in_memory().expect("Failed to open in-memory database");

        db.with_conn(|conn| {
            let tables = vec!["shared_memory", "persona_shared_memory"];

            for table in tables {
                let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                        params![table],
                        |row| row.get(0),
                    )
                    .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                assert_eq!(count, 1, "Table '{}' should exist", table);
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn test_foreign_key_constraint_on_personas() {
        let db = Database::open_in_memory().expect("Failed to open in-memory database");

        db.with_conn(|conn| {
            // Attempting to insert a persona with a non-existent agent_type_id should fail
            let result = conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p1', 'test', 'nonexistent', '/tmp', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            );
            assert!(result.is_err(), "Foreign key constraint should prevent insert");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn test_unique_constraint_on_persona_name() {
        let db = Database::open_in_memory().expect("Failed to open in-memory database");

        db.with_conn(|conn| {
            // Insert an agent type first
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            // Insert first persona
            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p1', 'my-persona', 'a1', '/tmp', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            // Attempt duplicate name
            let result = conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p2', 'my-persona', 'a1', '/other', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            );
            assert!(result.is_err(), "Unique constraint should prevent duplicate persona name");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn test_cascade_delete_persona_mcp_servers() {
        let db = Database::open_in_memory().expect("Failed to open in-memory database");

        db.with_conn(|conn| {
            // Insert agent type
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            // Insert persona
            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p1', 'test-persona', 'a1', '/tmp', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            // Insert MCP server entry
            conn.execute(
                "INSERT INTO persona_mcp_servers (id, persona_id, name, url, created_at, updated_at)
                 VALUES ('m1', 'p1', 'my-mcp', 'http://localhost:8080', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            // Delete persona — MCP server entry should cascade
            conn.execute("DELETE FROM personas WHERE id = 'p1'", [])
                .map_err(|e| OrchestratorError::Database(e.to_string()))?;

            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM persona_mcp_servers WHERE persona_id = 'p1'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            assert_eq!(count, 0, "MCP server entries should be cascade-deleted");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn test_idempotent_migrations() {
        // Opening the database twice should not fail (migrations already applied)
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let _db1 = Database::open(&db_path).expect("First open should succeed");
        drop(_db1);

        let db2 = Database::open(&db_path).expect("Second open should succeed");
        db2.with_conn(|conn| {
            let version = get_current_version(conn)?;
            assert_eq!(version, 4);
            Ok(())
        })
        .unwrap();
    }
}
