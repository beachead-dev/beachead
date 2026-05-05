# System Settings

## Overview

The System Settings page provides tools for managing your Beachead
installation, checking dependencies, and configuring system-level options.

## Dependency Check

Verify that all required dependencies are installed and accessible:

- **Docker** — Docker Engine or Docker Desktop must be running
- **sbx CLI** — The Docker Sandboxes CLI tool
- **Authentication** — Logged in via `sbx login`

## Version Information

View the installed version of the sbx CLI and check for compatibility
with the current Beachead release.

## Diagnostics

Run `sbx diagnose` to check the health of your Docker Sandboxes
installation. This verifies:

- Docker daemon connectivity
- Sandbox runtime availability
- Network configuration
- Storage and disk space

## Authentication

- **Login** — Authenticate with Docker Hub via `sbx login`
- **Logout** — Sign out via `sbx logout`
- **Status** — View current authentication state

## Data Storage

Beachead stores configuration in a local SQLite database. Session data,
persona configurations, and agent registrations are persisted locally.
Credentials are never stored in the database — they use the OS keychain
exclusively via `sbx secret`.
