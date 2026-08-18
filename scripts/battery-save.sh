#!/usr/bin/env bash
# ==============================================================================
# battery-save.sh — Power saving script for ASUS laptop
# AMD Ryzen 7 4800H + GTX 1660 Ti | Fedora 44
# ==============================================================================
# Usage: sudo ./battery-save.sh [--aggressive]
#   --aggressive : More aggressive savings (lower max CPU freq, deeper ASPM)
#
# To restore performance: sudo ./battery-restore.sh
# ==============================================================================

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

AGGRESSIVE=false
[[ "${1:-}" == "--aggressive" ]] && AGGRESSIVE=true

log()  { echo -e "${GREEN}[✓]${NC} $*"; }
warn() { echo -e "${YELLOW}[!]${NC} $*"; }
info() { echo -e "${CYAN}[i]${NC} $*"; }
err()  { echo -e "${RED}[✗]${NC} $*"; }

# Check root
if [[ $EUID -ne 0 ]]; then
    err "Script ini butuh sudo. Jalankan: sudo $0 $*"
    exit 1
fi

echo ""
echo -e "${CYAN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║       🔋 Battery Saver Mode — ASUS Laptop   ║${NC}"
echo -e "${CYAN}╚══════════════════════════════════════════════╝${NC}"
echo ""

# ─────────────────────────────────────────────
# 1. CPU — Powersave Governor + Limit Frequency
# ─────────────────────────────────────────────
info "CPU: Setting powersave governor..."

AVAILABLE_GOVERNORS=$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors)

if echo "$AVAILABLE_GOVERNORS" | grep -qw "powersave"; then
    for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
        echo "powersave" > "$cpu" 2>/dev/null || true
    done
    log "Governor → powersave"
else
    warn "Governor 'powersave' tidak tersedia, mencoba schedutil..."
    for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
        echo "schedutil" > "$cpu" 2>/dev/null || true
    done
    log "Governor → schedutil"
fi

# Limit max frequency
# Ryzen 7 4800H: 1.4 GHz base, 4.3 GHz boost
# Battery mode: cap at 2.0 GHz (aggressive) or 2.5 GHz (normal)
if $AGGRESSIVE; then
    MAX_FREQ="2000000"  # 2.0 GHz
    info "Aggressive mode: capping CPU at 2.0 GHz"
else
    MAX_FREQ="2500000"  # 2.5 GHz
    info "Normal mode: capping CPU at 2.5 GHz"
fi

for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_max_freq; do
    echo "$MAX_FREQ" > "$cpu" 2>/dev/null || true
done
log "CPU max freq → $((MAX_FREQ / 1000)) MHz"

# Disable turbo boost
for b in /sys/devices/system/cpu/cpufreq/boost /sys/devices/system/cpu/amd_pstate/cpb /sys/devices/system/cpu/cpu*/cpufreq/boost /sys/devices/system/cpu/cpufreq/policy*/boost; do [ -f "$b" ] && echo "0" > "$b" 2>/dev/null || true; done
log "Turbo boost → disabled"

# Set min frequency to lowest
for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_min_freq; do
    echo "1400000" > "$cpu" 2>/dev/null || true
done
log "CPU min freq → 1400 MHz (lowest P-state)"

# ─────────────────────────────────────────────
# 2. AMD P-State / EPP (Energy Performance Preference)
# ─────────────────────────────────────────────
EPP_AVAIL=$(cat /sys/devices/system/cpu/cpu0/cpufreq/energy_performance_available_preferences 2>/dev/null || echo "")
if [[ -n "$EPP_AVAIL" ]]; then
    for cpu in /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference; do
        echo "power" > "$cpu" 2>/dev/null || true
    done
    log "EPP → power"
else
    info "EPP not available (acpi-cpufreq driver)"
fi

# ─────────────────────────────────────────────
# 3. ASUS Platform Profile
# ─────────────────────────────────────────────
if [[ -f /sys/firmware/acpi/platform_profile ]]; then
    CURRENT_PROFILE=$(cat /sys/firmware/acpi/platform_profile)
    if echo "quiet balanced performance" | grep -qw "quiet"; then
        echo "quiet" > /sys/firmware/acpi/platform_profile 2>/dev/null && \
            log "Platform profile → quiet (was: $CURRENT_PROFILE)" || \
            warn "Gagal set platform profile"
    fi
fi

# ─────────────────────────────────────────────
# 4. NVIDIA GPU — Minimize Power
# ─────────────────────────────────────────────
info "NVIDIA GPU: Attempting power reduction..."

# Check if NVIDIA is being actively used by display server
NVIDIA_IN_USE=false
if fuser /dev/nvidia* 2>/dev/null | grep -q '[0-9]'; then
    NVIDIA_IN_USE=true
fi

# Try to enable runtime PM (will suspend GPU when idle)
NVIDIA_PCI="/sys/bus/pci/devices/0000:01:00.0"
if [[ -d "$NVIDIA_PCI" ]]; then
    echo "auto" > "$NVIDIA_PCI/power/control" 2>/dev/null || true
    
    # Set aggressive autosuspend delay (1 second)
    echo "1000" > "$NVIDIA_PCI/power/autosuspend_delay_ms" 2>/dev/null || true
    
    if $NVIDIA_IN_USE; then
        warn "NVIDIA GPU sedang dipakai (gnome-remote-desktop)"
        warn "Runtime PM diaktifkan, GPU akan suspend saat idle"
        info "Tip: Matikan 'Desktop Sharing' di Settings jika tidak perlu"
    else
        log "NVIDIA GPU → runtime PM auto (akan suspend saat idle)"
    fi
    
    # Also try the audio part of NVIDIA
    NVIDIA_HDA="/sys/bus/pci/devices/0000:01:00.1"
    if [[ -d "$NVIDIA_HDA" ]]; then
        echo "auto" > "$NVIDIA_HDA/power/control" 2>/dev/null || true
        log "NVIDIA HDA audio → runtime PM auto"
    fi
fi

# ─────────────────────────────────────────────
# 5. PCIe ASPM (Active State Power Management)
# ─────────────────────────────────────────────
if [[ -f /sys/module/pcie_aspm/parameters/policy ]]; then
    if $AGGRESSIVE; then
        echo "powersupersave" > /sys/module/pcie_aspm/parameters/policy 2>/dev/null && \
            log "PCIe ASPM → powersupersave" || warn "Gagal set ASPM"
    else
        echo "powersave" > /sys/module/pcie_aspm/parameters/policy 2>/dev/null && \
            log "PCIe ASPM → powersave" || warn "Gagal set ASPM"
    fi
fi

# ─────────────────────────────────────────────
# 6. NVMe SSD Power Management
# ─────────────────────────────────────────────
for nvme in /sys/class/nvme/nvme*/power/control; do
    echo "auto" > "$nvme" 2>/dev/null && log "NVMe ($(basename $(dirname $(dirname $nvme)))) → runtime PM auto"
done

# Set NVMe APST (Autonomous Power State Transition) if available
for nvme_ps in /sys/class/nvme/nvme*/power/pm_qos_latency_tolerance_us; do
    echo "200" > "$nvme_ps" 2>/dev/null || true  # Allow deeper sleep states
done

# ─────────────────────────────────────────────
# 7. HDD Power Saving (/dev/sda — ST1000LM035)
# ─────────────────────────────────────────────
if [[ -b /dev/sda ]]; then
    if command -v hdparm &>/dev/null; then
        # APM level: 1-127 (aggressive), 128-254 (less aggressive)
        # 1 = most aggressive, spindown quickly
        if $AGGRESSIVE; then
            hdparm -B 1 -S 12 /dev/sda 2>/dev/null && \
                log "HDD /dev/sda → APM 1 (most aggressive), spindown ~10s" || \
                warn "Gagal set HDD power (butuh hdparm)"
        else
            hdparm -B 64 -S 60 /dev/sda 2>/dev/null && \
                log "HDD /dev/sda → APM 64, spindown ~5 min" || \
                warn "Gagal set HDD power (butuh hdparm)"
        fi
    else
        warn "hdparm tidak terinstall. Install: sudo dnf install hdparm"
    fi
fi

# ─────────────────────────────────────────────
# 8. SATA Link Power Management
# ─────────────────────────────────────────────
for host in /sys/class/scsi_host/host*/link_power_management_policy; do
    if $AGGRESSIVE; then
        echo "min_power" > "$host" 2>/dev/null || true
    else
        echo "med_power_with_dipm" > "$host" 2>/dev/null || true
    fi
done
if $AGGRESSIVE; then
    log "SATA link → min_power"
else
    log "SATA link → med_power_with_dipm"
fi

# ─────────────────────────────────────────────
# 9. WiFi Power Saving
# ─────────────────────────────────────────────
WIFI_DEV=$(iw dev 2>/dev/null | awk '/Interface/ {print $2}' | head -1)
if [[ -n "$WIFI_DEV" ]]; then
    # Enable WiFi power save via iw
    iw dev "$WIFI_DEV" set power_save on 2>/dev/null && \
        log "WiFi ($WIFI_DEV) → power save ON" || \
        warn "Gagal set WiFi power save"
    
    # Also try iwconfig if available
    if command -v iwconfig &>/dev/null; then
        iwconfig "$WIFI_DEV" power on 2>/dev/null || true
        iwconfig "$WIFI_DEV" power timeout 300u 2>/dev/null || true
    fi
else
    warn "WiFi device tidak ditemukan"
fi

# ─────────────────────────────────────────────
# 10. USB Autosuspend
# ─────────────────────────────────────────────
for usb_dev in /sys/bus/usb/devices/*/power/control; do [ -f "$(dirname $usb_dev)/idVendor" ] && [ "$(cat $(dirname $usb_dev)/idVendor 2>/dev/null)" = "046d" ] && continue; echo "auto" > "$usb_dev" 2>/dev/null || true; done

for usb_dev in /sys/bus/usb/devices/*/power/autosuspend; do
    echo "2" > "$usb_dev" 2>/dev/null || true  # 2 seconds
done
log "USB devices → autosuspend (2s)"

# ─────────────────────────────────────────────
# 11. Runtime PM for all PCI devices
# ─────────────────────────────────────────────
for pci_dev in /sys/bus/pci/devices/*/power/control; do
    # Skip NVIDIA (already handled) and essential devices
    echo "auto" > "$pci_dev" 2>/dev/null || true
done
log "PCI devices → runtime PM auto"

# ─────────────────────────────────────────────
# 12. Audio Power Save
# ─────────────────────────────────────────────
if [[ -f /sys/module/snd_hda_intel/parameters/power_save ]]; then
    echo "1" > /sys/module/snd_hda_intel/parameters/power_save 2>/dev/null || true
    log "HDA audio → power save (1s timeout)"
fi

if [[ -f /sys/module/snd_hda_intel/parameters/power_save_controller ]]; then
    echo "Y" > /sys/module/snd_hda_intel/parameters/power_save_controller 2>/dev/null || true
    log "HDA controller → power save ON"
fi

# ─────────────────────────────────────────────
# 13. Screen Brightness (reduce if too high)
# ─────────────────────────────────────────────
BRIGHTNESS_FILE=$(find /sys/class/backlight/ -name "brightness" 2>/dev/null | head -1)
MAX_BRIGHTNESS_FILE=$(find /sys/class/backlight/ -name "max_brightness" 2>/dev/null | head -1)

if [[ -n "$BRIGHTNESS_FILE" && -n "$MAX_BRIGHTNESS_FILE" ]]; then
    CURRENT=$(cat "$BRIGHTNESS_FILE")
    MAX=$(cat "$MAX_BRIGHTNESS_FILE")
    PERCENTAGE=$((CURRENT * 100 / MAX))
    
    if [[ $PERCENTAGE -gt 40 ]]; then
        # Set to 35% brightness
        NEW=$((MAX * 35 / 100))
        echo "$NEW" > "$BRIGHTNESS_FILE" 2>/dev/null && \
            log "Brightness → 35% (was ${PERCENTAGE}%)" || \
            warn "Gagal set brightness (mungkin butuh GNOME settings)"
    else
        info "Brightness sudah rendah (${PERCENTAGE}%)"
    fi
fi

# ─────────────────────────────────────────────
# 14. Kernel VM Tuning
# ─────────────────────────────────────────────
# Reduce dirty page writeback frequency (less disk activity)
echo "1500" > /proc/sys/vm/dirty_writeback_centisecs 2>/dev/null || true
echo "3000" > /proc/sys/vm/dirty_expire_centisecs 2>/dev/null || true
# More aggressive swappiness to keep RAM usage efficient
if $AGGRESSIVE; then
    echo "10" > /proc/sys/vm/swappiness 2>/dev/null || true
    log "VM: swappiness=10, writeback=15s, expire=30s"
else
    echo "30" > /proc/sys/vm/swappiness 2>/dev/null || true
    log "VM: swappiness=30, writeback=15s, expire=30s"
fi

# ─────────────────────────────────────────────
# 15. Disable unnecessary services (optional)
# ─────────────────────────────────────────────
# bluetooth
if command -v bluetoothctl &>/dev/null; then
    bluetoothctl power off 2>/dev/null && log "Bluetooth → OFF" || info "Bluetooth sudah off"
fi

# ─────────────────────────────────────────────
# Summary
# ─────────────────────────────────────────────
echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║       ✅ Battery Save Mode AKTIF            ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════╝${NC}"
echo ""

# Show current battery status
if command -v upower &>/dev/null; then
    BAT_INFO=$(upower -i $(upower -e | grep BAT) 2>/dev/null)
    PERCENT=$(echo "$BAT_INFO" | grep "percentage" | awk '{print $2}')
    STATE=$(echo "$BAT_INFO" | grep "state" | awk '{print $2}')
    TIME_LEFT=$(echo "$BAT_INFO" | grep -E "time to (empty|full)" | awk '{print $4, $5}')
    RATE=$(echo "$BAT_INFO" | grep "energy-rate" | awk '{print $2}')
    
    echo -e "  🔋 Baterai   : ${CYAN}${PERCENT}${NC} (${STATE})"
    [[ -n "$TIME_LEFT" ]] && echo -e "  ⏱️  Estimasi  : ${CYAN}${TIME_LEFT}${NC}"
    [[ -n "$RATE" ]] && echo -e "  ⚡ Konsumsi  : ${CYAN}${RATE} W${NC}"
fi

echo ""
info "Untuk restore performa: sudo ~/bin/battery-restore.sh"
echo ""
