# Network Policies

## Default Modes

- **balanced** — Default mode, allows common development traffic
- **allow** — Permits all network access from sandboxes
- **deny** — Blocks all network access (most restrictive)

## Rules

Add allow or deny rules for specific network targets:
- `allow 127.0.0.1:8080` — Allow access to a local service
- `deny *.evil.com` — Block a domain
- `allow api.openai.com` — Allow API access

## Policy Log

View the traffic log to see which requests were allowed or denied,
helping you fine-tune your policy rules.
