"""Beachead Memory MCP Server.

Implements the MCP protocol with HTTP/SSE transport, providing:
- memory_store: Store text with embeddings for later retrieval
- memory_query: Semantic search over stored memories
- memory_list: List all stored memory entries
- memory_delete: Remove a memory entry by ID

Uses sentence-transformers (all-MiniLM-L6-v2) for local embeddings
and FAISS for vector similarity search.
"""

from __future__ import annotations

import hmac
import logging
import os
from pathlib import Path

from mcp.server.fastmcp import FastMCP

from beachead_memory_mcp.embeddings import EmbeddingModel
from beachead_memory_mcp.vector_store import VectorStore

logger = logging.getLogger(__name__)

# Configuration from environment
DATA_DIR = Path(os.environ.get("BEACHEAD_DATA_DIR", "/data/memory"))
BEARER_TOKEN = os.environ.get("BEACHEAD_BEARER_TOKEN", "")
PORT = int(os.environ.get("BEACHEAD_PORT", "9100"))
HOST = os.environ.get("BEACHEAD_HOST", "0.0.0.0")

# Initialize the MCP server
mcp = FastMCP(
    "beachead-memory",
    instructions="Long-term memory for Beachead personas via RAG",
)

# Lazy-initialized globals
_embedding_model: EmbeddingModel | None = None
_vector_store: VectorStore | None = None


def _get_embedding_model() -> EmbeddingModel:
    """Get or initialize the embedding model (lazy loading)."""
    global _embedding_model
    if _embedding_model is None:
        logger.info("Loading embedding model...")
        _embedding_model = EmbeddingModel()
        logger.info(
            "Embedding model loaded (dimension=%d)", _embedding_model.dimension
        )
    return _embedding_model


def _get_vector_store() -> VectorStore:
    """Get or initialize the vector store (lazy loading)."""
    global _vector_store
    if _vector_store is None:
        model = _get_embedding_model()
        _vector_store = VectorStore(data_dir=DATA_DIR, dimension=model.dimension)
        logger.info("Vector store initialized at %s", DATA_DIR)
    return _vector_store


def _validate_auth(token: str | None) -> bool:
    """Validate bearer token using constant-time comparison.

    Args:
        token: The token to validate (None if not provided).

    Returns:
        True if valid, False otherwise.
    """
    if not BEARER_TOKEN:
        # No token configured = auth disabled (development mode)
        return True
    if token is None:
        return False
    return hmac.compare_digest(token, BEARER_TOKEN)


@mcp.tool()
def memory_store(text: str, metadata: dict | None = None) -> dict:
    """Store a piece of knowledge in long-term memory.

    Use this to save important context, decisions, learnings, and facts
    that should be remembered across sessions.

    Args:
        text: The text content to store. Should be a meaningful piece of
              knowledge, decision, or context worth remembering.
        metadata: Optional metadata dictionary (e.g., {"source": "user", "topic": "architecture"}).

    Returns:
        Dictionary with the stored entry's ID and confirmation.
    """
    if not text or not text.strip():
        return {"error": "Text cannot be empty"}

    model = _get_embedding_model()
    store = _get_vector_store()

    embedding = model.embed(text.strip())
    entry_id = store.insert(text.strip(), embedding, metadata)

    return {
        "id": entry_id,
        "status": "stored",
        "text_length": len(text.strip()),
    }


@mcp.tool()
def memory_query(query: str, top_k: int = 5) -> dict:
    """Search long-term memory for relevant knowledge.

    Use this to retrieve past knowledge, decisions, and context before
    starting work or when you need to recall something.

    Args:
        query: The search query. Describe what you're looking for in
               natural language.
        top_k: Maximum number of results to return (default: 5).

    Returns:
        Dictionary with ranked search results including text, score, and metadata.
    """
    if not query or not query.strip():
        return {"error": "Query cannot be empty", "results": []}

    if top_k < 1:
        top_k = 1
    elif top_k > 100:
        top_k = 100

    model = _get_embedding_model()
    store = _get_vector_store()

    query_embedding = model.embed(query.strip())
    results = store.search(query_embedding, top_k=top_k)

    return {
        "results": [
            {
                "id": r.entry.id,
                "text": r.entry.text,
                "score": round(r.score, 4),
                "metadata": r.entry.metadata,
                "created_at": r.entry.created_at,
            }
            for r in results
        ],
        "total_results": len(results),
        "query": query.strip(),
    }


@mcp.tool()
def memory_list(limit: int = 50, offset: int = 0) -> dict:
    """List all stored memory entries.

    Use this to see what's currently stored in long-term memory.

    Args:
        limit: Maximum number of entries to return (default: 50).
        offset: Number of entries to skip (for pagination).

    Returns:
        Dictionary with all memory entries and total count.
    """
    if limit < 1:
        limit = 1
    elif limit > 500:
        limit = 500
    if offset < 0:
        offset = 0

    store = _get_vector_store()
    all_entries = store.list_all()

    # Apply pagination
    paginated = all_entries[offset : offset + limit]

    return {
        "entries": [
            {
                "id": e.id,
                "text": e.text,
                "metadata": e.metadata,
                "created_at": e.created_at,
            }
            for e in paginated
        ],
        "total_count": len(all_entries),
        "limit": limit,
        "offset": offset,
    }


@mcp.tool()
def memory_delete(entry_id: str) -> dict:
    """Remove a memory entry by ID.

    Use this to remove outdated, incorrect, or no longer relevant entries.

    Args:
        entry_id: The ID of the memory entry to delete.

    Returns:
        Dictionary confirming deletion or indicating entry not found.
    """
    if not entry_id or not entry_id.strip():
        return {"error": "Entry ID cannot be empty"}

    store = _get_vector_store()
    deleted = store.delete(entry_id.strip())

    if deleted:
        return {"id": entry_id.strip(), "status": "deleted"}
    else:
        return {"id": entry_id.strip(), "status": "not_found"}


def create_app():
    """Create the Starlette ASGI application with auth middleware.

    Returns the MCP server wrapped with bearer token authentication
    and a health check endpoint.
    """
    from starlette.applications import Starlette
    from starlette.responses import JSONResponse
    from starlette.routing import Route

    from beachead_memory_mcp.auth import BearerTokenMiddleware

    async def health_check(request):
        """Health check endpoint - returns server status."""
        store = _get_vector_store()
        return JSONResponse({
            "status": "healthy",
            "entries_count": store.count,
            "model_dimension": _get_embedding_model().dimension,
        })

    # Create a Starlette app with health check
    app = Starlette(
        routes=[Route("/health", health_check, methods=["GET"])],
    )

    # Add bearer token auth middleware if token is configured
    if BEARER_TOKEN:
        app.add_middleware(BearerTokenMiddleware, expected_token=BEARER_TOKEN)

    return app


def main():
    """Run the MCP server with HTTP/SSE transport."""
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    )

    logger.info("Starting Beachead Memory MCP Server on %s:%d", HOST, PORT)
    logger.info("Data directory: %s", DATA_DIR)
    logger.info("Auth: %s", "enabled" if BEARER_TOKEN else "disabled (no token configured)")

    # Run the MCP server with SSE transport
    mcp.run(transport="sse")


if __name__ == "__main__":
    main()
