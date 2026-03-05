#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────
# ZenGuard install script
# Installs the binary, assets, and (optionally) the systemd user service.
#
# Usage:
#   ./install.sh            # release build + service enable
#   ./install.sh --no-service   # install binary/assets only
# ──────────────────────────────────────────────────────────────────────────
set -euo pipefail

BIN_NAME="zenguard"
INSTALL_BIN="${HOME}/.local/bin/${BIN_NAME}"
ASSETS_DIR="${HOME}/.local/share/${BIN_NAME}"
SERVICE_DIR="${HOME}/.config/systemd/user"
SERVICE_FILE="systemd/${BIN_NAME}.service"
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

# ── Colour helpers ─────────────────────────────────────────────────────────
green() { printf '\033[0;32m%s\033[0m\n' "$*"; }
blue()  { printf '\033[0;34m%s\033[0m\n' "$*"; }
red()   { printf '\033[0;31m%s\033[0m\n' "$*" >&2; }

# ── Pre-flight checks ──────────────────────────────────────────────────────
blue "==> Checking dependencies..."

if ! command -v cargo &>/dev/null; then
    red "cargo not found — install Rust from https://rustup.rs"
    exit 1
fi

for dep in pkg-config; do
    if ! command -v "$dep" &>/dev/null; then
        red "Missing build dependency: $dep"
        red "On Ubuntu: sudo apt install build-essential pkg-config"
        exit 1
    fi
done

# Check GTK4 dev headers
if ! pkg-config --exists gtk4 2>/dev/null; then
    red "GTK4 development headers not found."
    red "On Ubuntu 24.04: sudo apt install libgtk-4-dev"
    exit 1
fi

green "    Dependencies OK"

# ── Build ──────────────────────────────────────────────────────────────────
blue "==> Building ZenGuard (release)..."
cargo build --release
green "    Build complete"

# ── Install binary ─────────────────────────────────────────────────────────
blue "==> Installing binary to ${INSTALL_BIN}..."
mkdir -p "$(dirname "${INSTALL_BIN}")"
install -m 755 "target/release/${BIN_NAME}" "${INSTALL_BIN}"
green "    Binary installed"

# ── Install assets ─────────────────────────────────────────────────────────
blue "==> Installing assets to ${ASSETS_DIR}..."
mkdir -p "${ASSETS_DIR}/animations"

install -m 644 assets/eye_blink.svg         "${ASSETS_DIR}/eye_blink.svg"
install -m 644 assets/animations/blink.css  "${ASSETS_DIR}/animations/blink.css"
install -m 644 assets/animations/breathe.css "${ASSETS_DIR}/animations/breathe.css"

# Install app icon if present
if [[ -f assets/icon.png ]]; then
    ICON_DIR="${HOME}/.local/share/icons/hicolor/256x256/apps"
    mkdir -p "${ICON_DIR}"
    install -m 644 assets/icon.png "${ICON_DIR}/${BIN_NAME}.png"
    # Update icon cache if gtk-update-icon-cache is available
    if command -v gtk-update-icon-cache &>/dev/null; then
        gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" 2>/dev/null || true
    fi
    green "    Icon installed"
fi

green "    Assets installed"

# ── Write default config (if not already present) ──────────────────────────
CONFIG_PATH="${HOME}/.config/zenguard/config.toml"
if [[ ! -f "${CONFIG_PATH}" ]]; then
    blue "==> Writing default config to ${CONFIG_PATH}..."
    mkdir -p "$(dirname "${CONFIG_PATH}")"
    install -m 644 config/default_config.toml "${CONFIG_PATH}"
    green "    Default config written"
else
    blue "==> Config already exists at ${CONFIG_PATH} — skipping"
fi

# ── Ensure ~/.local/bin is on PATH ─────────────────────────────────────────
if [[ ":${PATH}:" != *":${HOME}/.local/bin:"* ]]; then
    printf '\n\033[0;33m[WARN]\033[0m %s is not in your PATH.\n' "${HOME}/.local/bin"
    echo "      Add this to your shell profile (~/.bashrc, ~/.zshrc, ~/.config/fish/config.fish):"
    echo "      export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

# ── Systemd user service ────────────────────────────────────────────────────
if [[ "${ENABLE_SERVICE}" == "true" ]]; then
    blue "==> Installing systemd user service..."

    if ! command -v systemctl &>/dev/null; then
        printf '\033[0;33m[WARN]\033[0m systemctl not found — skipping service install\n'
    else
        mkdir -p "${SERVICE_DIR}"
        install -m 644 "${SERVICE_FILE}" "${SERVICE_DIR}/${BIN_NAME}.service"

        systemctl --user daemon-reload
        systemctl --user enable "${BIN_NAME}.service"
        systemctl --user restart "${BIN_NAME}.service"

        sleep 1
        if systemctl --user is-active --quiet "${BIN_NAME}.service"; then
            green "    Service enabled and running"
        else
            printf '\033[0;33m[WARN]\033[0m Service installed but may not be active yet.\n'
            printf '      Check with: systemctl --user status %s\n' "${BIN_NAME}"
        fi
    fi
fi

# ── Done ───────────────────────────────────────────────────────────────────
echo ""
green "✓  ZenGuard installed successfully!"
echo ""
echo "  Binary:  ${INSTALL_BIN}"
echo "  Assets:  ${ASSETS_DIR}"
echo "  Config:  ${CONFIG_PATH}"
if [[ "${ENABLE_SERVICE}" == "true" ]]; then
    echo "  Service: ${SERVICE_DIR}/${BIN_NAME}.service"
fi
echo ""
echo "  Manage:  systemctl --user {start|stop|restart|status} ${BIN_NAME}"
echo "  Logs:    journalctl --user -u ${BIN_NAME} -f"
echo "  Config:  \$EDITOR ${CONFIG_PATH}"
echo ""
