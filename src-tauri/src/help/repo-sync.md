# Repo Sync

Repo Sync manages git remote synchronization using a two-directory architecture. The agent works in a remote-free workspace where it can only commit locally. A separate host-side mirror directory holds the real remote configuration and credentials. All sync operations are user-initiated and run on the host.

## How It Works

When Repo Sync is enabled for a repository:

1. A **mirror** clone is created on the host with all remotes preserved
2. All remotes are stripped from the **workspace** `.git/config`
3. The agent can commit locally but cannot push, pull, or access remote URLs
4. You control when changes flow between workspace, mirror, and remote

This ensures credentials and remote access are never visible inside the sandbox.

## Enabling Repo Sync

### Existing repositories (with remotes)

Click **Scan Workspace** on the Repo Sync page. Repositories with configured remotes appear with an **Enable Repo Sync** button. Clicking it:

- Clones the workspace to the mirror directory (preserving remotes)
- Strips all remotes from the workspace
- Creates a managed repo record with default settings

### Agent-created repositories (no remotes)

Repositories created by the agent (no remotes) appear with two options:

- **Link to remote** — Provide a remote URL. A mirror is created with the URL added as `origin`. Sync mode is set to "Remote".
- **Keep local only** — A mirror is created with no remote. Sync mode is set to "Local only". You can add a remote later via the settings panel.

## Sync Operations

Four operations move commits between the workspace, mirror, and remote:

### Pull from Agent (Workspace → Mirror)

Fetches new commits from the workspace into the mirror and merges them. Use this after the agent has made commits you want to review or push upstream.

### Push to Remote (Mirror → Remote)

Pushes selected commits from the mirror to the configured remote. Opens a commit review view where you can:

- Select/deselect individual commits (cherry-pick)
- Squash multiple commits into one
- Review diffs before pushing

Requires credentials to be configured. Disabled when sync mode is "Local only".

### Fetch from Remote (Remote → Mirror)

Fetches new commits from the remote into the mirror. Use this to check for upstream changes before pushing them to the agent. Disabled when sync mode is "Local only".

### Push to Agent (Mirror → Workspace)

Pulls commits from the mirror into the workspace so the agent can work with upstream changes or resolved conflicts. Fails if the workspace has uncommitted changes.

## Credential Setup

Credentials are stored in your OS keyring (macOS Keychain, GNOME Keyring, Windows Credential Manager) and never written to git config or exposed to the sandbox.

To configure credentials for a repository:

1. Open the repo's settings panel on the Repo Sync page
2. Enter your username and token/password
3. Click Save

The system uses a `GIT_ASKPASS` helper binary that reads credentials from the keyring at authentication time. Credentials are keyed per-repository using the pattern `beachead-repo-sync-<repo-id>`.

## Mirror Directory Configuration

Mirrors are stored in a configurable directory with platform-specific defaults:

- **Linux**: `~/.local/share/beachead/mirrors/`
- **macOS**: `~/Library/Application Support/beachead/mirrors/`
- **Windows**: `%APPDATA%\beachead\mirrors\`

Each mirror is stored at `<mirrors-dir>/<persona-name>/<project-folder-name>/`.

To change the mirrors directory, use the Mirrors Directory settings section on the Repo Sync page. Existing mirrors are not moved automatically — you must relocate them manually if needed.

## Secret Scanning

Before pushing to remote, Repo Sync scans commits for potential secrets:

- `.env` files and `.env.*` variants
- Private key content (`-----BEGIN ... PRIVATE KEY-----`)
- AWS access keys (`AKIA...`)
- GitHub tokens (`ghp_*`, `gho_*`)
- GitLab tokens (`glpat-*`)
- Secret file extensions (`.pem`, `.key`, `.p12`, `.pfx`)

Two modes are available per repository:

- **Block** (default) — Push is rejected if secrets are detected
- **Warn only** — A warning is shown but you can proceed

Binary files are skipped during scanning.

## Troubleshooting

### Git not found

Repo Sync requires `git` to be installed and available in your PATH. Install git for your platform:

- **Linux**: `sudo apt install git` or equivalent for your distribution
- **macOS**: `xcode-select --install` or install via Homebrew
- **Windows**: Download from https://git-scm.com/download/win

Restart Beachead after installing git.

### Keyring locked or unavailable

Credentials are stored in the OS keyring. If the keyring is locked:

- **Linux (GNOME)**: Unlock via `gnome-keyring-daemon` or log out and back in. Ensure `libsecret` is installed.
- **macOS**: Unlock Keychain Access or restart your session
- **Windows**: Ensure Windows Credential Manager service is running

### Merge conflicts

Conflicts can occur during "Pull from agent" (workspace → mirror) or "Push to agent" (mirror → workspace). When a conflict occurs:

- The affected directory is left in a merge-conflict state with conflict markers in files
- Resolve conflicts manually in the appropriate directory (mirror or workspace)
- For mirror conflicts: navigate to the mirror path shown in the repo settings and resolve using standard git tools
- For workspace conflicts: the agent can resolve them in subsequent commits

### Authentication failures

If push or fetch operations fail with authentication errors:

- Verify your credentials are correct in the repo settings panel
- Check that your token has the required permissions (repo read/write)
- For GitHub: ensure the token has not expired
- For GitLab: ensure the token has `write_repository` scope
- Try removing and re-saving credentials

Authentication errors are distinct from keyring errors — if the keyring itself is inaccessible, you will see a "keyring unavailable" message instead.
