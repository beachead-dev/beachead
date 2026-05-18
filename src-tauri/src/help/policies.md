# Network Policies

## Overview

Network policies control which hosts sandboxes can reach. Policies can be **global** (apply to all sandboxes) or **per-sandbox** (scoped to a specific sandbox and automatically cleaned up when the sandbox is removed).

## Default Modes

- **balanced** — Default mode, allows common development traffic (AI services, package registries, code hosting, cloud infrastructure)
- **allow** — Permits all network access from sandboxes
- **deny** — Blocks all network access (most restrictive)

## Rules

Add allow or deny rules for specific network targets:
- `allow 127.0.0.1:8080` — Allow access to a local service
- `deny *.evil.com` — Block a domain and all subdomains
- `allow api.openai.com:443` — Allow API access on a specific port

Rules added via the UI are global. Per-sandbox rules (e.g., MCP port access) are managed automatically by the session lifecycle.

## Rule Types

- **local:** — Custom rules you add (removable)
- **kit:** — Rules injected by sandbox kits (removable by resource)
- **default-** — Built-in rules from the default policy (not removable individually; use "Reset" to clear all)

## Sorting and Filtering

- Click column headers (Rule, Action, Target) to sort alphabetically
- Use the search box to filter rules by any field
- Use the Refresh button to reload the current state
- Enable Auto-refresh for periodic updates (30 seconds)

## Policy Log

View the traffic log to see which requests were allowed or denied, helping you fine-tune your policy rules. Filter by sandbox name to focus on specific sessions.
