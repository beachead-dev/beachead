# Beachead — Secure AI Orchestrator

Beachead is a local-only desktop application that manages AI agents running inside Docker Sandbox microVMs. It provides a unified interface for creating personas, launching sandboxed agent sessions, managing network policies, and attaching long-term memory to each persona.

## Prerequisites

Before using Beachead, install the following:

| Dependency | Purpose | Install Guide |
|-----------|---------|---------------|
| **Docker Engine** | Container runtime for sandboxes and memory MCP containers | [docs.docker.com/engine/install](https://docs.docker.com/engine/install/) |
| **Docker Sandboxes (sbx)** | CLI for managing sandbox microVMs | [github.com/docker/sbx-releases](https://github.com/docker/sbx-releases/releases) |
| **Git** | Required for Repo Sync features (remote synchronization) | [git-scm.com/downloads](https://git-scm.com/downloads) |

> **Note:** Docker Engine must be running for both sandbox sessions and per-persona memory features. Memory MCP containers are managed automatically via the Docker API (bollard crate).

> **Note:** Git is optional but required for Repo Sync functionality. If git is not found on your PATH, Repo Sync features will be disabled (graceful degradation).

### Platform-Specific Notes

#### Linux — Keyring Support

Repo Sync stores git credentials in the OS keyring. On Linux, this requires `libsecret` (used by GNOME Keyring / KDE Wallet):

**Ubuntu/Debian:**
```bash
sudo apt install libsecret-1-dev gnome-keyring
```

**Fedora/RHEL:**
```bash
sudo dnf install libsecret-devel gnome-keyring
```

**Arch Linux:**
```bash
sudo pacman -S libsecret gnome-keyring
```

If the keyring service is not available, credential storage for Repo Sync will fail. The rest of Beachead will continue to work normally.

### Installing sbx

**macOS:**
```bash
brew install docker/tap/sbx
```

**Windows:**
```powershell
winget install Docker.sbx
```

**Linux (Ubuntu/Debian):**
Download the latest binary from [sbx-releases](https://github.com/docker/sbx-releases/releases) and place it on your PATH:
```bash
sudo mv sbx /usr/local/bin/sbx
sudo chmod +x /usr/local/bin/sbx
```

### Sign in to Docker

After installing sbx, authenticate with Docker:
```bash
sbx login
```
This opens a browser for Docker OAuth. Choose the "Balanced" network policy when prompted.

## Quick Start

### 1. Launch Beachead

Open the Beachead application. On first launch it will verify that `sbx` and `docker` are available.

If either dependency is missing, the app will display setup instructions in the System Settings page.

### 2. Create a Persona

1. Navigate to **Personas** in the sidebar.
2. Click **Create Persona**.
3. Fill in:
   - **Name** — a unique identifier (e.g., "my-project-claude")
   - **Agent Type** — select from built-in agents (Claude Code, Codex, Copilot, etc.)
   - **Workspace** — path to a local project folder
4. Optionally enable **Memory** for long-term context retention.
5. Optionally add **MCP Servers** for custom tool integrations.
6. Click **Save**.

### 3. Configure Agent Credentials

Before starting a session, ensure the agent's credentials are configured:

1. Navigate to **Agents** in the sidebar.
2. Select the agent type your persona uses.
3. In the **Credentials** section, set the required API key or initiate OAuth.

Credentials are stored securely in your OS keychain via `sbx secret` — they never touch the application database.

### 4. Start a Session

1. Navigate to **Sessions** in the sidebar.
2. Click **New Session** and select your persona.
3. A terminal tab opens with the agent running inside an isolated sandbox.
4. Interact with the agent directly in the terminal.

The sandbox mounts your workspace at the same path, so the agent can read and modify your project files.

### 5. Manage Network Policies

1. Navigate to **Policies** in the sidebar.
2. View and modify global network access rules.
3. Check the **Traffic Log** to see what network requests sandboxes are making.

### 6. Per-Persona Memory

Each persona can have long-term memory enabled, backed by a local MCP server running in a Docker container.

1. When creating or editing a persona, toggle **Memory** on.
2. The orchestrator automatically manages a dedicated memory container for that persona.
3. Memory persists across sessions — the agent retains context from previous conversations.
4. Memory data is stored in Docker volumes on your local machine.

**Requirements:** Docker Engine must be running for memory features. The memory MCP container starts automatically when Beachead launches.

### 7. Export and Import Memory

You can export a persona's memory for backup or transfer, and import memory from a previous export.

1. Navigate to the persona's detail view.
2. Click **Export Memory** to download the memory data as a file.
3. To restore or transfer memory, click **Import Memory** and select a previously exported file.

Exported memory files contain the persona's vector store and metadata. They can be imported into the same or a different Beachead installation.

## Key Concepts

| Term | Description |
|------|-------------|
| **Persona** | A saved configuration combining an agent, workspace, and settings |
| **Agent** | An AI tool (Claude Code, Codex, etc.) that runs inside a sandbox |
| **Session** | An active terminal connection to a running sandbox |
| **Sandbox** | A Docker Sandbox microVM with hypervisor-level isolation |
| **Kit** | A YAML config package applied to sandboxes at creation time |
| **MCP** | Model Context Protocol — enables agents to use external tools |
| **Memory** | Per-persona long-term context stored in a local MCP container |
| **MCP Container** | A Docker container hosting a memory MCP server (managed via bollard) |
| **Template** | A saved snapshot of a configured sandbox for reuse |
| **Policy** | A global network access rule applied to all sandboxes |

## Troubleshooting

### sbx CLI not found

Ensure `sbx` is on your system PATH:
```bash
sbx version
```
If not found, reinstall following the instructions above.

### Docker authentication failed

Sign in to Docker:
```bash
sbx login
```

### Sandbox creation errors

Check that Docker is running:
```bash
docker info
```

Run diagnostics:
```bash
sbx diagnose
```

### Credential issues

List configured secrets:
```bash
sbx secret ls
```

Set a missing credential:
```bash
sbx secret set -g <service> -t <api-key>
```

## Architecture

Beachead runs as a Tauri 2.0 desktop app with:
- **Rust backend** (Axum HTTP/WebSocket server) — manages all sandbox operations
- **React frontend** — provides the UI with xterm.js terminals
- **SQLite database** — persists configuration locally (no secrets)

All sandbox operations go through the official `sbx` CLI. No undocumented APIs are used.

## License

MIT
