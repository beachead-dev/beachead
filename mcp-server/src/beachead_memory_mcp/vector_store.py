"""FAISS-based vector store with SQLite metadata persistence.

Stores embeddings in a FAISS index for fast similarity search,
and metadata (text, timestamps, IDs) in a SQLite database.
Data persists across server restarts via filesystem storage.
"""

from __future__ import annotations

import json
import sqlite3
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

import faiss
import numpy as np


@dataclass
class MemoryEntry:
    """A single memory entry with text, embedding, and metadata."""

    id: str
    text: str
    metadata: dict
    created_at: str


@dataclass
class SearchResult:
    """A search result with similarity score."""

    entry: MemoryEntry
    score: float


class VectorStore:
    """FAISS vector store with SQLite metadata backend.

    Stores vectors in a FAISS IndexFlatIP (inner product / cosine similarity
    on normalized vectors) and metadata in SQLite for persistence.
    """

    def __init__(self, data_dir: Path, dimension: int) -> None:
        """Initialize the vector store.

        Args:
            data_dir: Directory for persistent storage (FAISS index + SQLite DB).
            dimension: Dimensionality of embedding vectors.
        """
        self._data_dir = data_dir
        self._dimension = dimension
        self._data_dir.mkdir(parents=True, exist_ok=True)

        self._db_path = self._data_dir / "metadata.db"
        self._index_path = self._data_dir / "vectors.index"

        self._db = self._init_db()
        self._index = self._load_or_create_index()

    @property
    def dimension(self) -> int:
        """Return the dimensionality of stored vectors."""
        return self._dimension

    def _init_db(self) -> sqlite3.Connection:
        """Initialize SQLite database for metadata storage."""
        conn = sqlite3.connect(str(self._db_path), check_same_thread=False)
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute("""
            CREATE TABLE IF NOT EXISTS entries (
                id TEXT PRIMARY KEY,
                text TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}',
                faiss_idx INTEGER NOT NULL,
                created_at TEXT NOT NULL
            )
        """)
        conn.execute("""
            CREATE INDEX IF NOT EXISTS idx_entries_faiss_idx
            ON entries(faiss_idx)
        """)
        conn.commit()
        return conn

    def _load_or_create_index(self) -> faiss.IndexFlatIP:
        """Load existing FAISS index or create a new one."""
        if self._index_path.exists():
            return faiss.read_index(str(self._index_path))
        return faiss.IndexFlatIP(self._dimension)

    def _save_index(self) -> None:
        """Persist the FAISS index to disk."""
        faiss.write_index(self._index, str(self._index_path))

    def _normalize(self, vector: np.ndarray) -> np.ndarray:
        """L2-normalize a vector for cosine similarity via inner product."""
        norm = np.linalg.norm(vector)
        if norm == 0:
            return vector
        return vector / norm

    def insert(self, text: str, embedding: np.ndarray, metadata: dict | None = None) -> str:
        """Insert a new memory entry.

        Args:
            text: The text content to store.
            embedding: The embedding vector (must match store dimension).
            metadata: Optional metadata dictionary.

        Returns:
            The generated entry ID.

        Raises:
            ValueError: If embedding dimension doesn't match store dimension.
        """
        if embedding.shape[0] != self._dimension:
            raise ValueError(
                f"Embedding dimension {embedding.shape[0]} doesn't match "
                f"store dimension {self._dimension}"
            )

        entry_id = str(uuid.uuid4())
        created_at = datetime.now(timezone.utc).isoformat()
        meta = metadata or {}

        # Normalize for cosine similarity
        normalized = self._normalize(embedding.astype(np.float32)).reshape(1, -1)

        # Add to FAISS index
        faiss_idx = self._index.ntotal
        self._index.add(normalized)

        # Persist metadata
        self._db.execute(
            "INSERT INTO entries (id, text, metadata, faiss_idx, created_at) VALUES (?, ?, ?, ?, ?)",
            (entry_id, text, json.dumps(meta), faiss_idx, created_at),
        )
        self._db.commit()
        self._save_index()

        return entry_id

    def search(self, query_embedding: np.ndarray, top_k: int = 5) -> list[SearchResult]:
        """Search for similar entries by embedding.

        Args:
            query_embedding: The query vector.
            top_k: Maximum number of results to return.

        Returns:
            List of SearchResult ordered by descending similarity.
        """
        if self._index.ntotal == 0:
            return []

        # Clamp top_k to available entries
        k = min(top_k, self._index.ntotal)

        normalized = self._normalize(query_embedding.astype(np.float32)).reshape(1, -1)
        scores, indices = self._index.search(normalized, k)

        results = []
        for score, idx in zip(scores[0], indices[0]):
            if idx == -1:
                continue
            row = self._db.execute(
                "SELECT id, text, metadata, created_at FROM entries WHERE faiss_idx = ?",
                (int(idx),),
            ).fetchone()
            if row:
                entry = MemoryEntry(
                    id=row[0],
                    text=row[1],
                    metadata=json.loads(row[2]),
                    created_at=row[3],
                )
                results.append(SearchResult(entry=entry, score=float(score)))

        return results

    def list_all(self) -> list[MemoryEntry]:
        """Return all stored memory entries.

        Returns:
            List of all MemoryEntry objects, ordered by creation time.
        """
        rows = self._db.execute(
            "SELECT id, text, metadata, created_at FROM entries ORDER BY created_at DESC"
        ).fetchall()
        return [
            MemoryEntry(
                id=row[0],
                text=row[1],
                metadata=json.loads(row[2]),
                created_at=row[3],
            )
            for row in rows
        ]

    def get(self, entry_id: str) -> MemoryEntry | None:
        """Get a single entry by ID.

        Args:
            entry_id: The entry ID to look up.

        Returns:
            The MemoryEntry if found, None otherwise.
        """
        row = self._db.execute(
            "SELECT id, text, metadata, created_at FROM entries WHERE id = ?",
            (entry_id,),
        ).fetchone()
        if row is None:
            return None
        return MemoryEntry(
            id=row[0],
            text=row[1],
            metadata=json.loads(row[2]),
            created_at=row[3],
        )

    def delete(self, entry_id: str) -> bool:
        """Delete an entry by ID.

        Note: FAISS does not support efficient single-vector deletion.
        We mark the entry as deleted in SQLite and rebuild the index
        periodically. For now, we rebuild immediately on delete.

        Args:
            entry_id: The entry ID to delete.

        Returns:
            True if the entry was found and deleted, False otherwise.
        """
        row = self._db.execute(
            "SELECT faiss_idx FROM entries WHERE id = ?", (entry_id,)
        ).fetchone()
        if row is None:
            return False

        # Remove from SQLite
        self._db.execute("DELETE FROM entries WHERE id = ?", (entry_id,))
        self._db.commit()

        # Rebuild FAISS index from remaining entries
        self._rebuild_index()
        return True

    def _rebuild_index(self) -> None:
        """Rebuild the FAISS index from all remaining entries.

        This is necessary after deletions since FAISS IndexFlatIP
        doesn't support efficient single-vector removal.
        """
        rows = self._db.execute(
            "SELECT id, faiss_idx FROM entries ORDER BY faiss_idx"
        ).fetchall()

        if not rows:
            # No entries left, create empty index
            self._index = faiss.IndexFlatIP(self._dimension)
            self._save_index()
            return

        # Build new index with only remaining entries using reconstruct
        old_index = self._index
        new_index = faiss.IndexFlatIP(self._dimension)

        for new_idx, (entry_id, old_faiss_idx) in enumerate(rows):
            vector = old_index.reconstruct(int(old_faiss_idx)).reshape(1, -1)
            new_index.add(vector)
            # Update the faiss_idx in SQLite
            self._db.execute(
                "UPDATE entries SET faiss_idx = ? WHERE id = ?",
                (new_idx, entry_id),
            )

        self._db.commit()
        self._index = new_index
        self._save_index()

    @property
    def count(self) -> int:
        """Return the number of stored entries."""
        row = self._db.execute("SELECT COUNT(*) FROM entries").fetchone()
        return row[0] if row else 0

    def close(self) -> None:
        """Close the database connection."""
        self._db.close()
