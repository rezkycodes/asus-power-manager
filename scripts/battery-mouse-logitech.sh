#!/usr/bin/env bash
# ==============================================================================
# battery-mouse-logitech.sh — Logitech G304 Control Script with Local Caching
# ==============================================================================

set -euo pipefail

ACTION="${1:-status}"
PARAM="${2:-}"

M_NAME="G304"
CONFIG_DIR="${HOME:-/home/rezkycodes}/.config/asus-power-manager"
CONFIG_FILE="$CONFIG_DIR/logitech.conf"

# Read cached value
get_cached() {
    if [[ -f "$CONFIG_FILE" ]]; then
        grep -i "^$1=" "$CONFIG_FILE" | cut -d= -f2 || echo "$2"
    else
        echo "$2"
    fi
}

# Update config value
set_cached() {
    local key="$1"
    local val="$2"
    mkdir -p "$CONFIG_DIR"
    if [[ -f "$CONFIG_FILE" ]]; then
        if grep -q "^$key=" "$CONFIG_FILE"; then
            sed -i "s|^$key=.*|$key=$val|" "$CONFIG_FILE"
        else
            echo "$key=$val" >> "$CONFIG_FILE"
        fi
    else
        echo "$key=$val" > "$CONFIG_FILE"
    fi
}

case "$ACTION" in
    status)
        # Try to read live status
        LIVE_OUT=$(solaar show 2>&1 || true)
        
        # Check if G304 is connected/awake in solaar output
        if echo "$LIVE_OUT" | grep -q "G304"; then
            # Extract live values
            BAT=$(echo "$LIVE_OUT" | grep -i "Battery:" | grep -oE "[0-9]+%" | head -1 | tr -d '%' || echo "")
            HZ_RAW=$(echo "$LIVE_OUT" | grep -iE "Report Rate" | head -1 || echo "")
            DPI=$(echo "$LIVE_OUT" | grep -i "Sensitivity" | grep -oE "[0-9]+" | head -1 || echo "")
            ONBOARD=$(echo "$LIVE_OUT" | grep -i "onboard_profiles" | head -1 || echo "")
            
            # Translate HZ
            HZ="1000"
            if [[ "$HZ_RAW" == *"1ms"* ]]; then HZ="1000"
            elif [[ "$HZ_RAW" == *"2ms"* ]]; then HZ="500"
            elif [[ "$HZ_RAW" == *"4ms"* ]]; then HZ="250"
            elif [[ "$HZ_RAW" == *"8ms"* ]]; then HZ="125"
            fi
            
            # Translate Onboard
            OB="off"
            if [[ "$ONBOARD" == *"Profile"* ]]; then OB="on"; fi
            
            # Update cache with live values
            [[ -n "$BAT" ]] && set_cached "BATTERY" "${BAT%%}"
            [[ -n "$HZ" ]] && set_cached "HZ" "$HZ"
            [[ -n "$DPI" ]] && set_cached "DPI" "$DPI"
            [[ -n "$OB" ]] && set_cached "ONBOARD" "$OB"
            
            echo "STATUS=Online"
            echo "BATTERY=${BAT%%}"
            echo "HZ=$HZ"
            echo "DPI=$DPI"
            echo "ONBOARD=$OB"
            echo "LED_MODE=$(get_cached "LED_MODE" "1")"
            echo "LED_COLOR=$(get_cached "LED_COLOR" "0x00c8ff")"
            echo "LED_PERIOD=$(get_cached "LED_PERIOD" "3000")"
            echo "LED_INTENSITY=$(get_cached "LED_INTENSITY" "100")"
        else
            # Fallback to cache if mouse is offline/sleeping
            echo "STATUS=Offline"
            echo "BATTERY=$(get_cached "BATTERY" "90")"
            echo "HZ=$(get_cached "HZ" "1000")"
            echo "DPI=$(get_cached "DPI" "1600")"
            echo "ONBOARD=$(get_cached "ONBOARD" "off")"
            echo "LED_MODE=$(get_cached "LED_MODE" "1")"
            echo "LED_COLOR=$(get_cached "LED_COLOR" "0x00c8ff")"
            echo "LED_PERIOD=$(get_cached "LED_PERIOD" "3000")"
            echo "LED_INTENSITY=$(get_cached "LED_INTENSITY" "100")"
        fi
        ;;

    hz|rate)
        HZ_VAL="1ms"
        if [[ "$PARAM" == "1000" || "$PARAM" == "1ms" ]]; then HZ_VAL="1ms"
        elif [[ "$PARAM" == "500" || "$PARAM" == "2ms" ]]; then HZ_VAL="2ms"
        elif [[ "$PARAM" == "250" || "$PARAM" == "4ms" ]]; then HZ_VAL="4ms"
        elif [[ "$PARAM" == "125" || "$PARAM" == "8ms" ]]; then HZ_VAL="8ms"
        fi
        # Only disable onboard profiles if not already off (avoids a slow extra solaar call)
        if [[ "$(get_cached ONBOARD off)" != "off" ]]; then
            solaar config "$M_NAME" onboard_profiles Disabled >/dev/null 2>&1 || true
            set_cached "ONBOARD" "off"
        fi
        solaar config "$M_NAME" report_rate "$HZ_VAL" >/dev/null 2>&1 || true
        
        # Save cache
        HZ_NUM="1000"
        [[ "$HZ_VAL" == "2ms" ]] && HZ_NUM="500"
        [[ "$HZ_VAL" == "4ms" ]] && HZ_NUM="250"
        [[ "$HZ_VAL" == "8ms" ]] && HZ_NUM="125"
        set_cached "HZ" "$HZ_NUM"
        set_cached "ONBOARD" "off"
        echo "HZ=$HZ_NUM"
        ;;

    dpi)
        DPI_VAL="${PARAM:-1600}"
        # Only disable onboard profiles if not already off (avoids a slow extra solaar call)
        if [[ "$(get_cached ONBOARD off)" != "off" ]]; then
            solaar config "$M_NAME" onboard_profiles Disabled >/dev/null 2>&1 || true
            set_cached "ONBOARD" "off"
        fi
        solaar config "$M_NAME" dpi "$DPI_VAL" >/dev/null 2>&1 || true
        
        # Save cache
        set_cached "DPI" "$DPI_VAL"
        echo "DPI=$DPI_VAL"
        ;;

    onboard)
        OB_VAL="Disabled"
        OB_CACHE="off"
        if [[ "$PARAM" == "1" || "$PARAM" == "on" || "$PARAM" == "Profile 1" ]]; then
            OB_VAL="Profile 1"
            OB_CACHE="on"
        fi
        solaar config "$M_NAME" onboard_profiles "$OB_VAL" >/dev/null 2>&1 || true
        
        # Save cache
        set_cached "ONBOARD" "$OB_CACHE"
        echo "ONBOARD=$OB_CACHE"
        ;;

    led|led-effect)
        MODE="${2:-1}"             # 0=Disabled, 1=Static, 3=Cycle, 10=Breathe
        RAW_COLOR="${3:-0x00c8ff}"  # e.g. 0x00c8ff, #00c8ff, or 00c8ff
        PERIOD="${4:-3000}"         # in ms
        INTENSITY="${5:-100}"       # 0-100

        # Normalize hex color to 0xRRGGBB
        HEX_CLEAN="${RAW_COLOR#\#}"
        HEX_CLEAN="${HEX_CLEAN#0x}"
        if [[ ${#HEX_CLEAN} -eq 6 ]]; then
            COLOR="0x$HEX_CLEAN"
        else
            COLOR="0x00c8ff"
        fi

        # Make sure led_control is enabled in solaar
        solaar config "$M_NAME" led_control on >/dev/null 2>&1 || true

        # Build YAML setting string for Solaar
        YAML_SET=""
        if [[ "$MODE" == "0" || "$MODE" == "off" || "$MODE" == "disabled" ]]; then
            YAML_SET="!LEDEffectSetting {ID: 0}"
            MODE="0"
        elif [[ "$MODE" == "3" || "$MODE" == "cycle" ]]; then
            YAML_SET="!LEDEffectSetting {ID: 3, period: $PERIOD, intensity: $INTENSITY}"
            MODE="3"
        elif [[ "$MODE" == "10" || "$MODE" == "breathe" || "$MODE" == "pulse" ]]; then
            YAML_SET="!LEDEffectSetting {ID: 10, color: $COLOR, period: $PERIOD, intensity: $INTENSITY}"
            MODE="10"
        else
            # Default to static (ID: 1)
            YAML_SET="!LEDEffectSetting {ID: 1, color: $COLOR, intensity: $INTENSITY}"
            MODE="1"
        fi

        solaar config "$M_NAME" led_zone_1 "$YAML_SET" >/dev/null 2>&1 || true

        # Update cache
        set_cached "LED_MODE" "$MODE"
        set_cached "LED_COLOR" "$COLOR"
        set_cached "LED_PERIOD" "$PERIOD"
        set_cached "LED_INTENSITY" "$INTENSITY"

        echo "LED_MODE=$MODE"
        echo "LED_COLOR=$COLOR"
        echo "LED_PERIOD=$PERIOD"
        echo "LED_INTENSITY=$INTENSITY"
        ;;

    *)
        echo "Usage: $0 [status | hz 1000|500|250|125 | dpi 400|800|1600|3200 | onboard on|off | led MODE [COLOR] [PERIOD] [INTENSITY]]"
        exit 1
        ;;
esac
