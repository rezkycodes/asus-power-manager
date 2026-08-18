#!/usr/bin/env bash
# ==============================================================================
# battery-on-ac.sh — Runs when AC connected (plugged in)
# Restores performance mode
# Called by udev rule (runs as root)
# ==============================================================================

LOG="/tmp/battery-mode.log"
echo "$(date) → Performance mode" >> "$LOG"

# 1. Switch tuned profile
/usr/bin/tuned-adm profile balanced 2>/dev/null

# 2. CPU
for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo "performance" > "$cpu" 2>/dev/null; done
for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_max_freq; do echo "4300000" > "$cpu" 2>/dev/null; done
for b in /sys/devices/system/cpu/cpufreq/boost /sys/devices/system/cpu/amd_pstate/cpb /sys/devices/system/cpu/cpu*/cpufreq/boost /sys/devices/system/cpu/cpufreq/policy*/boost; do [ -f "$b" ] && echo "1" > "$b" 2>/dev/null || true; done

# 3. ASUS platform
[[ -f /sys/firmware/acpi/platform_profile ]] && echo "balanced" > /sys/firmware/acpi/platform_profile

# 4. NVIDIA always on
[[ -d /sys/bus/pci/devices/0000:01:00.0 ]] && echo "on" > /sys/bus/pci/devices/0000:01:00.0/power/control 2>/dev/null

# 5. PCIe ASPM
echo "default" > /sys/module/pcie_aspm/parameters/policy 2>/dev/null

# 6. HDD
[[ -b /dev/sda ]] && /usr/sbin/hdparm -B 128 -S 0 /dev/sda 2>/dev/null

# 7. SATA
for host in /sys/class/scsi_host/host*/link_power_management_policy; do echo "max_performance" > "$host" 2>/dev/null; done

# 8. WiFi
WIFI_DEV=$(iw dev 2>/dev/null | awk '/Interface/ {print $2}' | head -1)
[[ -n "$WIFI_DEV" ]] && iw dev "$WIFI_DEV" set power_save off 2>/dev/null

# 9. USB
for usb in /sys/bus/usb/devices/*/power/control; do echo "on" > "$usb" 2>/dev/null; done

# 10. VM
echo "300" > /proc/sys/vm/dirty_writeback_centisecs 2>/dev/null
echo "3000" > /proc/sys/vm/dirty_expire_centisecs 2>/dev/null

# 11. Bluetooth
/usr/bin/bluetoothctl power on 2>/dev/null

echo "$(date) → Performance mode applied" >> "$LOG"
