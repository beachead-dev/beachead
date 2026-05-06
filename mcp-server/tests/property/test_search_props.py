"""Property-based tests for vector store search behavior.

Tests search relevance and store/retrieve round-trip using Hypothesis.
"""

import numpy as np
import pytest
from hypothesis import given, settings, assume
from hypothesis import strategies as st

from beachead_memory_mcp.embeddings import EmbeddingModel
from beachead_memory_mcp.vector_store import VectorStore


@pytest.fixture(scope="module")
def model():
    """Load the embedding model once for all property tests."""
    return EmbeddingModel()


# Strategy for generating non-empty meaningful text
text_strategy = st.text(
    alphabet=st.characters(
        whitelist_categories=("L", "N", "P", "Z"),
        blacklist_characters="\x00",
    ),
    min_size=3,
    max_size=200,
).filter(lambda t: len(t.strip()) >= 3)


class TestStoreRetrieveRoundTrip:
    """Property: Any valid text stored can be retrieved."""

    @given(text=text_strategy)
    @settings(max_examples=30, deadline=None)
    def test_stored_text_retrievable_by_id(
        self, text: str, model: EmbeddingModel, tmp_path_factory
    ):
        """For any valid text, storing it and getting by ID returns the same text."""
        data_dir = tmp_path_factory.mktemp("store")
        store = VectorStore(data_dir=data_dir, dimension=model.dimension)
        try:
            embedding = model.embed(text.strip())
            entry_id = store.insert(text.strip(), embedding)

            entry = store.get(entry_id)
            assert entry is not None
            assert entry.text == text.strip()
            assert entry.id == entry_id
        finally:
            store.close()

    @given(text=text_strategy)
    @settings(max_examples=30, deadline=None)
    def test_stored_text_appears_in_list(
        self, text: str, model: EmbeddingModel, tmp_path_factory
    ):
        """For any valid text, storing it makes it appear in list_all."""
        data_dir = tmp_path_factory.mktemp("store")
        store = VectorStore(data_dir=data_dir, dimension=model.dimension)
        try:
            embedding = model.embed(text.strip())
            entry_id = store.insert(text.strip(), embedding)

            entries = store.list_all()
            entry_ids = [e.id for e in entries]
            assert entry_id in entry_ids
        finally:
            store.close()


class TestSearchRelevance:
    """Property: Stored items can be found via similarity search."""

    @given(
        texts=st.lists(text_strategy, min_size=2, max_size=8, unique=True),
        query_idx=st.integers(min_value=0),
    )
    @settings(max_examples=20, deadline=None)
    def test_stored_item_found_in_search(
        self,
        texts: list[str],
        query_idx: int,
        model: EmbeddingModel,
        tmp_path_factory,
    ):
        """Store N items, query for one of them — it should appear in results."""
        query_idx = query_idx % len(texts)
        query_text = texts[query_idx].strip()

        data_dir = tmp_path_factory.mktemp("store")
        store = VectorStore(data_dir=data_dir, dimension=model.dimension)
        try:
            # Store all texts
            stored_ids = []
            for text in texts:
                embedding = model.embed(text.strip())
                entry_id = store.insert(text.strip(), embedding)
                stored_ids.append(entry_id)

            # Query for one of them
            query_embedding = model.embed(query_text)
            results = store.search(query_embedding, top_k=len(texts))

            # The queried text should appear in results
            result_ids = [r.entry.id for r in results]
            assert stored_ids[query_idx] in result_ids
        finally:
            store.close()

    @given(
        texts=st.lists(
            st.text(
                alphabet=st.characters(whitelist_categories=("L", "N", "Z")),
                min_size=10,
                max_size=200,
            ).filter(lambda t: len(t.strip()) >= 10),
            min_size=3,
            max_size=8,
            unique=True,
        ),
        query_idx=st.integers(min_value=0),
    )
    @settings(max_examples=20, deadline=None)
    def test_self_query_ranks_first(
        self,
        texts: list[str],
        query_idx: int,
        model: EmbeddingModel,
        tmp_path_factory,
    ):
        """Querying with the exact stored text should rank it in the top results."""
        query_idx = query_idx % len(texts)
        query_text = texts[query_idx].strip()

        data_dir = tmp_path_factory.mktemp("store")
        store = VectorStore(data_dir=data_dir, dimension=model.dimension)
        try:
            stored_ids = []
            for text in texts:
                embedding = model.embed(text.strip())
                entry_id = store.insert(text.strip(), embedding)
                stored_ids.append(entry_id)

            query_embedding = model.embed(query_text)
            results = store.search(query_embedding, top_k=len(texts))

            # The exact match should be in the top 3 results
            top_ids = [r.entry.id for r in results[:3]]
            assert stored_ids[query_idx] in top_ids
        finally:
            store.close()
