# Credentials

## Overview

Credentials are stored securely in your OS keychain via `sbx secret`.
Beachead never stores secret values in its database.

## Setting Credentials

- **API Key**: Provide the key value directly
- **OAuth**: Opens a browser for authentication
- **Device Flow**: Displays a URL and code for verification

## Supported Services

Each agent type declares which services it requires. Common services:
- `anthropic` — Anthropic API key
- `openai` — OpenAI API key
- `github` — GitHub token (GH_TOKEN)
- `google` — Google API key

## Security

- Secrets are zeroized from memory after use
- Never logged or stored in SQLite
- Managed exclusively through OS keychain
