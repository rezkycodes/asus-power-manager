#!/usr/bin/env bash
# ==============================================================================
# battery-mouse-logitech.sh — Logitech G304 Control Script via Solaar CLI
# ==============================================================================

set -euo pipefail

ACTION="${1:-status}"
PARAM="${2:-}"

M_NAME="G304"

case "$ACTION" in
    status)
        solaar show 2>&1 | grep -iE "battery:|report rate:|dpi =|onboard_profiles =" || true
        ;;

    hz|rate)
        # Rates: 1000 (1ms), 500 (2ms), 250 (4ms), 125 (8ms)
        HZ_VAL="1ms"
        if [[ "$PARAM" == "1000" || "$PARAM" == "1ms" ]]; then HZ_VAL="1ms"
        elif [[ "$PARAM" == "500" || "$PARAM" == "2ms" ]]; then HZ_VAL="2ms"
        elif [[ "$PARAM" == "250" || "$PARAM" == "4ms" ]]; then HZ_VAL="4ms"
        elif [[ "$PARAM" == "125" || "$PARAM" == "8ms" ]]; then HZ_VAL="8ms"
        fi
        solaar config "$M_NAME" onboard_profiles Disabled 2>/dev/null || true
        solaar config "$M_NAME" report_rate "$HZ_VAL" 2>&1
        ;;

    dpi)
        DPI_VAL="${PARAM:-1600}"
        solaar config "$M_NAME" onboard_profiles Disabled 2>/dev/null || true
        solaar config "$M_NAME" dpi "$DPI_VAL" 2>&1
        ;;

    onboard)
        # Disabled or Profile 1
        OB_VAL="Disabled"
        if [[ "$PARAM" == "1" || "$PARAM" == "on" || "$PARAM" == "Profile 1" ]]; then
            OB_VAL="Profile 1"
        fi
        solaar config "$M_NAME" onboard_profiles "$OB_VAL" 2>&1
        ;;

    *)
        echo "Usage: $0 [status | hz 1000|500|250|125 | dpi 400|800|1600|3200 | onboard on|off]"
        exit 1
        ;;
esac
