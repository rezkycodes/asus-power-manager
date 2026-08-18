#!/usr/bin/env bash
# Set ASUS Fan Policy (0=Normal, 1=Turbo/Overboost, 2=Silent)
set -euo pipefail

POLICY="${1:-0}"
if [[ "$POLICY" != "0" && "$POLICY" != "1" && "$POLICY" != "2" ]]; then
    echo "Usage: $0 [0(Normal) | 1(Turbo) | 2(Silent)]"
    exit 1
fi

APPLIED=false
for p in /sys/devices/platform/asus-nb-wmi/throttle_thermal_policy /sys/devices/platform/asus-nb-wmi/fan_boost_mode; do
    if [[ -f "$p" ]]; then
        echo "$POLICY" > "$p" 2>/dev/null || true
        APPLIED=true
    fi
done

if $APPLIED; then
    echo "Fan policy set to $POLICY successfully."
else
    echo "ASUS thermal policy interface not found."
fi
