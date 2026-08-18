#!/usr/bin/env bash
# ==============================================================================
# battery-set-fan.sh — ASUS Fan & Platform Profile Synchronizer
# ==============================================================================

set -euo pipefail

POLICY="${1:-0}" # 0=Normal, 1=Turbo, 2=Silent

if [[ "$POLICY" != "0" && "$POLICY" != "1" && "$POLICY" != "2" ]]; then
    echo "Usage: $0 [0(Normal) | 1(Turbo) | 2(Silent)]"
    exit 1
fi

# 1. Set ASUS Thermal Throttle Policy & Fan Boost Mode
for p in /sys/devices/platform/asus-nb-wmi/throttle_thermal_policy /sys/devices/platform/asus-nb-wmi/fan_boost_mode; do
    if [[ -f "$p" ]]; then
        echo "$POLICY" > "$p" 2>/dev/null || true
    fi
done

# 2. Sync ASUS ACPI Platform Profile
PROFILE="balanced"
if [[ "$POLICY" == "1" ]]; then
    PROFILE="performance"
elif [[ "$POLICY" == "2" ]]; then
    PROFILE="quiet"
fi

if [[ -f "/sys/firmware/acpi/platform_profile" ]]; then
    echo "$PROFILE" > /sys/firmware/acpi/platform_profile 2>/dev/null || true
fi

echo "Fan policy set to $POLICY (Profile: $PROFILE) successfully."
