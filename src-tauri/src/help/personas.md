# Personas

## Overview

A Persona is a named configuration that combines an agent type, a workspace
path, and optional settings. Personas let you quickly launch pre-configured
agent sessions without re-entering details each time.

## Creating a Persona

1. Navigate to the **Personas** page
2. Click **+ New Persona**
3. Fill in the required fields:
   - **Name** — A unique identifier for this persona
   - **Agent Type** — Select from registered agents (e.g., Claude Code, Codex)
   - **Workspace Path** — Local folder path to mount into the sandbox. You can type a path directly or click **Browse** to open a folder picker.
4. Optionally configure:
   - **Enable Memory** — Toggle long-term memory for this persona
   - **Agent CLI Args** — Additional command-line arguments passed to the agent
   - **MCP Servers** — Additional MCP server entries (name, URL, optional auth headers)
5. Click **Create**

## MCP Server Entries

Each persona can have additional MCP server entries that provide custom tools
to the agent. Each entry requires:

- **Name** — Identifier for the MCP server
- **URL** — Must use `http://` or `https://` scheme with a valid host
- **Description** (optional) — What the server provides
- **Auth Headers** (optional) — JSON object of authentication headers

## Editing a Persona

Click **Edit** on an existing persona to modify its configuration. Changes are
saved to the local database immediately.

If the persona has active sessions:
- **Adding** MCP servers or modifying settings takes effect immediately
- **Removing** MCP servers requires a session restart (you'll see a notification)

## Deleting a Persona

A persona can only be deleted when it has no active sessions. Stop all
sessions associated with the persona before deleting it.

## How Personas Work

When you start a session from a persona, the orchestrator:

1. Generates a mixin kit (spec.yaml + files/) with the persona's configuration
2. Launches a sandbox via `sbx run` with the generated kit applied
3. Opens a terminal connection to the running agent

## Memory Toggle

Each persona can optionally enable long-term memory via the **Enable Memory**
toggle in the persona form.

When memory is enabled:

- The orchestrator provisions a dedicated MCP container for the persona
- A unique bearer token secures communication between the agent and its
  memory server
- The generated Persona Kit includes MCP configuration and network rules
  so the agent can reach its memory server
- The agent can use `memory_store`, `memory_query`, `memory_list`, and
  `memory_delete` tools to persist and retrieve knowledge

When memory is disabled, no MCP container is created and the agent has no
access to long-term memory tools.

## MCP Container Status

If a persona has memory enabled, the Personas page displays the status of
its MCP container:

- **Running** — The memory server is healthy and accepting requests
- **Starting** — The container is being created or restarted
- **Stopped** — The container is not running (will restart on next
  orchestrator launch)
- **Error** — The container failed health checks; check Docker logs for
  details

The orchestrator performs periodic health checks on all MCP containers and
automatically restarts any that fail.

## Tips

- Use descriptive names that reflect the project or task
- Each persona can have its own network policy overrides via kit configuration
- Multiple personas can share the same agent type with different workspaces
