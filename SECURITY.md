# Security Policy

## Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

Use GitHub's [Private Vulnerability Reporting](https://github.com/beachead-dev/beachead/security/advisories/new) to submit a report directly to the maintainers. This keeps the details confidential until a fix is available.

If you are unsure whether something is a security issue, report it privately and we will assess it.

---

## Response Time

We aim to acknowledge reports within **72 hours** and provide an initial assessment within **7 days**. Critical issues affecting credential handling or sandbox isolation will be prioritized.

---

## Scope

### In Scope

- **Credential handling** — API keys and OAuth tokens stored via `sbx secret` / OS keychain
- **Sandbox escape** — any path by which agent code could break out of the Docker Sandbox microVM
- **Authentication bypass** — bypassing the MCP server bearer token check or any other auth control
- **Path traversal** — accessing files outside the declared workspace mounts
- **Injection attacks** — command injection, SQL injection, or similar in any input path
- **Secrets leakage** — credentials appearing in logs, error messages, or network traffic in cleartext

### Out of Scope

- Vulnerabilities in Docker Engine, Docker Sandboxes (sbx), or the agent tools themselves (Claude Code, Codex, etc.) — report those to their respective projects
- Issues that require physical access to the machine
- Denial of service against a local desktop application
- Social engineering

---

## Note on Scope

Beachead manages AI agent credentials and controls which network domains agents can reach. Security issues in this project can have direct impact on the credentials and code of anyone using it. We take security reports seriously and will respond promptly.
