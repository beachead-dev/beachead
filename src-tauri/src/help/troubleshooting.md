# Troubleshooting

## sbx CLI Not Found

**Symptom:** Beachead reports that the `sbx` command is not available.

**Solutions:**
- Download and install sbx from the
  [Docker Sandboxes releases](https://github.com/docker/sbx-releases/releases)
- Ensure the `sbx` binary is in your system PATH
- Restart Beachead after installing sbx
- Run `sbx version` in a terminal to verify the installation

## Docker Authentication Failures

**Symptom:** Login fails or sessions cannot start due to auth errors.

**Solutions:**
- Run `sbx login` in a terminal and complete the authentication flow
- Verify your Docker Hub account has access to Docker Sandboxes
- Check your internet connection (login requires network access)
- If using a proxy, ensure Docker is configured to use it
- Try `sbx logout` followed by `sbx login` to refresh credentials

## Sandbox Creation Errors

**Symptom:** Starting a session fails with sandbox creation errors.

**Solutions:**
- Ensure Docker Desktop or Docker Engine is running
- Run `sbx diagnose` to check system health
- Verify the workspace path exists and is accessible
- Check available disk space (sandboxes require storage)
- Review Docker Desktop resource limits (memory, CPU)
- If using a template, verify it still exists via `sbx template ls`

## Credential Configuration

**Symptom:** Agent fails to authenticate or API calls are rejected.

**Solutions:**
- Go to the **Agents** page and verify credentials are set for the agent
- For API key agents: ensure the key is valid and has not expired
- For OAuth agents: re-authenticate through the credential setup flow
- Run `sbx secret ls` to verify secrets are stored in the keychain
- Check that the correct service name is used (e.g., `anthropic`, `openai`)
- For Kiro agent: complete the device flow (URL + verification code)

## MCP Connection Issues

**Symptom:** Memory MCP server is unreachable or not responding.

**Solutions:**
- Check that Docker is running (MCP containers use Docker, not sbx)
- Verify the MCP container is healthy via Docker Desktop or `docker ps`
- Check the container logs: `docker logs <container-name>`
- Ensure the MCP port is not in use by another process
- Restart Beachead to trigger automatic MCP container restart
- Verify network policy allows access to `host.docker.internal:<port>`

## General Tips

- Run **System Settings > Diagnostics** for a comprehensive health check
- Check the Beachead logs for detailed error messages
- Ensure you are running the latest version of both Beachead and sbx
- Restart Docker Desktop if sandboxes behave unexpectedly
