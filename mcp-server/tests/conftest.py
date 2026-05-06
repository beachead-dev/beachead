"""Shared test fixtures for the MCP server test suite."""

import os

import pytest

# Ensure test environment variables are set
os.environ.setdefault("BEACHEAD_DATA_DIR", "/tmp/beachead_test")
os.environ.setdefault("BEACHEAD_BEARER_TOKEN", "")
os.environ.setdefault("BEACHEAD_PORT", "9199")


@pytest.fixture(scope="session")
def embedding_model():
    """Load the embedding model once for the entire test session.

    This is expensive (~1-2s) so we share it across all tests.
    """
    from beachead_memory_mcp.embeddings import EmbeddingModel

    return EmbeddingModel()
