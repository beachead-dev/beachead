"""Unit tests for the embedding model."""

import numpy as np
import pytest

from beachead_memory_mcp.embeddings import DEFAULT_MODEL_NAME, EmbeddingModel


@pytest.fixture(scope="module")
def model():
    """Load the embedding model once for all tests in this module."""
    return EmbeddingModel()


class TestEmbeddingModel:
    """Tests for EmbeddingModel initialization and vector generation."""

    def test_model_loads_successfully(self, model: EmbeddingModel):
        """Model should load without errors."""
        assert model is not None

    def test_dimension_is_positive(self, model: EmbeddingModel):
        """Model dimension should be a positive integer."""
        assert model.dimension > 0

    def test_default_model_dimension_is_384(self, model: EmbeddingModel):
        """all-MiniLM-L6-v2 produces 384-dimensional embeddings."""
        assert model.dimension == 384

    def test_embed_returns_correct_shape(self, model: EmbeddingModel):
        """Single embedding should be a 1-D array of correct dimension."""
        embedding = model.embed("hello world")
        assert embedding.shape == (384,)

    def test_embed_returns_float32(self, model: EmbeddingModel):
        """Embeddings should be float32 arrays."""
        embedding = model.embed("test text")
        assert embedding.dtype == np.float32

    def test_embed_deterministic(self, model: EmbeddingModel):
        """Same input should always produce the same embedding."""
        text = "The quick brown fox jumps over the lazy dog"
        e1 = model.embed(text)
        e2 = model.embed(text)
        np.testing.assert_array_equal(e1, e2)

    def test_embed_different_texts_produce_different_vectors(self, model: EmbeddingModel):
        """Different inputs should produce different embeddings."""
        e1 = model.embed("cats are great pets")
        e2 = model.embed("quantum physics is complex")
        # They should not be identical
        assert not np.array_equal(e1, e2)

    def test_embed_batch_returns_correct_shape(self, model: EmbeddingModel):
        """Batch embedding should return (N, dimension) array."""
        texts = ["hello", "world", "test"]
        embeddings = model.embed_batch(texts)
        assert embeddings.shape == (3, 384)

    def test_embed_batch_returns_float32(self, model: EmbeddingModel):
        """Batch embeddings should be float32."""
        texts = ["one", "two"]
        embeddings = model.embed_batch(texts)
        assert embeddings.dtype == np.float32

    def test_embed_batch_matches_individual(self, model: EmbeddingModel):
        """Batch embedding should match individual embeddings."""
        texts = ["alpha", "beta", "gamma"]
        batch = model.embed_batch(texts)
        for i, text in enumerate(texts):
            individual = model.embed(text)
            np.testing.assert_allclose(batch[i], individual, atol=1e-5)

    def test_embed_empty_string(self, model: EmbeddingModel):
        """Empty string should still produce a valid embedding."""
        embedding = model.embed("")
        assert embedding.shape == (384,)
        assert embedding.dtype == np.float32

    def test_embed_long_text(self, model: EmbeddingModel):
        """Long text should produce a valid embedding without error."""
        long_text = "word " * 1000
        embedding = model.embed(long_text)
        assert embedding.shape == (384,)

    def test_embed_unicode_text(self, model: EmbeddingModel):
        """Unicode text should produce valid embeddings."""
        embedding = model.embed("こんにちは世界 🌍")
        assert embedding.shape == (384,)
        assert embedding.dtype == np.float32
