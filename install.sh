#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "Error: Silakan jalankan dengan sudo: sudo ./install.sh"
    exit 1
fi

echo "=== Installing ASUS Power Manager ==="
make install

# Reload services
if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules || true
    udevadm trigger --subsystem-match=power_supply || true
fi
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload || true
    systemctl kill --kill-who=main --signal=HUP systemd-logind.service || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi

echo "✓ Installation complete! Run 'asus-power-manager' or search 'Power & Battery Manager' in GNOME."
