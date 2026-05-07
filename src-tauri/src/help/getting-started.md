# Getting Started with Beachead

## Prerequisites

1. **Docker Desktop** or **Docker Engine** installed and running
2. **sbx CLI** installed from [Docker Sandboxes](https://docs.docker.com/ai/sandboxes/get-started/)
3. Logged in via `sbx login`

## Quick Start

1. Create a persona with an agent type and workspace path
2. Configure any required credentials for the agent
3. Start a session to launch the agent in a sandbox
4. Use the terminal to interact with the agent

## Key Concepts

- **Personas**: Named configurations combining an agent, workspace, and settings
- **Sessions**: Running instances of a persona in a Docker Sandbox
- **Agents**: AI coding assistants (Claude, Codex, Copilot, etc.)
- **Credentials**: API keys and OAuth tokens stored securely in OS keychain

## Memory-Enabled Personas

Personas can optionally enable long-term memory. When memory is enabled:

1. The orchestrator creates a dedicated MCP container running a RAG-based
   memory server for that persona.
2. A bearer token is generated to secure communication between the agent
   and its memory server.
3. The generated Persona Kit automatically configures the agent to connect
   to its memory MCP server via `host.docker.internal:<port>`.
4. The agent gains access to memory tools: `memory_store`, `memory_query`,
   `memory_list`, and `memory_delete`.

To enable memory on a persona, toggle **Enable Memory** in the persona
creation or edit form. The MCP container starts automatically when the
orchestrator launches and persists data across restarts via Docker volumes.

## Export and Import

You can export your entire orchestrator configuration to an encrypted file
for backup or migration to another machine.

### Exporting

1. Navigate to the **Export/Import** section in the UI
2. Click **Export**
3. Enter a password to encrypt the export file
4. Save the generated file

The export includes persona configurations, agent registrations, network
policy rules, MCP server entries, and container port mappings. Secret values
(API keys, tokens) are **not** included — only metadata about which services
were configured.

### Importing

1. Navigate to the **Export/Import** section in the UI
2. Click **Import** and select an export file
3. Enter the password used during export
4. Review the configuration preview
5. Resolve any naming conflicts (rename, skip, or overwrite)
6. After import, configure any required secrets on the Agents page

Personas with missing required secrets are flagged with a warning indicator
until the credentials are configured.
