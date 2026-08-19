# Tweaks ASUS TUF

> **A native Rust + GTK4 / Libadwaita system monitor and hardware control app for ASUS TUF Gaming & Linux laptops.**

[![GitHub Release](https://img.shields.io/github/v/release/rezkycodes/asus-power-manager?style=for-the-badge&logo=github&color=blue)](https://github.com/rezkycodes/asus-power-manager/releases/latest)
[![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20GNOME-informational?style=for-the-badge&logo=linux)](https://github.com/rezkycodes/asus-power-manager)
[![Built with Rust](https://img.shields.io/badge/Built%20with-Rust-orange?style=for-the-badge&logo=rust)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg?style=for-the-badge)](https://opensource.org/licenses/MIT)

---

> **v2.0.0 — Rewritten in Rust.** The app was fully migrated from Python to a native Rust/GTK4 binary: lower memory (~50 MB idle vs ~70 MB), instant startup, and Mission Center–style realtime monitors. The Python version is archived under `python-legacy/`.

## ⬇️ Direct Downloads (v2.0.0)

| Package Format | Target Distribution | Direct Download Link | Quick Terminal Install |
| :--- | :--- | :---: | :--- |
| **📦 Debian / Ubuntu (`.deb`)** | Ubuntu, Debian, Pop!_OS, Linux Mint | [**Download .deb**](https://github.com/rezkycodes/asus-power-manager/releases/download/v2.0.0/asus-power-manager_2.0.0_amd64.deb) | `sudo dpkg -i asus-power-manager_2.0.0_amd64.deb` |
| **📦 Fedora / RHEL (`.rpm`)** | Fedora, RHEL, openSUSE, AlmaLinux | [**Download .rpm**](https://github.com/rezkycodes/asus-power-manager/releases/download/v2.0.0/asus-power-manager-2.0.0-1.fc44.x86_64.rpm) | `sudo dnf install ./asus-power-manager-*.rpm` |

👉 **[View Full Release Assets on GitHub](https://github.com/rezkycodes/asus-power-manager/releases/tag/v2.0.0)**

---

## ✨ Features

### 📊 Realtime Monitors (Mission Center–style, auto-detecting)
- **CPU:** per-core usage graphs, temperature, current/base speed, logical processors, virtualization, cache, uptime, process/thread counts.
- **Memory:** usage & swap graphs, in-use/available/committed/cached, plus DIMM hardware details (type, form factor, speed, slots).
- **GPU:** per-GPU utilization & VRAM graphs, clocks, power, temperature, encode/decode, PCIe link (NVIDIA + AMD, auto-detected).
- **Fan:** per-fan speed graphs (auto-detected via hwmon).
- **Network:** per-interface throughput (RX/TX), totals, IP/SSID/signal details.
- **Drive:** per-disk active-time & throughput graphs, read/write speeds, partitions, hotplug auto-detect.
- **Battery:** system + peripheral (mouse) charge/power graphs, health, energy, charge threshold, cycles.

### 🎛️ Hardware Controls
- 🔋 **Battery Health Care (80% charge limit)** — protects the cell during 24/7 plugged-in use.
- ⚡ **CPU profiles** — Powersave / Performance / Auto-switch (AC vs battery).
- 🌀 **Fan profiles** — Silent / Normal / Turbo.
- 🎮 **GPU mode** — Hybrid / AMD iGPU only / NVIDIA dedicated.
- 🌈 **Keyboard RGB** — palette, color picker, RGB sliders, effects, brightness & speed.
- 🖱️ **Logitech G304** — polling rate, DPI (slider + presets), onboard memory.
- 💻 **Clamshell / server mode** — lid closed, display off, CPU & background servers stay active.

### 🧰 Task Manager
- **Applications & Processes:** live table (CPU / Memory / Swap / Drive I/O / listening Port), search, stop / force-kill.
- **All Services:** full systemd user + system unit table with status, filters, start/stop/restart, and a detail modal (unit file + journal).

### 🎨 Design
- Full-black `#000000` monochrome theme, always-visible sidebar, Lucide icons, white realtime graphs.

---

## 📦 Installation

### Ubuntu / Debian / Linux Mint / Pop!_OS
```bash
wget https://github.com/rezkycodes/asus-power-manager/releases/download/v2.0.0/asus-power-manager_2.0.0_amd64.deb
sudo dpkg -i asus-power-manager_2.0.0_amd64.deb
sudo apt-get install -f   # pull GTK4/libadwaita runtime deps if needed
```

### Fedora / RHEL / openSUSE
```bash
sudo dnf install https://github.com/rezkycodes/asus-power-manager/releases/download/v2.0.0/asus-power-manager-2.0.0-1.fc44.x86_64.rpm
```

---

## 🛠️ Building From Source

Requires the Rust toolchain (`cargo`) plus GTK4 + libadwaita development libraries.

```bash
git clone https://github.com/rezkycodes/asus-power-manager.git
cd asus-power-manager

# Run directly
cd rust-gui && cargo run --release

# Or build both .deb and .rpm (cargo build + packaging)
cd .. && ./build-packages.sh
```
Generated packages land in `dist/`:
- `dist/asus-power-manager_2.0.0_amd64.deb`
- `dist/asus-power-manager-2.0.0-1.fc44.x86_64.rpm`

---

## 🖥️ Usage

- **From the app menu:** press `Super` and search for **Tweaks ASUS TUF**.
- **From the terminal:**
  ```bash
  asus-tuf-cpu           # the Rust binary
  asus-power-manager     # backward-compatible symlink
  ```

Hardware control buttons run helper scripts in `/usr/libexec/asus-power-manager/scripts/` via a scoped passwordless sudoers rule.

---

## 📁 Project Layout
- `rust-gui/` — the Rust/GTK4 application (source, `Cargo.toml`, bundled Lucide icons).
- `scripts/` — hardware backend scripts (battery, fan, GPU, RGB, mouse, DIMM).
- `data/` — desktop entry, systemd/udev/sysctl/modprobe/sudoers assets.
- `debian/`, `rpm/` — packaging metadata.
- `python-legacy/` — the archived original Python app.

---

## 📄 License
MIT License © 2026 [Rezky P. Budihartono](https://github.com/rezkycodes).
