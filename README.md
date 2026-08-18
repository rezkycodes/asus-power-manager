# Tweaks ASUS TUF

> **A modern GTK4 / Libadwaita hardware tweak utility, power manager, and RGB studio for ASUS TUF & Linux laptops.**

[![GitHub Release](https://img.shields.io/github/v/release/rezkycodes/asus-power-manager?style=for-the-badge&logo=github&color=blue)](https://github.com/rezkycodes/asus-power-manager/releases/latest)
[![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20GNOME-informational?style=for-the-badge&logo=linux)](https://github.com/rezkycodes/asus-power-manager)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg?style=for-the-badge)](https://opensource.org/licenses/MIT)

---

## ⬇️ Direct Downloads (v1.0.0)

| Package Format | Target Distribution | Direct Download Link | Quick Terminal Install |
| :--- | :--- | :---: | :--- |
| **📦 Debian / Ubuntu (`.deb`)** | Ubuntu, Debian, Pop!_OS, Linux Mint | [**Download .deb**](https://github.com/rezkycodes/asus-power-manager/releases/download/v1.0.0/asus-power-manager_1.0.0_all.deb) | `sudo dpkg -i asus-power-manager_1.0.0_all.deb` |
| **📦 Fedora / RHEL (`.rpm`)** | Fedora, RHEL, openSUSE, AlmaLinux | [**Download .rpm**](https://github.com/rezkycodes/asus-power-manager/releases/download/v1.0.0/asus-power-manager-1.0.0-1.fc44.noarch.rpm) | `sudo dnf install ./asus-power-manager-*.rpm` |
| **🗜️ Source Archive (`.zip`)** | Generic Linux, Arch, Source Build | [**Download .zip**](https://github.com/rezkycodes/asus-power-manager/releases/download/v1.0.0/asus-power-manager-1.0.0-source.zip) | `unzip asus-power-manager-*.zip && sudo ./install.sh` |

👉 **[View Full Release Assets on GitHub](https://github.com/rezkycodes/asus-power-manager/releases/tag/v1.0.0)**

---

## ✨ Features

- 🔋 **Live Real-time Battery Stats:** Displays battery percentage, power draw in Watts ($W$), health capacity %, and runtime estimate.
- ⚡ **1-Click Performance Profiles:**
  - **Powersave Mode:** Throttles CPU to 1.7–2.5 GHz, disables Turbo Boost, quiet fan profile.
  - **Performance Mode:** Unlocks up to 4.3 GHz, enables Turbo Boost, balanced/performance fan curve.
  - **Auto-Switching Mode:** Automatically switches between Performance (on AC charger) and Powersave (on battery).
- 🛡️ **Battery Health Care (80% Charge Limit):** Prevents battery degradation & swelling during 24/7 plugged-in usage.
- 💻 **Clamshell / Server Mode:** Allows closing laptop lid without sleep; display turns off while CPU, Wi-Fi, and background servers remain active 100%.
- 🚀 **Hardware Crash & Freeze Hardening:**
  - AMD Ryzen C6 Voltage Droop fix (`processor.max_cstate=5 idle=nomwait`).
  - NVMe APST timeout fix (`nvme_core.default_ps_max_latency_us=0`).
  - NVIDIA VRAM & PCIe D3hot stability (`NVreg_PreserveVideoMemoryAllocations=1`).
  - Logitech Lightspeed USB autosuspend prevention (zero cursor lag).

---

## 📦 Installation Guide

### 1. Ubuntu / Debian / Linux Mint / Pop!_OS
```bash
# 1. Download .deb package
wget https://github.com/rezkycodes/asus-power-manager/releases/download/v1.0.0/asus-power-manager_1.0.0_all.deb

# 2. Install
sudo dpkg -i asus-power-manager_1.0.0_all.deb
sudo apt-get install -f
```

### 2. Fedora / RHEL / openSUSE
```bash
# 1. Download & install .rpm package
sudo dnf install https://github.com/rezkycodes/asus-power-manager/releases/download/v1.0.0/asus-power-manager-1.0.0-1.fc44.noarch.rpm
```

### 3. Install from Source (.zip or git)
```bash
# Download and extract source
wget https://github.com/rezkycodes/asus-power-manager/releases/download/v1.0.0/asus-power-manager-1.0.0-source.zip
unzip asus-power-manager-1.0.0-source.zip
cd asus-power-manager-1.0.0

# Install
sudo ./install.sh
```

---

## 🛠️ Building Packages Locally

You can build both `.deb` and `.rpm` packages with a single script:
```bash
./build-packages.sh
```
The generated packages will be placed in the `dist/` directory:
- `dist/asus-power-manager_1.0.0_all.deb`
- `dist/asus-power-manager-1.0.0-1.fc44.noarch.rpm`
- `dist/asus-power-manager-1.0.0-source.zip`

---

## 🖥️ Usage

- **From Application Menu:** Press `Super` and search for **Power & Battery Manager**.
- **From Terminal:**
  ```bash
  asus-power-manager
  ```
- **CLI Battery Status:**
  ```bash
  battery-status.sh
  ```

---

## 📄 License
MIT License © 2026 [Rezky P. Budihartono](https://github.com/rezkycodes).
