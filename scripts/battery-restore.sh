#!/usr/bin/env bash
# ==============================================================================
# battery-restore.sh — Restore performance mode after battery-save.sh
# ASUS Laptop | AMD Ryzen 7 4800H + GTX 1660 Ti | Fedora 44
# ==============================================================================
# Usage: sudo ./battery-restore.sh
# ==============================================================================

set -euo pipefail

GREEN='\033[0;32m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${GREEN}[✓]${NC} $*"; }
info() { echo -e "${CYAN}[i]${NC} $*"; }

if [[ $EUID -ne 0 ]]; then
    echo "Butuh sudo. Jalankan: sudo $0"
    exit 1
fi

echo ""
echo -e "${CYAN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║       ⚡ Performance Mode — Restoring...    ║${NC}"
echo -e "${CYAN}╚══════════════════════════════════════════════╝${NC}"
echo ""

# 1. CPU Governor → performance
for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
    echo "performance" > "$cpu" 2>/dev/null || true
done
log "CPU governor → performance"

# 2. CPU max freq → full speed (4.3 GHz boost)
for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_max_freq; do
    echo "4300000" > "$cpu" 2>/dev/null || true
done
log "CPU max freq → 4300 MHz (full boost)"

# 3. CPU min freq → normal
for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_min_freq; do
    echo "1400000" > "$cpu" 2>/dev/null || true
done

# 4. Re-enable turbo boost
if [[ -f /sys/devices/system/cpu/cpufreq/boost ]]; then
    echo "1" > /sys/devices/system/cpu/cpufreq/boost
    log "Turbo boost → enabled"
fi

# 5. EPP → performance
for cpu in /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference; do
    echo "performance" > "$cpu" 2>/dev/null || true
done
log "EPP → performance"

# 6. ASUS Platform Profile → performance
if [[ -f /sys/firmware/acpi/platform_profile ]]; then
    echo "performance" > /sys/firmware/acpi/platform_profile 2>/dev/null && \
        log "Platform profile → performance"
fi

# 7. NVIDIA GPU → always on
NVIDIA_PCI="/sys/bus/pci/devices/0000:01:00.0"
if [[ -d "$NVIDIA_PCI" ]]; then
    echo "on" > "$NVIDIA_PCI/power/control" 2>/dev/null || true
    log "NVIDIA GPU → always on"
fi

# 8. PCIe ASPM → default
if [[ -f /sys/module/pcie_aspm/parameters/policy ]]; then
    echo "default" > /sys/module/pcie_aspm/parameters/policy 2>/dev/null && \
        log "PCIe ASPM → default"
fi

# 9. HDD → normal
if [[ -b /dev/sda ]] && command -v hdparm &>/dev/null; then
    hdparm -B 128 -S 0 /dev/sda 2>/dev/null && \
        log "HDD → APM 128, no spindown" || true
fi

# 10. SATA → normal
for host in /sys/class/scsi_host/host*/link_power_management_policy; do
    echo "max_performance" > "$host" 2>/dev/null || true
done
log "SATA link → max_performance"

# 11. WiFi → normal
WIFI_DEV=$(iw dev 2>/dev/null | awk '/Interface/ {print $2}' | head -1)
if [[ -n "$WIFI_DEV" ]]; then
    iw dev "$WIFI_DEV" set power_save off 2>/dev/null && \
        log "WiFi → power save OFF" || true
fi

# 12. USB → no autosuspend
for usb_dev in /sys/bus/usb/devices/*/power/control; do
    echo "on" > "$usb_dev" 2>/dev/null || true
done
log "USB → always on"

# 13. VM → normal
echo "300" > /proc/sys/vm/dirty_writeback_centisecs 2>/dev/null || true
echo "3000" > /proc/sys/vm/dirty_expire_centisecs 2>/dev/null || true
echo "60" > /proc/sys/vm/swappiness 2>/dev/null || true
log "VM → defaults"

# 14. Bluetooth → on
if command -v bluetoothctl &>/dev/null; then
    bluetoothctl power on 2>/dev/null && log "Bluetooth → ON" || true
fi

echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║       ✅ Performance Mode AKTIF             ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════╝${NC}"
echo ""
