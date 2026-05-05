#!/usr/bin/env bash
# Beachead - Post-Installation Dependency Check
# This script verifies that required dependencies (sbx CLI and Docker)
# are available on the system. It is informational only — the app can
# still start without sbx, but sandbox features will be unavailable.

set -euo pipefail

BOLD='\033[1m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo ""
echo -e "${BOLD}╔══════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║   Beachead - Secure AI Orchestrator              ║${NC}"
echo -e "${BOLD}║   Post-Installation Dependency Check             ║${NC}"
echo -e "${BOLD}╚══════════════════════════════════════════════════╝${NC}"
echo ""

SBX_OK=false
DOCKER_OK=false

# --- Check for sbx CLI ---
echo -e "${BOLD}Checking for Docker Sandboxes (sbx) CLI...${NC}"
if command -v sbx &>/dev/null; then
    SBX_VERSION=$(sbx version 2>/dev/null || echo "unknown")
    echo -e "  ${GREEN}✓${NC} sbx found: ${SBX_VERSION}"
    SBX_OK=true
else
    echo -e "  ${RED}✗${NC} sbx not found on PATH"
    echo ""
    echo -e "  ${YELLOW}To install sbx:${NC}"
    if [[ "$OSTYPE" == "darwin"* ]]; then
        echo "    brew install docker/tap/sbx"
    elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
        echo "    1. Download the latest release from:"
        echo "       https://github.com/docker/sbx-releases/releases"
        echo "    2. Extract and place the binary on your PATH:"
        echo "       sudo mv sbx /usr/local/bin/sbx"
        echo "       sudo chmod +x /usr/local/bin/sbx"
    fi
    echo ""
    echo "  After installing, sign in with:"
    echo "    sbx login"
    echo ""
fi

# --- Check for Docker ---
echo -e "${BOLD}Checking for Docker Engine...${NC}"
if command -v docker &>/dev/null; then
    DOCKER_VERSION=$(docker --version 2>/dev/null || echo "unknown")
    echo -e "  ${GREEN}✓${NC} Docker found: ${DOCKER_VERSION}"
    DOCKER_OK=true
else
    echo -e "  ${RED}✗${NC} docker not found on PATH"
    echo ""
    echo -e "  ${YELLOW}To install Docker:${NC}"
    if [[ "$OSTYPE" == "darwin"* ]]; then
        echo "    brew install --cask docker"
        echo "    OR download Docker Desktop from:"
        echo "    https://www.docker.com/products/docker-desktop/"
    elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
        echo "    Follow the official install guide for your distribution:"
        echo "    https://docs.docker.com/engine/install/"
        echo ""
        echo "    After installing, add your user to the docker group:"
        echo "    sudo usermod -aG docker \$USER"
        echo "    Then log out and back in."
    fi
    echo ""
fi

echo ""
echo -e "${BOLD}── Verification ──${NC}"
echo ""
# --- Verification step ---
if [ "$SBX_OK" = true ]; then
    echo -e "${BOLD}Running sbx version...${NC}"
    sbx version 2>&1 | sed 's/^/  /'
    echo ""

    echo -e "${BOLD}Verifying Docker auth (sbx ls)...${NC}"
    if sbx ls &>/dev/null; then
        echo -e "  ${GREEN}✓${NC} Docker authentication verified"
    else
        echo -e "  ${YELLOW}!${NC} Could not verify Docker auth. Run 'sbx login' to sign in."
    fi
    echo ""
fi

if [ "$DOCKER_OK" = true ]; then
    echo -e "${BOLD}Running docker --version...${NC}"
    docker --version 2>&1 | sed 's/^/  /'
    echo ""
fi

# --- Summary ---
echo ""
echo -e "${BOLD}── Summary ──${NC}"
echo ""
if [ "$SBX_OK" = true ] && [ "$DOCKER_OK" = true ]; then
    echo -e "  ${GREEN}✓${NC} All dependencies found. Beachead is ready to use."
elif [ "$SBX_OK" = false ] && [ "$DOCKER_OK" = false ]; then
    echo -e "  ${YELLOW}!${NC} Both sbx and Docker are missing."
    echo "    Install them to use sandbox features."
else
    if [ "$SBX_OK" = false ]; then
        echo -e "  ${YELLOW}!${NC} sbx CLI is missing. Install it to use sandbox features."
    fi
    if [ "$DOCKER_OK" = false ]; then
        echo -e "  ${YELLOW}!${NC} Docker is missing. Install it to use sandbox features."
    fi
fi
echo ""
echo "  For more information, see the README.md in the installation directory"
echo "  or visit: https://docs.docker.com/ai/sandboxes/get-started/"
echo ""
