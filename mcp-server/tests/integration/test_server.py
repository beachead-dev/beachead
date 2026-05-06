"""Integration tests for the MCP server.

Tests server startup, health check, full MCP tool invocation,
store/query round-trip, and data persistence across restarts.
"""

import os

import pytest

# Configure environment before imports
os.environ["BEACHEAD_BEARER_TOKEN"] = "integration-test-token"
os.environ["BEACHEAD_PORT"] = "9199"

from beachead_memory_mcp.server import (
    memory_delete,
    memory_list,
    memory_query,
    memory_store,
)


@pytest.fixture(autouse=True)
def reset_server_state(tmp_path, monkeypatch):
    """Reset server globals and use temp directory for each test."""
    import beachead_memory_mcp.server as server_module

    monkeypatch.setattr(server_module, "DATA_DIR", tmp_path / "data")
    monkeypatch.setattr(server_module, "_embedding_model", None)
    monkeypatch.setattr(server_module, "_vector_store", None)


class TestHealthCheck:
    """Tests for the health check endpoint."""

    def test_health_check_via_app(self):
        """Health check endpoint should return healthy status."""
        from starlette.testclient import TestClient

        import beachead_memory_mcp.server as server_module

        # Ensure the server module is initialized
        app = server_module.create_app()
        client = TestClient(app)

        response = client.get("/health")
        assert response.status_code == 200
        data = response.json()
        assert data["status"] == "healthy"
        assert "entries_count" in data
        assert "model_dimension" in data
        assert data["model_dimension"] == 384


class TestFullMcpToolInvocation:
    """Integration tests for MCP tool invocations."""

    def test_store_then_query_round_trip(self):
        """Store a fact, then query for it — should be found."""
        # Store
        store_result = memory_store("The database uses PostgreSQL version 15")
        assert store_result["status"] == "stored"
        entry_id = store_result["id"]

        # Query
        query_result = memory_query("what database is used")
        assert query_result["total_results"] > 0
        found_ids = [r["id"] for r in query_result["results"]]
        assert entry_id in found_ids

    def test_store_multiple_then_query_relevance(self):
        """Store multiple facts, query should return most relevant."""
        memory_store("Python is used for the MCP server")
        memory_store("Rust is used for the backend")
        memory_store("React is used for the frontend")
        memory_store("SQLite is used for persistence")

        result = memory_query("what language is the backend written in")
        assert result["total_results"] > 0
        # Rust entry should be highly ranked
        top_texts = [r["text"] for r in result["results"][:2]]
        assert any("Rust" in t for t in top_texts)

    def test_store_query_delete_query(self):
        """Full lifecycle: store → query → delete → query again."""
        # Store
        store_result = memory_store("Temporary fact that will be deleted")
        entry_id = store_result["id"]

        # Verify it's findable
        query_result = memory_query("temporary fact")
        found_ids = [r["id"] for r in query_result["results"]]
        assert entry_id in found_ids

        # Delete
        delete_result = memory_delete(entry_id)
        assert delete_result["status"] == "deleted"

        # Verify it's gone
        list_result = memory_list()
        list_ids = [e["id"] for e in list_result["entries"]]
        assert entry_id not in list_ids

    def test_store_with_metadata_preserved(self):
        """Metadata should be preserved through store and retrieval."""
        metadata = {"source": "meeting", "date": "2024-03-15", "priority": "high"}
        store_result = memory_store(
            "Decision: use event sourcing for audit trail",
            metadata=metadata,
        )
        entry_id = store_result["id"]

        # Check via list
        list_result = memory_list()
        entry = next(e for e in list_result["entries"] if e["id"] == entry_id)
        assert entry["metadata"] == metadata

        # Check via query
        query_result = memory_query("event sourcing")
        result = next(r for r in query_result["results"] if r["id"] == entry_id)
        assert result["metadata"] == metadata

    def test_multiple_stores_and_list(self):
        """Multiple stores should all appear in list."""
        ids = []
        for i in range(5):
            result = memory_store(f"Knowledge item number {i}")
            ids.append(result["id"])

        list_result = memory_list()
        assert list_result["total_count"] == 5
        listed_ids = [e["id"] for e in list_result["entries"]]
        for entry_id in ids:
            assert entry_id in listed_ids


class TestDataPersistence:
    """Tests for data persistence across server restarts."""

    def test_data_survives_store_reinitialization(self, tmp_path, monkeypatch):
        """Data should persist when the vector store is reinitialized."""
        import beachead_memory_mcp.server as server_module

        data_dir = tmp_path / "persist_data"

        # First "session" - store data
        monkeypatch.setattr(server_module, "DATA_DIR", data_dir)
        monkeypatch.setattr(server_module, "_embedding_model", None)
        monkeypatch.setattr(server_module, "_vector_store", None)

        store_result = memory_store("Persistent knowledge that must survive restart")
        entry_id = store_result["id"]

        # Simulate restart by resetting the vector store (but keeping same data dir)
        monkeypatch.setattr(server_module, "_vector_store", None)

        # Second "session" - verify data is still there
        list_result = memory_list()
        assert list_result["total_count"] == 1
        assert list_result["entries"][0]["id"] == entry_id
        assert list_result["entries"][0]["text"] == "Persistent knowledge that must survive restart"

    def test_search_works_after_reinitialization(self, tmp_path, monkeypatch):
        """Search should work correctly after store reinitialization."""
        import beachead_memory_mcp.server as server_module

        data_dir = tmp_path / "persist_search"

        # Store data
        monkeypatch.setattr(server_module, "DATA_DIR", data_dir)
        monkeypatch.setattr(server_module, "_embedding_model", None)
        monkeypatch.setattr(server_module, "_vector_store", None)

        memory_store("The architecture uses microservices")
        memory_store("Testing is done with pytest")

        # Reinitialize
        monkeypatch.setattr(server_module, "_vector_store", None)

        # Search should still work
        result = memory_query("architecture pattern")
        assert result["total_results"] > 0
        assert any("microservices" in r["text"] for r in result["results"])
