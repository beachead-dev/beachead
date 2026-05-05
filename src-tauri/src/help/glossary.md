# Glossary

## Key Terms

**Persona**
A named configuration combining an agent type, workspace path, and optional
settings (memory, shared memory, additional MCP servers). Personas are the
primary way to organize and launch agent sessions.

**Agent**
An AI coding assistant that runs inside a Docker Sandbox. Built-in agents
include Claude Code, Codex, Copilot, Cursor, Droid, Gemini, Kiro, OpenCode,
Docker Agent, and Shell. Custom agents can be registered via Agent Kits.

**Session**
An active connection between the Beachead UI and a running sandbox. Sessions
provide a terminal interface to interact with the agent and manage file
transfers and port forwarding.

**Sandbox**
A Docker Sandbox microVM providing hypervisor-level isolation. Each sandbox
has its own kernel, Docker daemon, and filesystem. Managed via the `sbx` CLI.

**Kit**
A YAML-based configuration package (spec.yaml + optional files/) applied to
sandboxes at creation time. Kits configure network rules, credentials,
environment variables, install commands, and startup scripts.

**MCP (Model Context Protocol)**
A protocol for providing context to AI agents. In Beachead, MCP servers run
in Docker containers and provide per-persona long-term memory via RAG-based
vector search.

**Template**
A saved snapshot of a configured sandbox. Templates capture installed tools
and configurations so new sandboxes can be created with the same setup.
Managed via `sbx template` commands.

**Policy**
A global network access rule applied to all sandboxes. Policies control which
network destinations sandboxes can reach. Modes: balanced, allow, or deny.
Individual rules can allow or deny specific hosts and ports.

**Workspace**
A local folder on the host mounted into a sandbox at the same absolute path.
The agent can read and write files in the workspace. Multiple workspaces can
be attached to a single persona.
