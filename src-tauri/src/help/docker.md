# Docker Management

The Docker page provides visibility and control over two categories of Docker resources used by Beachead: **Sandboxes** and **MCP Memory Containers**.

## Accessing the Docker Page

Click **Docker** in the sidebar navigation to open the Docker management page. The page has two tabs: Sandboxes and Containers.

## Sandboxes Tab

Sandboxes are isolated Docker environments managed by the `sbx` CLI, used to run AI agent sessions.

### Viewing Sandboxes

By default, only sandboxes associated with a Beachead session are shown (managed sandboxes). Enable the **Show All** toggle to see all sandboxes on the system, including those not managed by Beachead.

The table displays:
- **Name** — The sandbox name (if assigned)
- **Agent** — The AI agent running in the sandbox (e.g., kiro, claude, codex)
- **Status** — Current state (running, stopped, etc.)
- **ID** — The unique sandbox identifier

### Actions

- **Start** — Creates a new sandbox instance using the same agent and workspace configuration. Available when a sandbox is stopped.
- **Stop** — Stops a running sandbox. Available when a sandbox is running.
- **Remove** — Permanently deletes a sandbox. A confirmation dialog is shown before removal. Available when a sandbox is stopped.

Buttons are disabled when an action is not applicable to the current sandbox state.

## Containers Tab

The Containers tab shows Docker containers managed by Beachead. When **Show All** is enabled, it displays all Docker containers on the system.

### Viewing Containers

By default, only containers tracked in the Beachead database are shown (MCP memory containers). Enable the **Show All** toggle to see all Docker containers on the system. Containers not tracked by Beachead display an "Unmanaged" badge.

The table displays:
- **Persona Name** — The persona associated with this container (or the Docker container name for unmanaged containers)
- **Image** — The Docker image the container is running (e.g., beachead-memory-mcp:latest, hello-world, nginx)
- **Port** — The localhost port the container is listening on
- **Status** — Current Docker state (running, exited, stopped, created, etc.)
- **Volume Name** — The Docker volume used for persistent storage (if any)
- **Created Date** — When the container was created

### Actions

- **Start** — Starts a stopped container. Available when status is stopped, exited, or created.
- **Stop** — Stops a running container with a 10-second timeout. Available when status is running.
- **Remove** — Permanently removes the container. For managed containers, a confirmation dialog offers the option to also delete the associated Docker volume. Available when status is stopped, exited, or created.

For unmanaged containers, only Stop and Remove are available (no Start).

## Data Freshness

The Docker page automatically polls for updates every 10 seconds while a tab is visible. After performing an action, data refreshes immediately. Toggling "Show All" also triggers an immediate refresh. If a poll fails, the last successful data is retained with a "stale" indicator until the next successful refresh.

## Error Handling

- If an action fails, an error message is displayed with the failure reason. The message can be dismissed.
- If the `sbx` CLI is unavailable, sandbox operations return a service unavailable error.
- If Docker is unreachable, container status falls back to the database-recorded state.
