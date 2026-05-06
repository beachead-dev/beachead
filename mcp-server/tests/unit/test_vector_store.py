"""Unit tests for the FAISS vector store."""

import numpy as np
import pytest

from beachead_memory_mcp.vector_store import VectorStore


@pytest.fixture
def store(tmp_path):
    """Create a fresh vector store for each test."""
    s = VectorStore(data_dir=tmp_path / "test_store", dimension=4)
    yield s
    s.close()


def _random_embedding(dim: int = 4) -> np.ndarray:
    """Generate a random normalized embedding vector."""
    v = np.random.randn(dim).astype(np.float32)
    return v / np.linalg.norm(v)


class TestVectorStoreInsert:
    """Tests for inserting entries into the vector store."""

    def test_insert_returns_id(self, store: VectorStore):
        """Insert should return a non-empty string ID."""
        embedding = _random_embedding(4)
        entry_id = store.insert("test text", embedding)
        assert isinstance(entry_id, str)
        assert len(entry_id) > 0

    def test_insert_increments_count(self, store: VectorStore):
        """Each insert should increment the entry count."""
        assert store.count == 0
        store.insert("first", _random_embedding(4))
        assert store.count == 1
        store.insert("second", _random_embedding(4))
        assert store.count == 2

    def test_insert_with_metadata(self, store: VectorStore):
        """Insert with metadata should persist the metadata."""
        meta = {"source": "test", "topic": "unit testing"}
        entry_id = store.insert("text with meta", _random_embedding(4), metadata=meta)
        entry = store.get(entry_id)
        assert entry is not None
        assert entry.metadata == meta

    def test_insert_without_metadata(self, store: VectorStore):
        """Insert without metadata should default to empty dict."""
        entry_id = store.insert("no meta", _random_embedding(4))
        entry = store.get(entry_id)
        assert entry is not None
        assert entry.metadata == {}

    def test_insert_wrong_dimension_raises(self, store: VectorStore):
        """Insert with wrong dimension should raise ValueError."""
        wrong_dim = np.random.randn(8).astype(np.float32)
        with pytest.raises(ValueError, match="dimension"):
            store.insert("wrong dim", wrong_dim)

    def test_insert_preserves_text(self, store: VectorStore):
        """Inserted text should be retrievable unchanged."""
        text = "This is important knowledge about the project."
        entry_id = store.insert(text, _random_embedding(4))
        entry = store.get(entry_id)
        assert entry is not None
        assert entry.text == text


class TestVectorStoreSearch:
    """Tests for similarity search."""

    def test_search_empty_store_returns_empty(self, store: VectorStore):
        """Search on empty store should return empty list."""
        results = store.search(_random_embedding(4), top_k=5)
        assert results == []

    def test_search_returns_results(self, store: VectorStore):
        """Search should return results after inserting entries."""
        for i in range(5):
            store.insert(f"entry {i}", _random_embedding(4))
        results = store.search(_random_embedding(4), top_k=3)
        assert len(results) <= 3
        assert len(results) > 0

    def test_search_results_have_scores(self, store: VectorStore):
        """Each search result should have a numeric score."""
        store.insert("test entry", _random_embedding(4))
        results = store.search(_random_embedding(4), top_k=1)
        assert len(results) == 1
        assert isinstance(results[0].score, float)

    def test_search_finds_similar_vectors(self, store: VectorStore):
        """Search should rank similar vectors higher."""
        # Create a known vector and a very similar one
        target = np.array([1.0, 0.0, 0.0, 0.0], dtype=np.float32)
        similar = np.array([0.9, 0.1, 0.0, 0.0], dtype=np.float32)
        dissimilar = np.array([0.0, 0.0, 0.0, 1.0], dtype=np.float32)

        store.insert("similar to target", similar)
        store.insert("dissimilar to target", dissimilar)

        results = store.search(target, top_k=2)
        assert len(results) == 2
        # The similar vector should score higher
        assert results[0].entry.text == "similar to target"

    def test_search_top_k_limits_results(self, store: VectorStore):
        """Search should return at most top_k results."""
        for i in range(10):
            store.insert(f"entry {i}", _random_embedding(4))
        results = store.search(_random_embedding(4), top_k=3)
        assert len(results) == 3

    def test_search_top_k_larger_than_store(self, store: VectorStore):
        """top_k larger than store size should return all entries."""
        for i in range(3):
            store.insert(f"entry {i}", _random_embedding(4))
        results = store.search(_random_embedding(4), top_k=100)
        assert len(results) == 3


class TestVectorStoreListAll:
    """Tests for listing all entries."""

    def test_list_empty_store(self, store: VectorStore):
        """List on empty store should return empty list."""
        entries = store.list_all()
        assert entries == []

    def test_list_returns_all_entries(self, store: VectorStore):
        """List should return all inserted entries."""
        store.insert("first", _random_embedding(4))
        store.insert("second", _random_embedding(4))
        store.insert("third", _random_embedding(4))
        entries = store.list_all()
        assert len(entries) == 3

    def test_list_entries_have_correct_fields(self, store: VectorStore):
        """Listed entries should have all required fields."""
        store.insert("test", _random_embedding(4), metadata={"key": "val"})
        entries = store.list_all()
        assert len(entries) == 1
        entry = entries[0]
        assert entry.id
        assert entry.text == "test"
        assert entry.metadata == {"key": "val"}
        assert entry.created_at


class TestVectorStoreDelete:
    """Tests for deleting entries."""

    def test_delete_existing_entry(self, store: VectorStore):
        """Delete should return True for existing entry."""
        entry_id = store.insert("to delete", _random_embedding(4))
        assert store.delete(entry_id) is True
        assert store.count == 0

    def test_delete_nonexistent_entry(self, store: VectorStore):
        """Delete should return False for non-existent entry."""
        assert store.delete("nonexistent-id") is False

    def test_delete_removes_from_search(self, store: VectorStore):
        """Deleted entries should not appear in search results."""
        target = np.array([1.0, 0.0, 0.0, 0.0], dtype=np.float32)
        entry_id = store.insert("will be deleted", target)
        store.insert("will remain", _random_embedding(4))

        store.delete(entry_id)

        results = store.search(target, top_k=10)
        for r in results:
            assert r.entry.id != entry_id

    def test_delete_removes_from_list(self, store: VectorStore):
        """Deleted entries should not appear in list_all."""
        entry_id = store.insert("to delete", _random_embedding(4))
        store.insert("to keep", _random_embedding(4))

        store.delete(entry_id)

        entries = store.list_all()
        assert len(entries) == 1
        assert entries[0].text == "to keep"

    def test_delete_then_get_returns_none(self, store: VectorStore):
        """Get after delete should return None."""
        entry_id = store.insert("ephemeral", _random_embedding(4))
        store.delete(entry_id)
        assert store.get(entry_id) is None


class TestVectorStorePersistence:
    """Tests for data persistence across store instances."""

    def test_data_persists_across_instances(self, tmp_path):
        """Data should survive closing and reopening the store."""
        data_dir = tmp_path / "persist_test"

        # Write data
        store1 = VectorStore(data_dir=data_dir, dimension=4)
        entry_id = store1.insert("persistent data", _random_embedding(4))
        store1.close()

        # Read data in new instance
        store2 = VectorStore(data_dir=data_dir, dimension=4)
        entry = store2.get(entry_id)
        assert entry is not None
        assert entry.text == "persistent data"
        store2.close()

    def test_count_persists(self, tmp_path):
        """Entry count should be correct after reopening."""
        data_dir = tmp_path / "count_test"

        store1 = VectorStore(data_dir=data_dir, dimension=4)
        store1.insert("one", _random_embedding(4))
        store1.insert("two", _random_embedding(4))
        store1.close()

        store2 = VectorStore(data_dir=data_dir, dimension=4)
        assert store2.count == 2
        store2.close()
