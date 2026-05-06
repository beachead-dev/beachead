"""Unit tests for MCP tool handlers."""

import os
from unittest.mock import patch

import pytest

# Set environment before importing server module
os.environ.setdefault("BEACHEAD_DATA_DIR", "/tmp/test_mcp_tools")
os.environ.setdefault("BEACHEAD_BEARER_TOKEN", "")


from beachead_memory_mcp.server import (
    memory_delete,
    memory_list,
    memory_query,
    memory_store,
    _get_embedding_model,
    _get_vector_store,
)


@pytest.fixture(autouse=True)
def reset_globals(tmp_path, monkeypatch):
    """Reset global state and use temp directory for each test."""
    import beachead_memory_mcp.server as server_module

    monkeypatch.setattr(server_module, "DATA_DIR", tmp_path / "data")
    monkeypatch.setattr(server_module, "_embedding_model", None)
    monkeypatch.setattr(server_module, "_vector_store", None)


class TestMemoryStore:
    """Tests for the memory_store tool."""

    def test_store_valid_text(self):
        """Storing valid text should return success with ID."""
        result = memory_store("Important fact about the project")
        assert "id" in result
        assert result["status"] == "stored"
        assert result["text_length"] > 0

    def test_store_empty_text_returns_error(self):
        """Empty text should return an error."""
        result = memory_store("")
        assert "error" in result

    def test_store_whitespace_only_returns_error(self):
        """Whitespace-only text should return an error."""
        result = memory_store("   \n\t  ")
        assert "error" in result

    def test_store_with_metadata(self):
        """Storing with metadata should succeed."""
        result = memory_store(
            "A decision was made",
            metadata={"source": "meeting", "date": "2024-01-01"},
        )
        assert result["status"] == "stored"

    def test_store_strips_whitespace(self):
        """Text should be stripped of leading/trailing whitespace."""
        result = memory_store("  trimmed text  ")
        assert result["text_length"] == len("trimmed text")


class TestMemoryQuery:
    """Tests for the memory_query tool."""

    def test_query_empty_store(self):
        """Query on empty store should return empty results."""
        result = memory_query("anything")
        assert result["results"] == []
        assert result["total_results"] == 0

    def test_query_returns_results_after_store(self):
        """Query should find stored entries."""
        memory_store("Python is a programming language")
        result = memory_query("programming language")
        assert result["total_results"] > 0
        assert len(result["results"]) > 0

    def test_query_empty_string_returns_error(self):
        """Empty query should return an error."""
        result = memory_query("")
        assert "error" in result

    def test_query_whitespace_only_returns_error(self):
        """Whitespace-only query should return an error."""
        result = memory_query("   ")
        assert "error" in result

    def test_query_respects_top_k(self):
        """Query should return at most top_k results."""
        for i in range(10):
            memory_store(f"Entry number {i} about various topics")
        result = memory_query("topics", top_k=3)
        assert result["total_results"] <= 3

    def test_query_top_k_clamped_to_minimum(self):
        """top_k below 1 should be clamped to 1."""
        memory_store("test entry")
        result = memory_query("test", top_k=0)
        assert result["total_results"] <= 1

    def test_query_results_have_required_fields(self):
        """Each result should have id, text, score, metadata, created_at."""
        memory_store("test knowledge")
        result = memory_query("knowledge")
        assert result["total_results"] > 0
        r = result["results"][0]
        assert "id" in r
        assert "text" in r
        assert "score" in r
        assert "metadata" in r
        assert "created_at" in r


class TestMemoryList:
    """Tests for the memory_list tool."""

    def test_list_empty_store(self):
        """List on empty store should return empty entries."""
        result = memory_list()
        assert result["entries"] == []
        assert result["total_count"] == 0

    def test_list_returns_stored_entries(self):
        """List should return all stored entries."""
        memory_store("first entry")
        memory_store("second entry")
        result = memory_list()
        assert result["total_count"] == 2
        assert len(result["entries"]) == 2

    def test_list_entries_have_required_fields(self):
        """Each listed entry should have id, text, metadata, created_at."""
        memory_store("test", metadata={"key": "value"})
        result = memory_list()
        entry = result["entries"][0]
        assert "id" in entry
        assert "text" in entry
        assert "metadata" in entry
        assert "created_at" in entry

    def test_list_respects_limit(self):
        """List should return at most 'limit' entries."""
        for i in range(10):
            memory_store(f"entry {i}")
        result = memory_list(limit=3)
        assert len(result["entries"]) == 3
        assert result["total_count"] == 10

    def test_list_respects_offset(self):
        """List should skip 'offset' entries."""
        for i in range(5):
            memory_store(f"entry {i}")
        result = memory_list(limit=10, offset=3)
        assert len(result["entries"]) == 2
        assert result["total_count"] == 5

    def test_list_clamps_negative_offset(self):
        """Negative offset should be clamped to 0."""
        memory_store("test")
        result = memory_list(offset=-5)
        assert result["offset"] == 0


class TestMemoryDelete:
    """Tests for the memory_delete tool."""

    def test_delete_existing_entry(self):
        """Deleting an existing entry should return 'deleted' status."""
        store_result = memory_store("to be deleted")
        entry_id = store_result["id"]
        result = memory_delete(entry_id)
        assert result["status"] == "deleted"
        assert result["id"] == entry_id

    def test_delete_nonexistent_entry(self):
        """Deleting a non-existent entry should return 'not_found'."""
        result = memory_delete("nonexistent-uuid")
        assert result["status"] == "not_found"

    def test_delete_empty_id_returns_error(self):
        """Empty ID should return an error."""
        result = memory_delete("")
        assert "error" in result

    def test_delete_removes_from_query(self):
        """Deleted entries should not appear in query results."""
        store_result = memory_store("unique deletable content xyz123")
        entry_id = store_result["id"]
        memory_delete(entry_id)
        query_result = memory_query("unique deletable content xyz123")
        for r in query_result["results"]:
            assert r["id"] != entry_id

    def test_delete_removes_from_list(self):
        """Deleted entries should not appear in list."""
        store_result = memory_store("will be removed")
        entry_id = store_result["id"]
        memory_store("will remain")
        memory_delete(entry_id)
        list_result = memory_list()
        assert list_result["total_count"] == 1
        assert list_result["entries"][0]["text"] == "will remain"
