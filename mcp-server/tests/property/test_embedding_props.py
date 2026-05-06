"""Property-based tests for embedding model behavior.

Tests embedding determinism and dimension consistency using Hypothesis.
"""

import numpy as np
import pytest
from hypothesis import given, settings, assume
from hypothesis import strategies as st

from beachead_memory_mcp.embeddings import EmbeddingModel


@pytest.fixture(scope="module")
def model():
    """Load the embedding model once for all property tests."""
    return EmbeddingModel()


# Strategy for generating non-empty text strings
text_strategy = st.text(
    alphabet=st.characters(
        whitelist_categories=("L", "N", "P", "Z"),
        blacklist_characters="\x00",
    ),
    min_size=1,
    max_size=200,
).filter(lambda t: t.strip())


class TestEmbeddingDeterminism:
    """Property: Same input always produces the same embedding vector."""

    @given(text=text_strategy)
    @settings(max_examples=50, deadline=None)
    def test_embed_is_deterministic(self, text: str, model: EmbeddingModel):
        """For any text, embed(text) called twice produces identical vectors."""
        e1 = model.embed(text)
        e2 = model.embed(text)
        np.testing.assert_array_equal(e1, e2)


class TestEmbeddingDimensions:
    """Property: All vectors have consistent dimensionality."""

    @given(text=text_strategy)
    @settings(max_examples=50, deadline=None)
    def test_embed_always_returns_correct_dimension(
        self, text: str, model: EmbeddingModel
    ):
        """For any text, the embedding dimension matches the model's declared dimension."""
        embedding = model.embed(text)
        assert embedding.shape == (model.dimension,)

    @given(text=text_strategy)
    @settings(max_examples=50, deadline=None)
    def test_embed_always_returns_float32(self, text: str, model: EmbeddingModel):
        """For any text, the embedding dtype is always float32."""
        embedding = model.embed(text)
        assert embedding.dtype == np.float32

    @given(texts=st.lists(text_strategy, min_size=1, max_size=10))
    @settings(max_examples=20, deadline=None)
    def test_batch_embed_consistent_dimensions(
        self, texts: list[str], model: EmbeddingModel
    ):
        """For any batch of texts, all embeddings have the same dimension."""
        embeddings = model.embed_batch(texts)
        assert embeddings.shape == (len(texts), model.dimension)
        assert embeddings.dtype == np.float32
