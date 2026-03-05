#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────
# ZenGuard uninstall script
# Removes the binary, assets, and systemd user service.
# The user config (~/.config/zenguard/) is preserved by default.
#
# Usage:
#   ./uninstall.sh                # remove everything except config
#   ./uninstall.sh --purge-config # also remove config
# ──────────────────────────────────────────────────────────────────────────
set -euo pipefail

BIN_NAME="zenguard"
INSTALL_BIN="${HOME}/.local/bin/${BIN_NAME}"
ASSETS_DIR="${HOME}/.local/share/${BIN_NAME}"
SERVICE_DIR="${HOME}/.config/systemd/user"
SERVICE_FILE="${SERVICE_DIR}/${BIN_NAME}.service"
CONFIG_DIR="${HOME}/.config/zenguard"
PURGE_CONFIG=false

# ── Parse args ─────────────────────────────────────────────────────────────
for arg in "$@"; do
    case "$arg" in
        --purge-config) PURGE_CONFIG=true ;;
        --help|-h)
            echo "Usage: $0 [--purge-config]"
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

# ── Stop and disable service ───────────────────────────────────────────────
if command -v systemctl &>/dev/null; then
    blue "==> Stopping and disabling systemd user service..."
    systemctl --user stop    "${BIN_NAME}.service" 2>/dev/null || true
    systemctl --user disable "${BIN_NAME}.service" 2>/dev/null || true
fi

# ── Remove service file ────────────────────────────────────────────────────
if [[ -f "${SERVICE_FILE}" ]]; then
    blue "==> Removing service file..."
    rm -f "${SERVICE_FILE}"
    systemctl --user daemon-reload 2>/dev/null || true
    green "    Service file removed"
fi

# ── Remove binary ──────────────────────────────────────────────────────────
if [[ -f "${INSTALL_BIN}" ]]; then
    blue "==> Removing binary..."
    rm -f "${INSTALL_BIN}"
    green "    Binary removed"
fi

# ── Remove assets ──────────────────────────────────────────────────────────
if [[ -d "${ASSETS_DIR}" ]]; then
    blue "==> Removing assets..."
    rm -rf "${ASSETS_DIR}"
    green "    Assets removed"
fi

# ── Remove icon ────────────────────────────────────────────────────────────
ICON_FILE="${HOME}/.local/share/icons/hicolor/256x256/apps/${BIN_NAME}.png"
if [[ -f "${ICON_FILE}" ]]; then
    rm -f "${ICON_FILE}"
    if command -v gtk-update-icon-cache &>/dev/null; then
        gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" 2>/dev/null || true
    fi
fi

# ── Optionally remove config ───────────────────────────────────────────────
if [[ "${PURGE_CONFIG}" == "true" ]]; then
    if [[ -d "${CONFIG_DIR}" ]]; then
        blue "==> Removing config directory (--purge-config)..."
        rm -rf "${CONFIG_DIR}"
        green "    Config removed"
    fi
else
    blue "==> Config preserved at ${CONFIG_DIR}"
    echo "      Run with --purge-config to remove it."
fi

# ── Done ───────────────────────────────────────────────────────────────────
echo ""
green "✓  ZenGuard uninstalled."
echo ""
