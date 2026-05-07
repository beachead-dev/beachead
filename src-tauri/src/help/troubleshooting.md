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

## MCP Container Failures

**Symptom:** MCP container shows "Error" status or fails to start.

**Solutions:**
- Check Docker daemon is running: `docker info`
- Inspect container logs: `docker logs beachead-mcp-<persona-name>`
- Verify sufficient disk space for Docker volumes
- Check that the allocated port is not blocked by a firewall or another
  process: `lsof -i :<port>` (macOS/Linux) or `netstat -ano | findstr :<port>` (Windows)
- Remove and recreate the container by disabling and re-enabling memory
  on the persona
- If the container repeatedly fails health checks, check Docker resource
  limits (memory, CPU) in Docker Desktop settings

## Port Allocation Failures

**Symptom:** Error message indicating port exhaustion when enabling memory
on a persona.

**Solutions:**
- Check how many MCP containers are running: `docker ps --filter name=beachead-mcp`
- Release ports by disabling memory on personas that no longer need it
- Verify the configured port range has available ports
- Check for external processes occupying ports in the MCP range
- Restart Beachead to reconcile port allocation state with actual
  container status

## Export and Import Errors

**Symptom:** Export fails or produces a corrupted file.

**Solutions:**
- Ensure sufficient disk space for the export file
- Verify write permissions to the target directory
- If the export is interrupted, delete the partial file and retry

**Symptom:** Import fails with a decryption error.

**Solutions:**
- Verify you are using the correct password (passwords are case-sensitive)
- Ensure the export file has not been modified or truncated
- If the file was transferred between machines, verify the transfer was
  binary-safe (not text-mode FTP or email attachment corruption)

**Symptom:** Import succeeds but personas show warning indicators.

**Solutions:**
- This is expected — secrets (API keys) are not included in exports
- Navigate to the **Agents** page and configure the required credentials
- Check that workspace paths referenced by imported personas exist on
  this machine; update paths if needed

## General Tips

- Run **System Settings > Diagnostics** for a comprehensive health check
- Check the Beachead logs for detailed error messages
- Ensure you are running the latest version of both Beachead and sbx
- Restart Docker Desktop if sandboxes behave unexpectedly
