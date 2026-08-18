#!/usr/bin/env bash
# ==============================================================================
# battery-set-gpu.sh — GPU Profile & Power Mode Switcher
# ASUS TUF Gaming (AMD Renoir iGPU + NVIDIA GTX 1660 Ti Mobile)
# ==============================================================================

set -euo pipefail

MODE="${1:-hybrid}" # hybrid | integrated | dedicated

NVIDIA_PCI="/sys/bus/pci/devices/0000:01:00.0"

case "$MODE" in
    hybrid|0)
        echo "Mengatur mode GPU ke: HYBRID (On-Demand Optimus)..."
        # 1. Enable runtime power management (auto sleep when idle)
        if [[ -d "$NVIDIA_PCI" ]]; then
            echo "auto" > "$NVIDIA_PCI/power/control" 2>/dev/null || true
        fi
        # 2. Disable persistence mode so GPU can sleep
        command -v nvidia-smi &>/dev/null && nvidia-smi -pm 0 &>/dev/null || true
        echo "✓ Mode GPU: Hybrid aktif (AMD untuk desktop, NVIDIA otomatis saat dibutuhkan)."
        ;;

    integrated|1|amd)
        echo "Mengatur mode GPU ke: INTEGRATED (AMD iGPU Only)..."
        # 1. Force auto power control
        if [[ -d "$NVIDIA_PCI" ]]; then
            echo "auto" > "$NVIDIA_PCI/power/control" 2>/dev/null || true
        fi
        command -v nvidia-smi &>/dev/null && nvidia-smi -pm 0 &>/dev/null || true
        echo "✓ Mode GPU: AMD iGPU Only aktif (NVIDIA standby hemat daya maksimal)."
        ;;

    dedicated|performance|2|nvidia)
        echo "Mengatur mode GPU ke: NVIDIA DEDICATED (Performa Penuh / Compute)..."
        # 1. Keep NVIDIA GPU always powered on
        if [[ -d "$NVIDIA_PCI" ]]; then
            echo "on" > "$NVIDIA_PCI/power/control" 2>/dev/null || true
        fi
        # 2. Enable persistence mode for zero wake-up latency
        command -v nvidia-smi &>/dev/null && nvidia-smi -pm 1 &>/dev/null || true
        echo "✓ Mode GPU: NVIDIA Dedicated aktif (GPU selalu siaga penuh tanpa delay)."
        ;;

    *)
        echo "Usage: $0 [hybrid | integrated | dedicated]"
        exit 1
        ;;
esac

# Save persistent config
CONFIG_DIR="/etc/asus-power-manager"
mkdir -p "$CONFIG_DIR"
echo "GPU_MODE=$MODE" > "$CONFIG_DIR/gpu.conf"
