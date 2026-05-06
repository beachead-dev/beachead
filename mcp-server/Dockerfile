FROM python:3.11-slim

# Set environment variables
ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    BEACHEAD_DATA_DIR=/data/memory \
    BEACHEAD_PORT=9100 \
    BEACHEAD_HOST=0.0.0.0

# Install system dependencies for FAISS and sentence-transformers
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        build-essential \
        && rm -rf /var/lib/apt/lists/*

# Create non-root user for security
RUN useradd --create-home --shell /bin/bash mcpuser

# Set working directory
WORKDIR /app

# Copy project files
COPY pyproject.toml .
COPY src/ src/

# Install the package and dependencies
RUN pip install --no-cache-dir -e .

# Pre-download the embedding model so it's baked into the image
RUN python -c "from sentence_transformers import SentenceTransformer; SentenceTransformer('all-MiniLM-L6-v2')"

# Create data directory with correct permissions
RUN mkdir -p /data/memory && chown -R mcpuser:mcpuser /data

# Switch to non-root user
USER mcpuser

# Volume for persistent vector storage data
VOLUME ["/data/memory"]

# Expose the MCP server port
EXPOSE 9100

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=60s --retries=3 \
    CMD python -c "import httpx; r = httpx.get('http://localhost:9100/health'); r.raise_for_status()"

# Run the MCP server
ENTRYPOINT ["python", "-m", "beachead_memory_mcp.server"]
