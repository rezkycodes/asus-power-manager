#!/usr/bin/env bash
# ==============================================================================
# battery-set-rgb.sh — ASUS Keyboard RGB Backlight Control
# ==============================================================================

set -euo pipefail

MODE="${1:-0}"         # 0=Static, 1=Breathing, 2=Cycle, 3=Strobe, 10=Pulse
RED="${2:-0}"          # 0-255
GREEN="${3:-200}"      # 0-255
BLUE="${4:-255}"       # 0-255
SPEED="${5:-1}"        # 0=Slow, 1=Med, 2=Fast
BRIGHTNESS="${6:-3}"   # 0=Off, 1=Low, 2=Med, 3=High

KBD_DIR="/sys/devices/platform/asus-nb-wmi/leds/asus::kbd_backlight"

# 1. Set brightness first (0-3)
if [[ -f "$KBD_DIR/brightness" ]]; then
    echo "$BRIGHTNESS" > "$KBD_DIR/brightness" 2>/dev/null || true
fi

# 2. Set mode with speed and RGB
if [[ -f "$KBD_DIR/kbd_rgb_mode" ]]; then
    # Format: cmd mode red green blue speed
    echo "1 $MODE $RED $GREEN $BLUE $SPEED" > "$KBD_DIR/kbd_rgb_mode" 2>/dev/null || true
fi

# Save persistent config
CONFIG_DIR="/etc/asus-power-manager"
mkdir -p "$CONFIG_DIR"
cat << EOF > "$CONFIG_DIR/rgb.conf"
MODE=$MODE
RED=$RED
GREEN=$GREEN
BLUE=$BLUE
SPEED=$SPEED
BRIGHTNESS=$BRIGHTNESS
EOF

echo "RGB Mode: $MODE, Color: ($RED,$GREEN,$BLUE), Speed: $SPEED, Brightness: $BRIGHTNESS applied."
