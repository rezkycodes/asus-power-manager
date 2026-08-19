#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "Error: please run with sudo: sudo ./uninstall.sh"
    exit 1
fi

echo "=== Uninstalling Tweaks ASUS TUF ==="
if command -v systemctl >/dev/null 2>&1; then
    systemctl disable --now battery-charge-threshold.service || true
fi

make uninstall

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi

echo "✓ Uninstallation complete."
