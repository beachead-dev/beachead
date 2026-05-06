//! Port allocator for MCP containers.
//!
//! Manages a configurable range of localhost ports, persisting allocations
//! in the `port_allocations` SQLite table to prevent conflicts.

use std::sync::Arc;

use chrono::Utc;
use rusqlite::params;

use crate::db::Database;
use crate::error::OrchestratorError;

/// Allocates ports from a configured range for MCP containers.
///
/// Ports are persisted in the `port_allocations` table so that allocations
/// survive restarts and are visible to other components.
pub struct PortAllocator {
    db: Arc<Database>,
    range_start: u16,
    range_end: u16,
}

impl PortAllocator {
    /// Create a new `PortAllocator` managing ports in `[range_start, range_end]` (inclusive).
    pub fn new(db: Arc<Database>, range_start: u16, range_end: u16) -> Self {
        Self {
            db,
            range_start,
            range_end,
        }
    }

    /// Allocate the first available port in the range for the given MCP container.
    ///
    /// Inserts a record into `port_allocations`. Returns `PortExhaustion` if
    /// every port in the range is already allocated.
    pub fn allocate(&self, mcp_container_id: &str) -> Result<u16, OrchestratorError> {
        self.db.with_conn(|conn| {
            // Find the first port in range that is NOT in port_allocations
            let allocated_port: Option<u16> = {
                let mut stmt = conn
                    .prepare("SELECT port FROM port_allocations WHERE port >= ?1 AND port <= ?2")
                    .map_err(|e| OrchestratorError::Database(e.to_string()))?;

                let allocated: std::collections::HashSet<u16> = stmt
                    .query_map(params![self.range_start as i64, self.range_end as i64], |row| {
                        row.get::<_, i64>(0).map(|p| p as u16)
                    })
                    .map_err(|e| OrchestratorError::Database(e.to_string()))?
                    .filter_map(|r| r.ok())
                    .collect();

                (self.range_start..=self.range_end).find(|port| !allocated.contains(port))
            };

            match allocated_port {
                Some(port) => {
                    let now = Utc::now().to_rfc3339();
                    conn.execute(
                        "INSERT INTO port_allocations (port, mcp_container_id, allocated_at) VALUES (?1, ?2, ?3)",
                        params![port as i64, mcp_container_id, now],
                    )
                    .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                    Ok(port)
                }
                None => Err(OrchestratorError::PortExhaustion),
            }
        })
    }

    /// Release a previously allocated port, removing it from the `port_allocations` table.
    pub fn release(&self, port: u16) -> Result<(), OrchestratorError> {
        self.db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM port_allocations WHERE port = ?1",
                params![port as i64],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })
    }

    /// Check whether a port is available (not currently allocated).
    pub fn is_available(&self, port: u16) -> Result<bool, OrchestratorError> {
        self.db.with_conn(|conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM port_allocations WHERE port = ?1",
                    params![port as i64],
                    |row| row.get(0),
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(count == 0)
        })
    }

    /// Return the configured port range start (inclusive).
    pub fn range_start(&self) -> u16 {
        self.range_start
    }

    /// Return the configured port range end (inclusive).
    pub fn range_end(&self) -> u16 {
        self.range_end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_allocator(range_start: u16, range_end: u16) -> PortAllocator {
        let db = Arc::new(Database::open_in_memory().expect("Failed to open in-memory db"));

        // Insert a dummy mcp_container record to satisfy the foreign key constraint
        db.with_conn(|conn| {
            // First we need an agent_type and persona for the FK chain
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('at1', 'test-agent', 'test', 0, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p1', 'test-persona', 'at1', '/tmp', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            // Insert mcp_container records that we'll reference
            for i in 1..=20 {
                conn.execute(
                    "INSERT INTO mcp_containers (id, persona_id, port, bearer_token, volume_name, status, created_at, updated_at)
                     VALUES (?1, 'p1', ?2, 'token', 'vol', 'running', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                    params![format!("mc{}", i), 9000 + i],
                ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            }

            Ok(())
        })
        .unwrap();

        PortAllocator::new(db, range_start, range_end)
    }

    #[test]
    fn test_allocate_returns_first_available_port() {
        let allocator = setup_allocator(9000, 9010);
        let port = allocator.allocate("mc1").unwrap();
        assert_eq!(port, 9000);
    }

    #[test]
    fn test_allocate_skips_already_allocated() {
        let allocator = setup_allocator(9000, 9010);

        let p1 = allocator.allocate("mc1").unwrap();
        let p2 = allocator.allocate("mc2").unwrap();

        assert_eq!(p1, 9000);
        assert_eq!(p2, 9001);
    }

    #[test]
    fn test_release_makes_port_available_again() {
        let allocator = setup_allocator(9000, 9010);

        let port = allocator.allocate("mc1").unwrap();
        assert_eq!(port, 9000);
        assert!(!allocator.is_available(9000).unwrap());

        allocator.release(9000).unwrap();
        assert!(allocator.is_available(9000).unwrap());

        // Re-allocate should get 9000 again since it's the first available
        let port2 = allocator.allocate("mc2").unwrap();
        assert_eq!(port2, 9000);
    }

    #[test]
    fn test_is_available_for_unallocated_port() {
        let allocator = setup_allocator(9000, 9010);
        assert!(allocator.is_available(9000).unwrap());
        assert!(allocator.is_available(9005).unwrap());
    }

    #[test]
    fn test_is_available_for_allocated_port() {
        let allocator = setup_allocator(9000, 9010);
        allocator.allocate("mc1").unwrap();
        assert!(!allocator.is_available(9000).unwrap());
    }

    #[test]
    fn test_port_exhaustion_error() {
        // Range of only 3 ports
        let allocator = setup_allocator(9000, 9002);

        allocator.allocate("mc1").unwrap();
        allocator.allocate("mc2").unwrap();
        allocator.allocate("mc3").unwrap();

        let result = allocator.allocate("mc4");
        assert!(result.is_err());
        match result.unwrap_err() {
            OrchestratorError::PortExhaustion => {} // expected
            other => panic!("Expected PortExhaustion, got: {:?}", other),
        }
    }

    #[test]
    fn test_allocate_full_range_then_release_and_reallocate() {
        let allocator = setup_allocator(9000, 9001);

        let p1 = allocator.allocate("mc1").unwrap();
        let p2 = allocator.allocate("mc2").unwrap();
        assert_eq!(p1, 9000);
        assert_eq!(p2, 9001);

        // Exhausted
        assert!(allocator.allocate("mc3").is_err());

        // Release one
        allocator.release(9000).unwrap();

        // Should be able to allocate again
        let p3 = allocator.allocate("mc3").unwrap();
        assert_eq!(p3, 9000);
    }
}
