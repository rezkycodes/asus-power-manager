# ASUS Power & Battery Manager

> **A modern GTK4 / Libadwaita power management and battery health control utility for ASUS & Linux laptops (Ubuntu, Debian, Fedora, Arch, etc.).**

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20GNOME-informational.svg)
![Packages](https://img.shields.io/badge/packages-.deb%20%7C%20.rpm-success.svg)

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

## 📦 Installation

### 1. Ubuntu / Debian / Linux Mint / Pop!_OS (`.deb`)
Download the latest `.deb` from the Release page or `dist/`:
```bash
sudo dpkg -i asus-power-manager_1.0.0_all.deb
sudo apt-get install -f  # if dependencies needed
```

### 2. Fedora / RHEL / openSUSE (`.rpm`)
Download the latest `.rpm` from the Release page or `dist/`:
```bash
sudo dnf install ./asus-power-manager-1.0.0-1.fc*.noarch.rpm
```

### 3. Install from Source (Any Linux Distro)
```bash
git clone https://github.com/rezkycodes/asus-power-manager.git
cd asus-power-manager
sudo ./install.sh
```

---

## 🛠️ Building Packages (.deb and .rpm)

You can build both packages with a single command:
```bash
./build-packages.sh
```
The generated packages will be placed in the `dist/` folder:
- `dist/asus-power-manager_1.0.0_all.deb`
- `dist/asus-power-manager-1.0.0-1.fc44.noarch.rpm`

---

## 🖥️ Usage

- **Launch from App Menu:** Press `Super` and search for **Power & Battery Manager**.
- **Launch from Terminal:**
  ```bash
  asus-power-manager
  ```

---

## 📄 License
MIT License. Created by [Rezky P. Budihartono](https://github.com/rezkycodes).
