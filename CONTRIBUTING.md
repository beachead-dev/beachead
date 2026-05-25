# Contributing to Beachead

Thank you for your interest in contributing. Please read this document before opening an issue or pull request.

---

## Prerequisites

You will need the following installed before building Beachead from source:

| Tool | Purpose | Install |
|------|---------|---------|
| **Rust toolchain** (stable) | Backend build | [rustup.rs](https://rustup.rs) |
| **Node.js 20+** | Frontend build | [nodejs.org](https://nodejs.org) |
| **Docker Engine** | Sandbox runtime | [docs.docker.com/engine/install](https://docs.docker.com/engine/install/) |
| **Docker Sandboxes (sbx)** | Sandbox CLI | [github.com/docker/sbx-releases](https://github.com/docker/sbx-releases/releases) |
| **uv** | Python package manager (MCP server) | [docs.astral.sh/uv](https://docs.astral.sh/uv/) |

---

## Building from Source

```bash
# Clone the repo
git clone https://github.com/beachead-dev/beachead.git
cd beachead

# Install frontend dependencies
npm install

# Run in development mode (Tauri + Vite dev server)
npm run tauri dev

# Build a release binary
npm run tauri build
```

The MCP server is a separate Python project:

```bash
cd mcp-server
uv sync
uv run beachead-memory-mcp
```

---

## Running Tests

```bash
# Rust tests
cargo test

# Frontend tests
npm test

# Python MCP server tests
cd mcp-server && uv run pytest

# Lint
cargo fmt --check
cargo clippy -- -D warnings
cd mcp-server && uv run ruff check src/
```

All of these must pass before a PR can be merged.

---

## Opening Issues

For bugs and feature requests, please use the issue templates provided. Before opening a new issue, search existing issues to avoid duplicates.

For security vulnerabilities, do **not** open a public issue — see [SECURITY.md](SECURITY.md).

---

## Pull Request Policy

1. **Open an issue first** for any non-trivial change so the approach can be discussed before you invest time in an implementation.
2. Keep PRs focused. One logical change per PR.
3. All tests must pass (`cargo test`, `npm test`, `uv run pytest`).
4. Run `cargo fmt` and `cargo clippy -- -D warnings` before pushing.
5. Update in-app help content if user-facing behavior changes.
6. Update `.kiro/specs/code-reference.md` if files, public APIs, or data flows change.

---

## Code Style

- **Rust:** `cargo fmt` (project `rustfmt.toml` is in `src-tauri/`), `cargo clippy`
- **TypeScript/React:** standard TypeScript, no additional linter configured beyond `tsc`
- **Python:** `ruff check` with project config in `mcp-server/pyproject.toml`

---

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
