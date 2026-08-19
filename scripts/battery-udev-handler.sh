#!/usr/bin/env bash
# ==============================================================================
# battery-udev-handler.sh — Runs as root from udev rule
# Directly applies battery/performance settings
# ==============================================================================

LOG_FILE="/tmp/battery-udev.log"
echo "$(date '+%Y-%m-%d %H:%M:%S') battery-udev-handler called" >> "$LOG_FILE"

# Detect AC status
AC_ONLINE=0
for ps_dir in /sys/class/power_supply/*/online; do
    type_file="$(dirname $ps_dir)/type"
    if [[ -f "$type_file" ]] && grep -q "Mains" "$type_file" 2>/dev/null; then
        AC_ONLINE=$(cat "$ps_dir" 2>/dev/null)
        break
    fi
done

echo "$(date '+%Y-%m-%d %H:%M:%S') AC status: $AC_ONLINE" >> "$LOG_FILE"

# Find the logged-in user for notifications
ACTIVE_USER=$(loginctl list-sessions --no-legend 2>/dev/null | awk '{print $2}' | head -1)
USER_UID=$(id -u "$ACTIVE_USER" 2>/dev/null)
DBUS_ADDR="unix:path=/run/user/${USER_UID}/bus"

notify_user() {
    if [[ -n "$ACTIVE_USER" && -n "$USER_UID" ]]; then
        su - "$ACTIVE_USER" -c "DISPLAY=:0 DBUS_SESSION_BUS_ADDRESS=$DBUS_ADDR notify-send -a 'Tweaks ASUS TUF' -i 'com.rezkycodes.AsusTufCpu' -u normal '⚡ Tweaks ASUS TUF' '$1'" 2>/dev/null || true
    fi
}

if [[ "$AC_ONLINE" == "1" ]]; then
    echo "$(date '+%Y-%m-%d %H:%M:%S') → Performance mode" >> "$LOG_FILE"

    # Performance mode
    for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo "performance" > "$cpu" 2>/dev/null; done
    for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_max_freq; do echo "4300000" > "$cpu" 2>/dev/null; done
    for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_min_freq; do echo "1400000" > "$cpu" 2>/dev/null; done
    [[ -f /sys/devices/system/cpu/cpufreq/boost ]] && echo "1" > /sys/devices/system/cpu/cpufreq/boost
    for cpu in /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference; do echo "performance" > "$cpu" 2>/dev/null; done
    [[ -f /sys/firmware/acpi/platform_profile ]] && echo "performance" > /sys/firmware/acpi/platform_profile
    NVIDIA_PCI="/sys/bus/pci/devices/0000:01:00.0"
    [[ -d "$NVIDIA_PCI" ]] && echo "on" > "$NVIDIA_PCI/power/control"
    [[ -f /sys/module/pcie_aspm/parameters/policy ]] && echo "default" > /sys/module/pcie_aspm/parameters/policy
    [[ -b /dev/sda ]] && hdparm -B 128 -S 0 /dev/sda 2>/dev/null
    for host in /sys/class/scsi_host/host*/link_power_management_policy; do echo "max_performance" > "$host" 2>/dev/null; done
    WIFI_DEV=$(iw dev 2>/dev/null | awk '/Interface/ {print $2}' | head -1)
    [[ -n "$WIFI_DEV" ]] && iw dev "$WIFI_DEV" set power_save off 2>/dev/null
    for usb in /sys/bus/usb/devices/*/power/control; do echo "on" > "$usb" 2>/dev/null; done
    bluetoothctl power on 2>/dev/null

    notify_user "⚡ Performance Mode — AC connected"

else
    echo "$(date '+%Y-%m-%d %H:%M:%S') → Battery save mode" >> "$LOG_FILE"

    # Battery save mode
    for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo "powersave" > "$cpu" 2>/dev/null; done
    for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_max_freq; do echo "2900000" > "$cpu" 2>/dev/null; done
    for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_min_freq; do echo "1400000" > "$cpu" 2>/dev/null; done
    [[ -f /sys/devices/system/cpu/cpufreq/boost ]] && echo "0" > /sys/devices/system/cpu/cpufreq/boost
    for cpu in /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference; do echo "power" > "$cpu" 2>/dev/null; done
    [[ -f /sys/firmware/acpi/platform_profile ]] && echo "quiet" > /sys/firmware/acpi/platform_profile
    NVIDIA_PCI="/sys/bus/pci/devices/0000:01:00.0"
    [[ -d "$NVIDIA_PCI" ]] && echo "auto" > "$NVIDIA_PCI/power/control"
    [[ -f /sys/module/pcie_aspm/parameters/policy ]] && echo "powersave" > /sys/module/pcie_aspm/parameters/policy
    [[ -b /dev/sda ]] && hdparm -B 64 -S 60 /dev/sda 2>/dev/null
    for host in /sys/class/scsi_host/host*/link_power_management_policy; do echo "med_power_with_dipm" > "$host" 2>/dev/null; done
    WIFI_DEV=$(iw dev 2>/dev/null | awk '/Interface/ {print $2}' | head -1)
    [[ -n "$WIFI_DEV" ]] && iw dev "$WIFI_DEV" set power_save on 2>/dev/null
    for usb in /sys/bus/usb/devices/*/power/control; do [ -f "$(dirname $usb)/idVendor" ] && [ "$(cat $(dirname $usb)/idVendor 2>/dev/null)" = "046d" ] && continue; echo "auto" > "$usb" 2>/dev/null; done; for dev in /sys/bus/usb/devices/*; do [ -f "$dev/idVendor" ] && [ "$(cat $dev/idVendor 2>/dev/null)" = "046d" ] && echo "on" > "$dev/power/control" 2>/dev/null; done
    for nvme in /sys/class/nvme/nvme*/power/control; do echo "auto" > "$nvme" 2>/dev/null; done
    echo "1500" > /proc/sys/vm/dirty_writeback_centisecs 2>/dev/null
    echo "3000" > /proc/sys/vm/dirty_expire_centisecs 2>/dev/null
    bluetoothctl power off 2>/dev/null

    # Brightness to 35%
    BRIGHT_FILE=$(find /sys/class/backlight/ -name "brightness" 2>/dev/null | head -1)
    MAX_BRIGHT_FILE=$(find /sys/class/backlight/ -name "max_brightness" 2>/dev/null | head -1)
    if [[ -n "$BRIGHT_FILE" && -n "$MAX_BRIGHT_FILE" ]]; then
        MAX=$(cat "$MAX_BRIGHT_FILE")
        NEW=$((MAX * 35 / 100))
        echo "$NEW" > "$BRIGHT_FILE" 2>/dev/null
    fi

    notify_user "🍃 Battery Save Mode — Running on battery"
fi

echo "$(date '+%Y-%m-%d %H:%M:%S') Done" >> "$LOG_FILE"
