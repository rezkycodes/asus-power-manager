#!/usr/bin/env bash
# Set Battery Charge Threshold (e.g. 80 or 100)
set -euo pipefail

VAL="${1:-80}"
if [[ "$VAL" != "80" && "$VAL" != "100" && "$VAL" != "60" ]]; then
    echo "Usage: $0 [60|80|100]"
    exit 1
fi

APPLIED=false
for b in /sys/class/power_supply/BAT*; do
    if [[ -f "$b/charge_control_end_threshold" ]]; then
        echo "$VAL" > "$b/charge_control_end_threshold"
        APPLIED=true
        echo "Set $b charge_control_end_threshold to $VAL%"
    fi
done

# Update udev rule
echo "SUBSYSTEM==\"power_supply\", ATTR{charge_control_end_threshold}=\"$VAL\"" > /etc/udev/rules.d/99-battery-charge-threshold.rules

if $APPLIED; then
    echo "Threshold $VAL% applied successfully."
else
    echo "No battery found with charge_control_end_threshold support."
fi
