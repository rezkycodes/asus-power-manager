#!/usr/bin/env bash
# ==============================================================================
# battery-on-battery.sh — Runs when AC disconnected (on battery power)
# Combines tuned profile + custom optimizations
# Called by udev rule (runs as root)
# ==============================================================================

LOG="/tmp/battery-mode.log"
echo "$(date) → Battery save mode" >> "$LOG"

# 1. Switch tuned profile
/usr/bin/tuned-adm profile powersave 2>/dev/null

# 2. CPU
for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo "powersave" > "$cpu" 2>/dev/null; done
for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_max_freq; do echo "2500000" > "$cpu" 2>/dev/null; done
for b in /sys/devices/system/cpu/cpufreq/boost /sys/devices/system/cpu/amd_pstate/cpb /sys/devices/system/cpu/cpu*/cpufreq/boost /sys/devices/system/cpu/cpufreq/policy*/boost; do [ -f "$b" ] && echo "0" > "$b" 2>/dev/null || true; done

# 3. ASUS platform
[[ -f /sys/firmware/acpi/platform_profile ]] && echo "quiet" > /sys/firmware/acpi/platform_profile

# 4. NVIDIA runtime PM
[[ -d /sys/bus/pci/devices/0000:01:00.0 ]] && echo "auto" > /sys/bus/pci/devices/0000:01:00.0/power/control 2>/dev/null
[[ -d /sys/bus/pci/devices/0000:01:00.1 ]] && echo "auto" > /sys/bus/pci/devices/0000:01:00.1/power/control 2>/dev/null

# 5. PCIe ASPM
echo "powersave" > /sys/module/pcie_aspm/parameters/policy 2>/dev/null

# 6. HDD
[[ -b /dev/sda ]] && /usr/sbin/hdparm -B 64 -S 60 /dev/sda 2>/dev/null

# 7. SATA
for host in /sys/class/scsi_host/host*/link_power_management_policy; do echo "med_power_with_dipm" > "$host" 2>/dev/null; done

# 8. NVMe
for nvme in /sys/class/nvme/nvme*/power/control; do echo "auto" > "$nvme" 2>/dev/null; done

# 9. WiFi
WIFI_DEV=$(iw dev 2>/dev/null | awk '/Interface/ {print $2}' | head -1)
[[ -n "$WIFI_DEV" ]] && iw dev "$WIFI_DEV" set power_save on 2>/dev/null

# 10. USB
for usb in /sys/bus/usb/devices/*/power/control; do [ -f "$(dirname $usb)/idVendor" ] && [ "$(cat $(dirname $usb)/idVendor 2>/dev/null)" = "046d" ] && continue; echo "auto" > "$usb" 2>/dev/null; done; for dev in /sys/bus/usb/devices/*; do [ -f "$dev/idVendor" ] && [ "$(cat $dev/idVendor 2>/dev/null)" = "046d" ] && echo "on" > "$dev/power/control" 2>/dev/null; done

# 11. Audio
echo "1" > /sys/module/snd_hda_intel/parameters/power_save 2>/dev/null
echo "Y" > /sys/module/snd_hda_intel/parameters/power_save_controller 2>/dev/null

# 12. VM
echo "1500" > /proc/sys/vm/dirty_writeback_centisecs 2>/dev/null
echo "3000" > /proc/sys/vm/dirty_expire_centisecs 2>/dev/null

# 13. Bluetooth
/usr/bin/bluetoothctl power off 2>/dev/null

# 14. Brightness (35%)
BRIGHT_FILE=$(find /sys/class/backlight/ -name "brightness" 2>/dev/null | head -1)
MAX_FILE=$(find /sys/class/backlight/ -name "max_brightness" 2>/dev/null | head -1)
if [[ -n "$BRIGHT_FILE" && -n "$MAX_FILE" ]]; then
    MAX=$(cat "$MAX_FILE")
    echo $((MAX * 35 / 100)) > "$BRIGHT_FILE" 2>/dev/null
fi

echo "$(date) → Battery save mode applied" >> "$LOG"
