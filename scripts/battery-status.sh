#!/usr/bin/env bash
# ==============================================================================
# battery-status.sh — Quick battery & power status check
# ==============================================================================

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

echo ""
echo -e "${CYAN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║          🔋 Battery & Power Status          ║${NC}"
echo -e "${CYAN}╚══════════════════════════════════════════════╝${NC}"
echo ""

# Battery
BAT_PATH=$(upower -e 2>/dev/null | grep BAT)
if [[ -n "$BAT_PATH" ]]; then
    BAT=$(upower -i "$BAT_PATH")
    PERCENT=$(echo "$BAT" | grep "percentage" | awk '{print $2}')
    STATE=$(echo "$BAT" | grep "state" | awk '{print $2}')
    RATE=$(echo "$BAT" | grep "energy-rate" | awk '{print $2}')
    ENERGY=$(echo "$BAT" | grep "energy:" | head -1 | awk '{print $2}')
    FULL=$(echo "$BAT" | grep "energy-full:" | head -1 | awk '{print $2}')
    DESIGN=$(echo "$BAT" | grep "energy-full-design:" | awk '{print $2}')
    CAPACITY=$(echo "$BAT" | grep "capacity:" | awk '{print $2}')
    TIME_EMPTY=$(echo "$BAT" | grep "time to empty" | awk '{print $4, $5}')
    
    # Color based on percentage
    PCT_NUM=${PERCENT%\%}
    if [[ $PCT_NUM -le 20 ]]; then
        COLOR=$RED
    elif [[ $PCT_NUM -le 50 ]]; then
        COLOR=$YELLOW
    else
        COLOR=$GREEN
    fi
    
    echo -e "  📊 Charge     : ${COLOR}${PERCENT}${NC} ($STATE)"
    echo -e "  ⚡ Drain Rate : ${RATE} W"
    [[ -n "$TIME_EMPTY" ]] && echo -e "  ⏱️  Time Left  : ${TIME_EMPTY}"
    echo -e "  🔋 Capacity   : ${CAPACITY} (${FULL} / ${DESIGN} Wh)"
fi

# CPU Governor
GOVERNOR=$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null)
MAX_FREQ=$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq 2>/dev/null)
BOOST=$(cat /sys/devices/system/cpu/cpufreq/boost 2>/dev/null)
echo ""
echo -e "  🖥️  CPU Gov    : ${CYAN}${GOVERNOR}${NC}"
echo -e "  📈 Max Freq    : $((MAX_FREQ / 1000)) MHz"
echo -e "  🚀 Turbo Boost : $([ "$BOOST" = "1" ] && echo -e "${GREEN}ON${NC}" || echo -e "${YELLOW}OFF${NC}")"

# Platform Profile
if [[ -f /sys/firmware/acpi/platform_profile ]]; then
    PROFILE=$(cat /sys/firmware/acpi/platform_profile)
    echo -e "  🎯 Profile     : ${CYAN}${PROFILE}${NC}"
fi

# NVIDIA GPU
NVIDIA_STATUS=$(cat /sys/bus/pci/devices/0000:01:00.0/power/runtime_status 2>/dev/null || echo "N/A")
echo -e "  🎮 NVIDIA GPU  : ${CYAN}${NVIDIA_STATUS}${NC}"

# Brightness
BRIGHT_FILE=$(find /sys/class/backlight/ -name "brightness" 2>/dev/null | head -1)
MAX_BRIGHT_FILE=$(find /sys/class/backlight/ -name "max_brightness" 2>/dev/null | head -1)
if [[ -n "$BRIGHT_FILE" ]]; then
    BRIGHT=$(cat "$BRIGHT_FILE")
    MAX_BRIGHT=$(cat "$MAX_BRIGHT_FILE")
    BRIGHT_PCT=$((BRIGHT * 100 / MAX_BRIGHT))
    echo -e "  💡 Brightness  : ${BRIGHT_PCT}%"
fi

# WiFi
WIFI_DEV=$(iw dev 2>/dev/null | awk '/Interface/ {print $2}' | head -1)
if [[ -n "$WIFI_DEV" ]]; then
    PS=$(iw dev "$WIFI_DEV" get power_save 2>/dev/null | grep -o "on\|off")
    echo -e "  📶 WiFi PS     : ${CYAN}${PS:-N/A}${NC}"
fi

# Bluetooth
BT=$(bluetoothctl show 2>/dev/null | grep "Powered" | awk '{print $2}')
echo -e "  🔵 Bluetooth   : ${CYAN}${BT:-N/A}${NC}"

# ASPM
ASPM=$(cat /sys/module/pcie_aspm/parameters/policy 2>/dev/null | grep -o '\[.*\]' | tr -d '[]')
echo -e "  🔌 ASPM        : ${CYAN}${ASPM:-N/A}${NC}"

echo ""
