#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "Error: Silakan jalankan dengan sudo: sudo ./uninstall.sh"
    exit 1
fi

echo "=== Uninstalling ASUS Power Manager ==="
make uninstall

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database /usr/share/applications || true
fi

echo "✓ Uninstallation complete."
