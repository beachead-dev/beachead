# Personas

## Overview

A Persona is a named configuration that combines an agent type, a workspace
path, and optional settings. Personas let you quickly launch pre-configured
agent sessions without re-entering details each time.

## Creating a Persona

1. Navigate to the **Personas** page
2. Click **Create Persona**
3. Fill in the required fields:
   - **Name** — A unique identifier for this persona
   - **Agent** — Select from registered agents (e.g., Claude Code, Codex)
   - **Workspace** — Local folder path to mount into the sandbox
4. Optionally configure memory settings and shared memory references
5. Click **Save**

## Editing a Persona

Click on an existing persona to edit its configuration. Changes are saved
to the local database immediately.

## Deleting a Persona

A persona can only be deleted when it has no active sessions. Stop all
sessions associated with the persona before deleting it.

## How Personas Work

When you start a session from a persona, the orchestrator:

1. Generates a mixin kit (spec.yaml + files/) with the persona's configuration
2. Launches a sandbox via `sbx run` with the generated kit applied
3. Opens a terminal connection to the running agent

## Tips

- Use descriptive names that reflect the project or task
- Each persona can have its own network policy overrides via kit configuration
- Multiple personas can share the same agent type with different workspaces
