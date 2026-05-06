"""Embedding model wrapper using sentence-transformers.

Uses all-MiniLM-L6-v2 for local embedding generation.
No internet connection required after initial model download.
"""

from __future__ import annotations

import numpy as np
from sentence_transformers import SentenceTransformer

# Default model - produces 384-dimensional embeddings
DEFAULT_MODEL_NAME = "all-MiniLM-L6-v2"


class EmbeddingModel:
    """Wrapper around sentence-transformers for generating text embeddings."""

    def __init__(self, model_name: str = DEFAULT_MODEL_NAME) -> None:
        self._model = SentenceTransformer(model_name)
        self._dimension = self._model.get_embedding_dimension()

    @property
    def dimension(self) -> int:
        """Return the dimensionality of embeddings produced by this model."""
        return self._dimension

    def embed(self, text: str) -> np.ndarray:
        """Generate an embedding vector for a single text string.

        Args:
            text: The input text to embed.

        Returns:
            A 1-D numpy array of shape (dimension,) with float32 values.
        """
        embedding = self._model.encode(text, convert_to_numpy=True)
        return embedding.astype(np.float32).flatten()

    def embed_batch(self, texts: list[str]) -> np.ndarray:
        """Generate embedding vectors for a batch of text strings.

        Args:
            texts: List of input texts to embed.

        Returns:
            A 2-D numpy array of shape (len(texts), dimension) with float32 values.
        """
        embeddings = self._model.encode(texts, convert_to_numpy=True)
        return embeddings.astype(np.float32)
