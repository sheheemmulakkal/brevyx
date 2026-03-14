#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────────
# Brevyx — one-command installer
#
# Downloads the latest pre-built .deb from GitHub Releases and installs it.
# Requires: curl (or wget), sudo, apt (Debian/Ubuntu)
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/sheheemmulakkal/brevyx/master/install.sh | bash
#   — or —
#   bash install.sh [--no-service]
# ──────────────────────────────────────────────────────────────────────────────
set -euo pipefail

REPO="sheheemmulakkal/brevyx"
BIN_NAME="brevyx"
ENABLE_SERVICE=true

# ── Parse args ─────────────────────────────────────────────────────────────
for arg in "$@"; do
    case "$arg" in
        --no-service) ENABLE_SERVICE=false ;;
        --help|-h)
            echo "Usage: $0 [--no-service]"
            exit 0
            ;;
        *)
            echo "Unknown argument: $arg" >&2
            exit 1
            ;;
    esac
done

# ── Colour helpers ──────────────────────────────────────────────────────────
green() { printf '\033[0;32m%s\033[0m\n' "$*"; }
blue()  { printf '\033[0;34m%s\033[0m\n' "$*"; }
red()   { printf '\033[0;31m%s\033[0m\n' "$*" >&2; }
warn()  { printf '\033[0;33m[WARN]\033[0m %s\n' "$*"; }

# ── Pre-flight checks ───────────────────────────────────────────────────────
blue "==> Checking requirements..."

if [[ "$(uname -s)" != "Linux" ]]; then
    red "Brevyx only supports Linux."
    exit 1
fi

if ! command -v apt-get &>/dev/null; then
    red "apt-get not found — Brevyx .deb install requires a Debian/Ubuntu system."
    exit 1
fi

if ! command -v sudo &>/dev/null; then
    red "sudo is required to install the .deb package."
    exit 1
fi

# Prefer curl, fall back to wget
if command -v curl &>/dev/null; then
    DOWNLOAD() { curl -fsSL -o "$1" "$2"; }
    FETCH()     { curl -fsSL "$1"; }
elif command -v wget &>/dev/null; then
    DOWNLOAD() { wget -qO "$1" "$2"; }
    FETCH()     { wget -qO- "$1"; }
else
    red "curl or wget is required. Install one and retry."
    exit 1
fi

green "    Requirements OK"

# ── Discover latest release ─────────────────────────────────────────────────
blue "==> Fetching latest release info from GitHub..."

API_URL="https://api.github.com/repos/${REPO}/releases/latest"
RELEASE_JSON=$(FETCH "$API_URL") || {
    red "Failed to reach GitHub API. Check your internet connection."
    exit 1
}

# Extract .deb download URL
DEB_URL=$(printf '%s' "$RELEASE_JSON" \
    | grep -o '"browser_download_url": *"[^"]*\.deb"' \
    | grep -o 'https://[^"]*' \
    | head -1)

if [[ -z "$DEB_URL" ]]; then
    red "No .deb asset found in the latest release."
    red "Check https://github.com/${REPO}/releases for available assets."
    exit 1
fi

VERSION=$(printf '%s' "$RELEASE_JSON" \
    | grep -o '"tag_name": *"[^"]*"' \
    | grep -o '"[^"]*"$' \
    | tr -d '"')

green "    Latest release: ${VERSION}"

# ── Download ────────────────────────────────────────────────────────────────
TMPDIR_PATH=$(mktemp -d)
trap 'rm -rf "$TMPDIR_PATH"' EXIT

DEB_FILE="${TMPDIR_PATH}/${BIN_NAME}.deb"

blue "==> Downloading ${BIN_NAME} ${VERSION}..."
DOWNLOAD "$DEB_FILE" "$DEB_URL" || {
    red "Download failed."
    exit 1
}
green "    Download complete"

# ── Install ─────────────────────────────────────────────────────────────────
blue "==> Installing .deb package..."
sudo apt-get install -y "$DEB_FILE"
green "    Package installed"

# ── Ensure systemd user service unit is in place ────────────────────────────
USER_UNIT_DIR="${HOME}/.config/systemd/user"
USER_UNIT="${USER_UNIT_DIR}/${BIN_NAME}.service"

if [[ ! -f "${USER_UNIT}" ]]; then
    blue "==> Installing systemd user unit..."
    mkdir -p "${USER_UNIT_DIR}"

    # Look for the unit file installed by the .deb in system locations
    SYSTEM_UNIT=""
    for path in /usr/lib/systemd/user /lib/systemd/user /usr/lib/systemd/system /lib/systemd/system; do
        if [[ -f "${path}/${BIN_NAME}.service" ]]; then
            SYSTEM_UNIT="${path}/${BIN_NAME}.service"
            break
        fi
    done

    if [[ -n "${SYSTEM_UNIT}" ]]; then
        cp "${SYSTEM_UNIT}" "${USER_UNIT}"
    else
        # Fallback: write the unit file directly
        cat > "${USER_UNIT}" <<'EOF'
[Unit]
Description=Brevyx — Wellness Reminder Daemon
Documentation=https://github.com/sheheemmulakkal/brevyx
After=graphical-session.target
PartOf=graphical-session.target

[Service]
Type=simple
ExecStart=/usr/bin/brevyx
Restart=on-failure
RestartSec=5s
ExecStartPre=/bin/sleep 3
PassEnvironment=DISPLAY WAYLAND_DISPLAY XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS
Environment=RUST_LOG=info
MemoryMax=128M
CPUQuota=10%
KillSignal=SIGTERM
TimeoutStopSec=5s

[Install]
WantedBy=default.target
EOF
    fi
    green "    Unit file installed"
fi

# ── Enable systemd user service ─────────────────────────────────────────────
if [[ "${ENABLE_SERVICE}" == "true" ]]; then
    blue "==> Enabling systemd user service..."
    if command -v systemctl &>/dev/null; then
        systemctl --user daemon-reload
        systemctl --user enable "${BIN_NAME}.service"
        systemctl --user start  "${BIN_NAME}.service"

        sleep 1
        if systemctl --user is-active --quiet "${BIN_NAME}.service"; then
            green "    Service enabled and running"
        else
            warn "Service installed but not yet active."
            printf '      Check with: systemctl --user status %s\n' "${BIN_NAME}"
        fi
    else
        warn "systemctl not found — skipping service setup"
    fi
fi

# ── Done ────────────────────────────────────────────────────────────────────
echo ""
green "✓  Brevyx ${VERSION} installed successfully!"
echo ""
echo "  Manage:  systemctl --user {start|stop|restart|status} ${BIN_NAME}"
echo "  Logs:    journalctl --user -u ${BIN_NAME} -f"
echo "  Config:  \$EDITOR \${HOME}/.config/${BIN_NAME}/config.toml"
echo ""
