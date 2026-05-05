# Sessions

## Overview

A Session is an active connection between the Beachead UI and a running
Docker Sandbox. Each session provides a terminal interface to interact
with the AI agent running inside the sandbox.

## Starting a Session

1. Go to the **Personas** page
2. Click **Start Session** on the desired persona
3. The orchestrator creates a sandbox and opens a terminal

Alternatively, use the **Sessions** page to view and manage all active sessions.

## Terminal Interface

The session terminal uses xterm.js to provide a full terminal emulator:

- Standard keyboard shortcuts work (Ctrl+C, Ctrl+D, etc.)
- Clickable URLs are detected automatically
- The terminal resizes to fit the available space

## File Upload

Upload files to the running sandbox using drag-and-drop or the file
upload button. Files are copied into the sandbox workspace via `sbx cp`.

## Port Management

View forwarded ports from the sandbox. Sandboxes can expose services
on specific ports, accessible from the host via `sbx ports`.

## Stopping a Session

Click **Stop** on an active session to terminate the sandbox. This runs
`sbx stop` followed by `sbx rm` to clean up resources.

## Session States

- **running** — Sandbox is active and terminal is connected
- **stopped** — Sandbox has been stopped
- **error** — Sandbox encountered a failure during creation or execution

## Troubleshooting

If a session fails to start, check:
- The agent's credentials are configured correctly
- Docker is running and `sbx` CLI is available
- The workspace path exists and is accessible
