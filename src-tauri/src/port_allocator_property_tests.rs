//! Property-based tests for port allocation invariants.
//!
//! Property 13: Port allocation invariants
//! - All allocated ports within configured range
//! - No duplicate active allocations
//! - Released ports become available for future allocation
//! - Port exhaustion error when range is full
//!
//! **Validates: Requirements 16.1, 16.2, 16.3, 16.4**

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rusqlite::params;
    use std::collections::HashSet;
    use std::sync::Arc;

    use crate::db::Database;
    use crate::error::OrchestratorError;
    use crate::port_allocator::PortAllocator;

    /// Operation in an allocate/release sequence.
    #[derive(Debug, Clone)]
    enum PortOp {
        Allocate,
        /// Release the port at the given index in the list of currently allocated ports.
        Release(usize),
    }

    /// Strategy for generating port ranges small enough to test exhaustion (2-20 ports).
    fn port_range_strategy() -> impl Strategy<Value = (u16, u16)> {
        (9000u16..9200, 2u16..=20).prop_map(|(start, size)| {
            let end = start + size - 1;
            (start, end)
        })
    }

    /// Strategy for generating a sequence of allocate/release operations.
    fn port_ops_strategy() -> impl Strategy<Value = Vec<PortOp>> {
        prop::collection::vec(
            prop_oneof![
                3 => Just(PortOp::Allocate),
                1 => (0usize..20).prop_map(PortOp::Release),
            ],
            1..30,
        )
    }

    /// Set up a PortAllocator with an in-memory database and pre-seeded MCP container records.
    fn setup_allocator(range_start: u16, range_end: u16) -> PortAllocator {
        let db = Arc::new(Database::open_in_memory().expect("Failed to open in-memory db"));

        // Insert prerequisite records for the FK chain
        db.with_conn(|conn| {
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

            // Insert enough mcp_container records to satisfy FK constraints during allocation
            for i in 1..=50 {
                conn.execute(
                    "INSERT INTO mcp_containers (id, persona_id, port, bearer_token, volume_name, status, created_at, updated_at)
                     VALUES (?1, 'p1', ?2, 'token', 'vol', 'running', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                    params![format!("mc{}", i), 8000 + i],
                ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            }

            Ok(())
        })
        .unwrap();

        PortAllocator::new(db, range_start, range_end)
    }

    proptest! {
        /// Property 13: All allocated ports are within [range_start, range_end].
        ///
        /// **Validates: Requirements 16.1, 16.2**
        #[test]
        fn prop_allocated_ports_within_range(
            (range_start, range_end) in port_range_strategy(),
            ops in port_ops_strategy(),
        ) {
            let allocator = setup_allocator(range_start, range_end);
            let mut allocated: Vec<u16> = Vec::new();
            let mut container_idx = 0usize;

            for op in &ops {
                match op {
                    PortOp::Allocate => {
                        container_idx += 1;
                        let container_id = format!("mc{}", container_idx);
                        match allocator.allocate(&container_id) {
                            Ok(port) => {
                                prop_assert!(
                                    port >= range_start && port <= range_end,
                                    "Allocated port {} outside range [{}, {}]",
                                    port, range_start, range_end
                                );
                                allocated.push(port);
                            }
                            Err(OrchestratorError::PortExhaustion) => {
                                // Expected when range is full — not a violation
                            }
                            Err(e) => {
                                prop_assert!(false, "Unexpected error: {:?}", e);
                            }
                        }
                    }
                    PortOp::Release(idx) => {
                        if !allocated.is_empty() {
                            let actual_idx = idx % allocated.len();
                            let port = allocated.remove(actual_idx);
                            allocator.release(port).unwrap();
                        }
                    }
                }
            }
        }

        /// Property 13: No two active allocations share the same port.
        ///
        /// **Validates: Requirements 16.1, 16.2**
        #[test]
        fn prop_no_duplicate_active_allocations(
            (range_start, range_end) in port_range_strategy(),
            ops in port_ops_strategy(),
        ) {
            let allocator = setup_allocator(range_start, range_end);
            let mut active_ports: HashSet<u16> = HashSet::new();
            let mut allocated_list: Vec<u16> = Vec::new();
            let mut container_idx = 0usize;

            for op in &ops {
                match op {
                    PortOp::Allocate => {
                        container_idx += 1;
                        let container_id = format!("mc{}", container_idx);
                        match allocator.allocate(&container_id) {
                            Ok(port) => {
                                prop_assert!(
                                    !active_ports.contains(&port),
                                    "Duplicate allocation of port {} detected",
                                    port
                                );
                                active_ports.insert(port);
                                allocated_list.push(port);
                            }
                            Err(OrchestratorError::PortExhaustion) => {}
                            Err(e) => {
                                prop_assert!(false, "Unexpected error: {:?}", e);
                            }
                        }
                    }
                    PortOp::Release(idx) => {
                        if !allocated_list.is_empty() {
                            let actual_idx = idx % allocated_list.len();
                            let port = allocated_list.remove(actual_idx);
                            active_ports.remove(&port);
                            allocator.release(port).unwrap();
                        }
                    }
                }
            }
        }

        /// Property 13: A released port becomes available for future allocation.
        ///
        /// **Validates: Requirements 16.3, 16.4**
        #[test]
        fn prop_released_port_becomes_available(
            (range_start, range_end) in port_range_strategy(),
        ) {
            let allocator = setup_allocator(range_start, range_end);
            let range_size = (range_end - range_start + 1) as usize;

            // Allocate all ports in the range
            let mut allocated: Vec<u16> = Vec::new();
            for i in 0..range_size {
                let container_id = format!("mc{}", i + 1);
                let port = allocator.allocate(&container_id).unwrap();
                allocated.push(port);
            }

            // Confirm exhaustion
            let exhaust_result = allocator.allocate(&format!("mc{}", range_size + 1));
            prop_assert!(
                matches!(exhaust_result, Err(OrchestratorError::PortExhaustion)),
                "Expected PortExhaustion after allocating all ports"
            );

            // Release a port and verify it becomes available
            let released_port = allocated[0];
            allocator.release(released_port).unwrap();

            prop_assert!(
                allocator.is_available(released_port).unwrap(),
                "Released port {} should be available",
                released_port
            );

            // Allocate again — should get the released port back
            let new_port = allocator.allocate(&format!("mc{}", range_size + 2)).unwrap();
            prop_assert_eq!(
                new_port, released_port,
                "Re-allocation should return the released port"
            );
        }

        /// Property 13: When all ports are allocated, the next allocation returns PortExhaustion.
        ///
        /// **Validates: Requirements 16.3**
        #[test]
        fn prop_exhaustion_when_full(
            (range_start, range_end) in port_range_strategy(),
        ) {
            let allocator = setup_allocator(range_start, range_end);
            let range_size = (range_end - range_start + 1) as usize;

            // Allocate all ports
            for i in 0..range_size {
                let container_id = format!("mc{}", i + 1);
                let result = allocator.allocate(&container_id);
                prop_assert!(
                    result.is_ok(),
                    "Should be able to allocate port {} of {}",
                    i + 1, range_size
                );
            }

            // Next allocation must fail with PortExhaustion
            let result = allocator.allocate(&format!("mc{}", range_size + 1));
            match result {
                Err(OrchestratorError::PortExhaustion) => {} // expected
                Ok(port) => {
                    prop_assert!(
                        false,
                        "Expected PortExhaustion but got port {}",
                        port
                    );
                }
                Err(e) => {
                    prop_assert!(
                        false,
                        "Expected PortExhaustion but got error: {:?}",
                        e
                    );
                }
            }
        }
    }
}
