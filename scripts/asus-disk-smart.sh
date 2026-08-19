#!/bin/bash
# asus-disk-smart.sh — Read S.M.A.R.T. data for a given device.
# Usage: asus-disk-smart.sh <device>  (e.g. nvme0n1, sda)
# Outputs KEY=VALUE lines for the GUI to parse.

DEV="$1"
if [ -z "$DEV" ]; then
    echo "ERROR=no device specified"
    exit 1
fi

if ! command -v smartctl &>/dev/null; then
    echo "SMARTCTL_MISSING=1"
    exit 0
fi

OUTPUT=$(smartctl -a "/dev/$DEV" 2>&1)
HEALTH_OUT=$(smartctl -H "/dev/$DEV" 2>&1)

# Health status
HEALTH=$(echo "$HEALTH_OUT" | grep -i "SMART overall-health" | sed 's/.*result: *//')
[ -z "$HEALTH" ] && HEALTH="Unknown"
echo "HEALTH=$HEALTH"

# Model
if echo "$DEV" | grep -q "^nvme"; then
    MODEL=$(echo "$OUTPUT" | grep "^Model Number:" | sed 's/^Model Number: *//')
else
    MODEL=$(echo "$OUTPUT" | grep "^Device Model:" | sed 's/^Device Model: *//')
    [ -z "$MODEL" ] && MODEL=$(echo "$OUTPUT" | grep "^Model Family:" | sed 's/^Model Family: *//')
fi
echo "MODEL=$MODEL"

# NVMe parsing
if echo "$DEV" | grep -q "^nvme"; then
    TEMP=$(echo "$OUTPUT" | grep "^Temperature:" | awk '{print $2}')
    POWER_ON_HOURS=$(echo "$OUTPUT" | grep "^Power On Hours:" | awk -F: '{print $2}' | tr -d ' ,')
    POWER_CYCLES=$(echo "$OUTPUT" | grep "^Power Cycles:" | awk -F: '{print $2}' | tr -d ' ,')
    PERCENT_USED=$(echo "$OUTPUT" | grep "^Percentage Used:" | awk '{print $3}' | tr -d '%')
    DATA_WRITTEN=$(echo "$OUTPUT" | grep "^Data Units Written:" | sed 's/.*\[//;s/\]//')
    REALLOCATED="N/A"
else
    # SATA parsing
    TEMP=$(echo "$OUTPUT" | grep "^ *194 " | awk '{print $10}')
    [ -z "$TEMP" ] && TEMP=$(echo "$OUTPUT" | grep "^ *190 " | awk '{print $10}')
    POWER_ON_HOURS=$(echo "$OUTPUT" | grep "^ *  9 " | awk '{print $10}' | sed 's/ .*//')
    POWER_CYCLES=$(echo "$OUTPUT" | grep "^ * 12 " | awk '{print $10}')
    REALLOCATED=$(echo "$OUTPUT" | grep "^ *  5 " | awk '{print $10}')
    [ -z "$REALLOCATED" ] && REALLOCATED="0"
    PERCENT_USED=""
    # Total data written for SATA: attribute 241 (Total_LBAs_Written) * 512 bytes
    LBAS=$(echo "$OUTPUT" | grep "^ *241 " | awk '{print $10}')
    if [ -n "$LBAS" ] && [ "$LBAS" != "0" ]; then
        # Convert LBAs to human-readable (LBA * 512 bytes)
        BYTES=$(echo "$LBAS * 512" | bc 2>/dev/null)
        if [ -n "$BYTES" ]; then
            TB=$(echo "scale=2; $BYTES / 1000000000000" | bc 2>/dev/null)
            if [ -n "$TB" ] && [ "$(echo "$TB > 0" | bc 2>/dev/null)" = "1" ]; then
                DATA_WRITTEN="${TB} TB"
            else
                GB=$(echo "scale=1; $BYTES / 1000000000" | bc 2>/dev/null)
                DATA_WRITTEN="${GB} GB"
            fi
        fi
    fi
fi

echo "TEMP=${TEMP:-Unknown}"
echo "POWER_ON_HOURS=${POWER_ON_HOURS:-Unknown}"
echo "POWER_CYCLES=${POWER_CYCLES:-Unknown}"
echo "REALLOCATED=${REALLOCATED:-N/A}"
echo "PERCENT_USED=${PERCENT_USED:-N/A}"
echo "DATA_WRITTEN=${DATA_WRITTEN:-N/A}"
