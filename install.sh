#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "Error: please run with sudo: sudo ./install.sh"
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: 'cargo' (Rust toolchain) is required to build. Install it from https://rustup.rs and retry."
    exit 1
fi

echo "=== Installing Tweaks ASUS TUF (Rust) ==="
make build
make install

# Reload services & caches
if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules || true
    udevadm trigger --subsystem-match=power_supply || true
fi
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload || true
    systemctl enable --now battery-charge-threshold.service || true
    systemctl kill --kill-who=main --signal=HUP systemd-logind.service || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi

echo "✓ Installation complete! Run 'asus-tuf-cpu' or search 'Tweaks ASUS TUF' in GNOME."
