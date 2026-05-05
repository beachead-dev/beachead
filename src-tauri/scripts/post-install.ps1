# Beachead - Post-Installation Dependency Check (Windows)
# This script verifies that required dependencies (sbx CLI and Docker)
# are available on the system. It is informational only - the app can
# still start without sbx, but sandbox features will be unavailable.

$ErrorActionPreference = "Continue"

Write-Host ""
Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Beachead - Secure AI Orchestrator" -ForegroundColor Cyan
Write-Host "  Post-Installation Dependency Check" -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan
Write-Host ""

$SbxOk = $false
$DockerOk = $false

# --- Check for sbx CLI ---
Write-Host "Checking for Docker Sandboxes (sbx) CLI..." -ForegroundColor White
$sbxCmd = Get-Command sbx -ErrorAction SilentlyContinue
if ($sbxCmd) {
    try {
        $sbxVersion = & sbx version 2>&1
        Write-Host "  [OK] sbx found: $sbxVersion" -ForegroundColor Green
        $SbxOk = $true
    } catch {
        Write-Host "  [OK] sbx found but version check failed" -ForegroundColor Yellow
        $SbxOk = $true
    }
} else {
    Write-Host "  [X] sbx not found on PATH" -ForegroundColor Red
    Write-Host ""
    Write-Host "  To install sbx:" -ForegroundColor Yellow
    Write-Host "    winget install Docker.sbx"
    Write-Host ""
    Write-Host "  OR download from:"
    Write-Host "    https://github.com/docker/sbx-releases/releases"
    Write-Host ""
    Write-Host "  After installing, sign in with:"
    Write-Host "    sbx login"
    Write-Host ""
}

# --- Check for Docker ---
Write-Host "Checking for Docker Engine..." -ForegroundColor White
$dockerCmd = Get-Command docker -ErrorAction SilentlyContinue
if ($dockerCmd) {
    try {
        $dockerVersion = & docker --version 2>&1
        Write-Host "  [OK] Docker found: $dockerVersion" -ForegroundColor Green
        $DockerOk = $true
    } catch {
        Write-Host "  [OK] Docker found but version check failed" -ForegroundColor Yellow
        $DockerOk = $true
    }
} else {
    Write-Host "  [X] docker not found on PATH" -ForegroundColor Red
    Write-Host ""
    Write-Host "  To install Docker:" -ForegroundColor Yellow
    Write-Host "    winget install Docker.DockerDesktop"
    Write-Host ""
    Write-Host "  OR download Docker Desktop from:"
    Write-Host "    https://www.docker.com/products/docker-desktop/"
    Write-Host ""
}

Write-Host ""
Write-Host "-- Verification --" -ForegroundColor White
Write-Host ""

# --- Verification step ---
if ($SbxOk) {
    Write-Host "Running sbx version..." -ForegroundColor White
    & sbx version 2>&1 | ForEach-Object { Write-Host "  $_" }
    Write-Host ""

    Write-Host "Verifying Docker auth (sbx ls)..." -ForegroundColor White
    try {
        $null = & sbx ls 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Host "  [OK] Docker authentication verified" -ForegroundColor Green
        } else {
            Write-Host "  [!] Could not verify Docker auth. Run 'sbx login' to sign in." -ForegroundColor Yellow
        }
    } catch {
        Write-Host "  [!] Could not verify Docker auth. Run 'sbx login' to sign in." -ForegroundColor Yellow
    }
    Write-Host ""
}

if ($DockerOk) {
    Write-Host "Running docker --version..." -ForegroundColor White
    & docker --version 2>&1 | ForEach-Object { Write-Host "  $_" }
    Write-Host ""
}

# --- Summary ---
Write-Host ""
Write-Host "-- Summary --" -ForegroundColor White
Write-Host ""
if ($SbxOk -and $DockerOk) {
    Write-Host "  [OK] All dependencies found. Beachead is ready to use." -ForegroundColor Green
} elseif (-not $SbxOk -and -not $DockerOk) {
    Write-Host "  [!] Both sbx and Docker are missing." -ForegroundColor Yellow
    Write-Host "    Install them to use sandbox features."
} else {
    if (-not $SbxOk) {
        Write-Host "  [!] sbx CLI is missing. Install it to use sandbox features." -ForegroundColor Yellow
    }
    if (-not $DockerOk) {
        Write-Host "  [!] Docker is missing. Install it to use sandbox features." -ForegroundColor Yellow
    }
}
Write-Host ""
Write-Host "  For more information, see the README.md in the installation directory"
Write-Host "  or visit: https://docs.docker.com/ai/sandboxes/get-started/"
Write-Host ""
