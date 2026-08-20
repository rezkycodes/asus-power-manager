// asus-tuf-cpu — Tweaks ASUS TUF (Rust/GTK4 reimplementation, MIT).
// Clean-room: uses standard Linux sysfs/procfs + the project's own bash scripts.
// Tabs: CPU (realtime graphs) and Daya & Baterai (power/GPU/fan controls).

use adw::prelude::*;
use gtk::glib;

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

const HISTORY: usize = 60;

#[derive(Default, Clone)]
struct PartInfo {
    name: String,   // e.g. "nvme0n1p1"
    fstype: String, // e.g. "btrfs"
    mount: String,  // mountpoint or ""
    used: u64,      // bytes (0 if unknown)
    size: u64,      // bytes
}

#[derive(Default, Clone, Copy)]
struct DiskPrev {
    reads: u64,
    sect_read: u64,
    ms_read: u64,
    writes: u64,
    sect_written: u64,
    ms_write: u64,
    ms_io: u64,
}

#[derive(Default, Clone)]
struct DriveInfo {
    dev: String,       // "nvme0n1"
    kind: String,      // "NVMe" / "SSD" / "HDD"
    model: String,
    serial: String,
    wwn: String,
    capacity: u64,     // bytes
    formatted: u64,    // bytes (sum of partition sizes)
    is_system: bool,
    rotational: bool,
    // live
    read_bps: f64,
    write_bps: f64,
    total_read: u64,
    total_written: u64,
    active_pct: f64,
    resp_ms: f64,
    active_hist: VecDeque<f64>, // percent 0-100
    thru_hist: VecDeque<f64>,   // bytes/s (read+write)
    partitions: Vec<PartInfo>,
    prev: Option<DiskPrev>,
}

#[derive(Default, Clone)]
struct GpuInfo {
    kind: String, // "NVIDIA"/"AMD"/"Intel"
    name: String,
    bus: String,
    nvidia_index: Option<u32>,
    drm_path: String, // sysfs device path for AMD/Intel
    // live
    util: f64,
    mem_used: u64,
    mem_total: u64,
    temp: f64,
    power_draw: f64,
    power_limit: f64,
    clock_cur: f64,     // MHz
    clock_max: f64,     // MHz
    mem_clock_cur: f64, // MHz
    mem_clock_max: f64, // MHz
    pcie: String,
    enc_util: f64,
    dec_util: f64,
    util_hist: VecDeque<f64>,
    mem_hist: VecDeque<f64>, // percent of VRAM
}

#[derive(Default, Clone)]
struct FanInfo {
    label: String, // e.g. "cpu_fan"
    path: String,  // fanN_input path
    key: String,   // stable id "hwmonX:fanN"
    rpm: f64,
    hist: VecDeque<f64>,
}

#[derive(Default, Clone)]
struct NetInfo {
    iface: String,
    kind: String,  // "Wired"/"Wireless"/"Other"
    model: String,
    mac: String,
    is_wireless: bool,
    // live
    rx_bps: f64,
    tx_bps: f64,
    total_rx: u64,
    total_tx: u64,
    status: String,
    ipv4: String,
    ipv6: String,
    ssid: String,
    signal: String,
    freq: String,
    rx_hist: VecDeque<f64>,
    tx_hist: VecDeque<f64>,
    prev: Option<(u64, u64)>,
}

#[derive(Default, Clone)]
struct TsPeer {
    name: String,
    ip: String,
    os: String,
    online: bool,
    is_self: bool,
}

#[derive(Default, Clone)]
struct BatInfo {
    name: String,
    model: String,
    path: String,
    is_system: bool,
    // live
    percent: f64,
    voltage: f64,
    power: f64,
    state: String,
    cycles: i64,
    serial: String,
    technology: String,
    capacity_health: f64,
    energy_full: f64,
    energy_full_design: f64,
    voltage_min_design: f64,
    charge_threshold: String,
    pct_hist: VecDeque<f64>,
    power_hist: VecDeque<f64>,
}

#[derive(Default, Clone)]
struct DiskSmartInfo {
    #[allow(dead_code)]
    dev: String,
    health: String,
    temp: String,
    power_on_hours: String,
    power_cycles: String,
    reallocated: String,
    percent_used: String,
    data_written: String,
    model: String,
    smartctl_missing: bool,
}

#[derive(Default, Clone)]
struct ProcInfo {
    pid: u32,
    name: String,
    cpu: f64,
    rss_kb: u64,
    swap_kb: u64,
    io_bps: f64,
    io_known: bool,
    ports: String,
}

#[derive(Default, Clone)]
struct SvcUnit {
    unit: String,
    is_user: bool,
    active: String, // active/inactive/failed
    sub: String,    // running/dead/exited/failed/listening/mounted
    mem: u64, // bytes
}

#[derive(Default)]
struct Shared {
    ready: bool,
    // ── CPU ────────────────────────────────
    logical: usize,
    per_core: Vec<VecDeque<f64>>,
    temp_hist: VecDeque<f64>,
    overall: u32,
    speed_ghz: String,
    temp: i32,
    governor: String,
    driver: String,
    processes: u64,
    threads: u64,
    handles: String,
    uptime: String,
    freq_mhz: u64,
    boost: String,
    profile: String,
    model: String,
    base_ghz: String,
    sockets: String,
    logical_str: String,
    virt: String,
    vm: String,
    l1: String,
    l2: String,
    l3: String,
    // ── Power / Battery ────────────────────
    ac_online: bool,
    bat_cap: String,
    bat_status: String,
    threshold: String,
    energy_rate: String,
    health_cap: String,
    time_str: String,
    // ── GPU ────────────────────────────────
    gpu_tel: String,
    gpu_mode: String,
    // ── Fan ────────────────────────────────
    fan1: String,
    fan2: String,
    fan_policy: String,
    // ── CPU power mode ─────────────────────
    power_mode: String,
    // ── Mouse Logitech ─────────────────────
    m_bat: String,
    m_status: String,
    m_hz: String,
    m_dpi: String,
    m_onboard: bool,
    m_led_mode: String,
    m_led_color: String,
    m_led_period: String,
    m_led_intensity: String,
    // ── Memory (kB, f64) ──
    mem_total: f64,
    mem_used: f64,
    mem_avail: f64,
    mem_cached: f64,
    swap_total: f64,
    swap_used: f64,
    committed: f64,
    mem_pct: u32,
    mem_hist: VecDeque<f64>,
    swap_hist: VecDeque<f64>,
    // DIMM hardware (from dmidecode, fetched once at startup)
    dimm_type: String,
    dimm_form: String,
    dimm_speed: String,
    dimm_slots: String,
    // ── Drives (per physical disk) ──
    drives: Vec<DriveInfo>,
    // ── Disk S.M.A.R.T. health ──
    smart_data: Vec<DiskSmartInfo>,
    // ── GPUs (per adapter) ──
    gpus: Vec<GpuInfo>,
    // ── Fans (per hwmon fan input) ──
    fans: Vec<FanInfo>,
    // ── Network interfaces ──
    nets: Vec<NetInfo>,
    // ── Tailscale tailnet devices (name + IP) ──
    ts_peers: Vec<TsPeer>,
    ts_running: bool,
    // ── Batteries (system + peripherals) ──
    bats: Vec<BatInfo>,
    // ── Processes (task manager) ──
    procs: Vec<ProcInfo>,
    proc_total: usize,
    // ── All systemd units (full services view) ──
    svc_all: Vec<SvcUnit>,
    // ── Systemd services (key "user:unit"/"sys:unit" -> state) ──
    services: std::collections::HashMap<String, String>,
    // ── Adaptive sampling (pause heavy work for hidden tabs) ──
    visible_tab: String,
    pause_hidden: bool,
    force_heavy: bool,
    // ── Temperature alerts ──
    alerts_enabled: bool,
    alert_cooldowns: std::collections::HashMap<String, Instant>,
}

// ───────────────────────── helpers ─────────────────────────
fn rd(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}
fn rd_u64(path: &str) -> Option<u64> {
    rd(path)?.parse().ok()
}
fn read_kv(path: &str, key: &str) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    for line in s.lines() {
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn read_stat() -> Option<((u64, u64), Vec<(u64, u64)>)> {
    let s = fs::read_to_string("/proc/stat").ok()?;
    let mut overall = (0u64, 0u64);
    let mut per = Vec::new();
    for line in s.lines() {
        if !line.starts_with("cpu") {
            continue;
        }
        let mut it = line.split_whitespace();
        let key = it.next()?;
        let vals: Vec<u64> = it.filter_map(|x| x.parse().ok()).collect();
        if vals.len() < 4 {
            continue;
        }
        let idle = vals[3] + vals.get(4).copied().unwrap_or(0);
        let total: u64 = vals.iter().sum();
        if key == "cpu" {
            overall = (idle, total);
        } else {
            per.push((idle, total));
        }
    }
    Some((overall, per))
}
fn pct(prev: (u64, u64), cur: (u64, u64)) -> f64 {
    let dt = cur.1.saturating_sub(prev.1) as f64;
    let di = cur.0.saturating_sub(prev.0) as f64;
    if dt <= 0.0 {
        0.0
    } else {
        (1.0 - di / dt).clamp(0.0, 1.0) * 100.0
    }
}
fn avg_speed_ghz(logical: usize) -> Option<(String, u64)> {
    let mut sum = 0u64;
    let mut n = 0u64;
    for i in 0..logical {
        if let Some(v) =
            rd_u64(&format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq"))
        {
            sum += v;
            n += 1;
        }
    }
    if n > 0 {
        let avg = sum / n;
        Some((format!("{:.2}", avg as f64 / 1e6), avg / 1000))
    } else {
        None
    }
}
fn find_temp_path() -> Option<String> {
    for entry in fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let p = entry.path();
        let name = fs::read_to_string(p.join("name")).unwrap_or_default();
        match name.trim() {
            "k10temp" | "coretemp" | "zenpower" => {
                let cand = p.join("temp1_input");
                if cand.exists() {
                    return Some(cand.to_string_lossy().into_owned());
                }
            }
            _ => {}
        }
    }
    None
}
fn fan_input(which: &str) -> Option<String> {
    let base = "/sys/devices/platform/asus-nb-wmi/hwmon";
    for entry in fs::read_dir(base).ok()?.flatten() {
        let cand = entry.path().join(which);
        if let Some(v) = rd(cand.to_str()?) {
            return Some(v);
        }
    }
    None
}
fn fmt_uptime(secs: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        secs / 86400,
        (secs % 86400) / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}
fn count_procs_threads() -> (u64, u64) {
    let (mut procs, mut threads) = (0u64, 0u64);
    if let Ok(rd) = fs::read_dir("/proc") {
        for e in rd.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
                procs += 1;
                if let Ok(t) = fs::read_dir(format!("/proc/{}/task", n)) {
                    threads += t.flatten().count() as u64;
                }
            }
        }
    }
    (procs, threads)
}
fn ac_online() -> bool {
    if let Ok(rd) = fs::read_dir("/sys/class/power_supply") {
        for e in rd.flatten() {
            let p = e.path();
            if fs::read_to_string(p.join("type")).map(|t| t.trim() == "Mains").unwrap_or(false) {
                if let Ok(o) = fs::read_to_string(p.join("online")) {
                    return o.trim() == "1";
                }
            }
        }
    }
    false
}
fn bat_dir() -> Option<PathBuf> {
    for e in fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let name = e.file_name();
        if name.to_string_lossy().starts_with("BAT") {
            return Some(e.path());
        }
    }
    None
}
fn upower_battery() -> (String, String, String) {
    // (energy_rate, capacity, time_str)
    let (mut er, mut cap, mut ts) = (String::new(), String::new(), String::new());
    if let Ok(out) = Command::new("sh")
        .arg("-c")
        .arg("upower -e | grep BAT | head -1")
        .output()
    {
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !path.is_empty() {
            if let Ok(info) = Command::new("upower").arg("-i").arg(&path).output() {
                for line in String::from_utf8_lossy(&info.stdout).lines() {
                    let l = line.trim();
                    if let Some(v) = l.strip_prefix("energy-rate:") {
                        er = v.trim().to_string();
                    } else if let Some(v) = l.strip_prefix("capacity:") {
                        cap = v.trim().to_string();
                    } else if l.contains("time to") {
                        if let Some((_, v)) = l.split_once(':') {
                            ts = v.trim().to_string();
                        }
                    }
                }
            }
        }
    }
    (er, cap, ts)
}
fn nvidia_telemetry() -> Option<(String, String, String, String)> {
    // (temp°C, powerW, vram, pstate)
    let out = Command::new("nvidia-smi")
        .args([
            "--query-gpu=temperature.gpu,power.draw,memory.used,memory.total,pstate",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let parts: Vec<String> = text.trim().split(',').map(|s| s.trim().to_string()).collect();
    if parts.len() >= 5 {
        let pwr = parts[1].parse::<f64>().map(|p| format!("{:.1}", p)).unwrap_or(parts[1].clone());
        Some((
            format!("{}°C", parts[0]),
            format!("{} W", pwr),
            format!("{}/{} MB", parts[2], parts[3]),
            parts[4].clone(),
        ))
    } else {
        None
    }
}

fn gather_static(sh: &Arc<Mutex<Shared>>, logical: usize) {
    let mut model = String::new();
    if let Ok(s) = fs::read_to_string("/proc/cpuinfo") {
        for line in s.lines() {
            if line.to_lowercase().starts_with("model name") {
                if let Some((_, v)) = line.split_once(':') {
                    model = v.trim().to_string();
                }
                break;
            }
        }
    }
    let base_ghz = rd_u64("/sys/devices/system/cpu/cpu0/cpufreq/base_frequency")
        .or_else(|| rd_u64("/sys/devices/system/cpu/cpu0/cpufreq/bios_limit"))
        .map(|khz| format!("{:.2}", khz as f64 / 1e6))
        .unwrap_or_default();
    let (mut sockets, mut virt, mut vm) = ("1".to_string(), "—".to_string(), "No".to_string());
    let (mut l1d, mut l1i, mut l2, mut l3) =
        (String::new(), String::new(), String::new(), String::new());
    if let Ok(out) = Command::new("lscpu").output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some((k, v)) = line.split_once(':') {
                let (k, v) = (k.trim(), v.trim());
                match k {
                    "Model name" if model.is_empty() => model = v.to_string(),
                    "Socket(s)" => sockets = v.to_string(),
                    "Virtualization" => virt = v.to_string(),
                    "Hypervisor vendor" => vm = "Yes".to_string(),
                    "L1d cache" => l1d = v.to_string(),
                    "L1i cache" => l1i = v.to_string(),
                    "L2 cache" => l2 = v.to_string(),
                    "L3 cache" => l3 = v.to_string(),
                    _ => {}
                }
            }
        }
    }
    if model.is_empty() {
        model = "CPU".into();
    }
    // DIMM/memory hardware via the privileged helper (dmidecode needs root).
    // Runs once; sudo -n succeeds through the sudoers libexec wildcard.
    let (mut dimm_type, mut dimm_form, mut dimm_speed, mut dimm_slots) =
        (String::new(), String::new(), String::new(), String::new());
    if let Ok(out) = Command::new("sudo")
        .arg("-n")
        .arg(script_path("battery-mem-dimm.sh"))
        .output()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some((k, v)) = line.split_once('=') {
                match k {
                    "TYPE" => dimm_type = v.trim().to_string(),
                    "FORM" => dimm_form = v.trim().to_string(),
                    "SPEED" => dimm_speed = v.trim().to_string(),
                    "SLOTS" => dimm_slots = v.trim().to_string(),
                    _ => {}
                }
            }
        }
    }
    let l1 = format!(
        "{} / {}",
        if l1d.is_empty() { "—" } else { &l1d },
        if l1i.is_empty() { "—" } else { &l1i }
    );
    if let Ok(mut g) = sh.lock() {
        g.model = model;
        g.base_ghz = base_ghz;
        g.sockets = sockets;
        g.logical_str = logical.to_string();
        g.virt = virt;
        g.vm = vm;
        g.l1 = l1;
        g.l2 = if l2.is_empty() { "—".into() } else { l2 };
        g.l3 = if l3.is_empty() { "—".into() } else { l3 };
        g.dimm_type = if dimm_type.is_empty() { "—".into() } else { dimm_type };
        g.dimm_form = if dimm_form.is_empty() { "—".into() } else { dimm_form };
        g.dimm_speed = if dimm_speed.is_empty() { "—".into() } else { dimm_speed };
        g.dimm_slots = if dimm_slots.is_empty() { "—".into() } else { dimm_slots };
    }
}

// Parse one `lsblk -P` line (KEY="value" pairs) into a map.
fn lsblk_kv(line: &str) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        let ks = i;
        while i < b.len() && b[i] != b'=' {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let key = line[ks..i].to_string();
        i += 1;
        if i >= b.len() || b[i] != b'"' {
            break;
        }
        i += 1;
        let vs = i;
        while i < b.len() && b[i] != b'"' {
            i += 1;
        }
        let val = line[vs..i.min(b.len())].to_string();
        i += 1;
        m.insert(key, val);
    }
    m
}

// Enumerate physical drives + partitions (design mirrors Mission Center's Disk
// view; data comes from lsblk + /proc/diskstats — no GPL code reused).
// Pure: returns a fresh list with graph history initialized, no shared lock.
fn enumerate_drives() -> Vec<DriveInfo> {
    let root_disk = Command::new("findmnt")
        .args(["-n", "-o", "SOURCE", "/"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split('[')
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .and_then(|src| Command::new("lsblk").args(["-no", "PKNAME", &src]).output().ok())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let out = match Command::new("lsblk")
        .args([
            "-b", "-P", "-o",
            "NAME,TYPE,MODEL,SERIAL,WWN,ROTA,SIZE,FSTYPE,MOUNTPOINT,FSUSED",
        ])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut drives: Vec<DriveInfo> = Vec::new();
    for line in text.lines() {
        let m = lsblk_kv(line);
        let ty = m.get("TYPE").map(|s| s.as_str()).unwrap_or("");
        let name = m.get("NAME").cloned().unwrap_or_default();
        if ty == "disk" {
            if name.starts_with("zram")
                || name.starts_with("loop")
                || name.starts_with("ram")
                || name.starts_with("dm-")
                || name.starts_with("md")
                || name.starts_with("sr")
            {
                continue;
            }
            let rota = m.get("ROTA").map(|s| s == "1").unwrap_or(false);
            let kind = if name.starts_with("nvme") {
                "NVMe"
            } else if rota {
                "HDD"
            } else {
                "SSD"
            };
            let cap = m.get("SIZE").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            drives.push(DriveInfo {
                dev: name.clone(),
                kind: kind.to_string(),
                model: m.get("MODEL").cloned().unwrap_or_default().trim().to_string(),
                serial: m.get("SERIAL").cloned().unwrap_or_default(),
                wwn: m.get("WWN").cloned().unwrap_or_default(),
                capacity: cap,
                formatted: 0,
                is_system: !root_disk.is_empty() && name == root_disk,
                rotational: rota,
                active_hist: VecDeque::from(vec![0.0; HISTORY]),
                thru_hist: VecDeque::from(vec![0.0; HISTORY]),
                ..Default::default()
            });
        } else if ty == "part" {
            if let Some(d) = drives.last_mut() {
                if !name.starts_with(&d.dev) {
                    continue;
                }
                let size = m.get("SIZE").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                let used = m.get("FSUSED").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                d.formatted += size;
                d.partitions.push(PartInfo {
                    name: name.clone(),
                    fstype: m.get("FSTYPE").cloned().unwrap_or_default(),
                    mount: m.get("MOUNTPOINT").cloned().unwrap_or_default(),
                    used,
                    size,
                });
            }
        }
    }
    drives.sort_by(|a, b| a.dev.cmp(&b.dev));
    drives
}

// Startup: populate the drive list once.
fn gather_drives_static(sh: &Arc<Mutex<Shared>>) {
    let drives = enumerate_drives();
    if let Ok(mut g) = sh.lock() {
        g.drives = drives;
    }
}

// Structural signature of the drive set (dev + partition names). Changes only
// when a disk or partition is added/removed — used to trigger a UI rebuild.
fn drive_signature(drives: &[DriveInfo]) -> String {
    let mut s = String::new();
    for d in drives {
        s.push_str(&d.dev);
        s.push('|');
        for p in &d.partitions {
            s.push_str(&p.name);
            s.push(',');
        }
        s.push(';');
    }
    s
}

// Periodic hotplug re-scan: merge a fresh enumeration into the shared list,
// preserving each surviving drive's live counters/history so graphs don't reset.
fn refresh_drive_list(sh: &Arc<Mutex<Shared>>) {
    let fresh = enumerate_drives();
    if let Ok(mut g) = sh.lock() {
        let mut old: std::collections::HashMap<String, DriveInfo> =
            g.drives.drain(..).map(|d| (d.dev.clone(), d)).collect();
        let mut merged = Vec::with_capacity(fresh.len());
        for mut f in fresh {
            if let Some(o) = old.remove(&f.dev) {
                f.active_hist = o.active_hist;
                f.thru_hist = o.thru_hist;
                f.prev = o.prev;
                f.read_bps = o.read_bps;
                f.write_bps = o.write_bps;
                f.total_read = o.total_read;
                f.total_written = o.total_written;
                f.active_pct = o.active_pct;
                f.resp_ms = o.resp_ms;
                for p in f.partitions.iter_mut() {
                    if p.used == 0 {
                        if let Some(op) = o.partitions.iter().find(|x| x.name == p.name) {
                            p.used = op.used;
                        }
                    }
                }
            }
            merged.push(f);
        }
        g.drives = merged;
    }
}
fn sample_diskstats(sh: &Arc<Mutex<Shared>>) {
    let content = match fs::read_to_string("/proc/diskstats") {
        Ok(c) => c,
        Err(_) => return,
    };
    let mut map: std::collections::HashMap<String, [u64; 7]> = std::collections::HashMap::new();
    for line in content.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 13 {
            continue;
        }
        let g = |i: usize| f.get(i).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        // reads f3, sect_read f5, ms_read f6, writes f7, sect_written f9, ms_write f10, ms_io f12
        map.insert(f[2].to_string(), [g(3), g(5), g(6), g(7), g(9), g(10), g(12)]);
    }
    if let Ok(mut guard) = sh.lock() {
        for d in guard.drives.iter_mut() {
            if let Some(c) = map.get(&d.dev) {
                let cur = DiskPrev {
                    reads: c[0],
                    sect_read: c[1],
                    ms_read: c[2],
                    writes: c[3],
                    sect_written: c[4],
                    ms_write: c[5],
                    ms_io: c[6],
                };
                d.total_read = cur.sect_read * 512;
                d.total_written = cur.sect_written * 512;
                if let Some(p) = d.prev {
                    let dt = 1.0_f64; // loop interval (s)
                    d.read_bps = cur.sect_read.saturating_sub(p.sect_read) as f64 * 512.0 / dt;
                    d.write_bps = cur.sect_written.saturating_sub(p.sect_written) as f64 * 512.0 / dt;
                    let dms_io = cur.ms_io.saturating_sub(p.ms_io) as f64;
                    d.active_pct = (dms_io / (dt * 1000.0) * 100.0).clamp(0.0, 100.0);
                    let dios = (cur.reads.saturating_sub(p.reads)
                        + cur.writes.saturating_sub(p.writes)) as f64;
                    let dms = (cur.ms_read.saturating_sub(p.ms_read)
                        + cur.ms_write.saturating_sub(p.ms_write)) as f64;
                    d.resp_ms = if dios > 0.0 { dms / dios } else { 0.0 };
                    d.active_hist.push_back(d.active_pct);
                    while d.active_hist.len() > HISTORY {
                        d.active_hist.pop_front();
                    }
                    d.thru_hist.push_back(d.read_bps + d.write_bps);
                    while d.thru_hist.len() > HISTORY {
                        d.thru_hist.pop_front();
                    }
                }
                d.prev = Some(cur);
            }
        }
    }
}

// Refresh live partition usage (gated to every 3rd tick).
fn refresh_part_usage(sh: &Arc<Mutex<Shared>>) {
    let out = match Command::new("lsblk")
        .args(["-b", "-P", "-o", "NAME,FSUSED"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return,
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut used: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for line in text.lines() {
        let m = lsblk_kv(line);
        if let Some(n) = m.get("NAME") {
            let u = m.get("FSUSED").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            used.insert(n.clone(), u);
        }
    }
    if let Ok(mut g) = sh.lock() {
        for d in g.drives.iter_mut() {
            for p in d.partitions.iter_mut() {
                if let Some(u) = used.get(&p.name) {
                    if *u > 0 {
                        p.used = *u;
                    }
                }
            }
        }
    }
}

fn parse_dpm(s: &str) -> (f64, f64) {
    // Lines like "2: 1600Mhz *". Returns (current-marked, max).
    let (mut cur, mut max) = (0.0f64, 0.0f64);
    for line in s.lines() {
        let star = line.contains('*');
        let t = line.split(':').nth(1).unwrap_or("").trim();
        let num: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(v) = num.parse::<f64>() {
            if v > max {
                max = v;
            }
            if star {
                cur = v;
            }
        }
    }
    (cur, max)
}

// Enumerate GPUs: NVIDIA via nvidia-smi, AMD/Intel via DRM sysfs. Clean-room,
// mirrors Mission Center's GPU view data sources only.
fn enumerate_gpus() -> Vec<GpuInfo> {
    let mut gpus: Vec<GpuInfo> = Vec::new();
    if let Ok(out) = Command::new("nvidia-smi")
        .args(["--query-gpu=index,name,pci.bus_id", "--format=csv,noheader,nounits"])
        .output()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let f: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if f.len() >= 3 {
                if let Ok(idx) = f[0].parse::<u32>() {
                    gpus.push(GpuInfo {
                        kind: "NVIDIA".into(),
                        name: f[1].to_string(),
                        bus: f[2].to_uppercase(),
                        nvidia_index: Some(idx),
                        util_hist: VecDeque::from(vec![0.0; HISTORY]),
                        mem_hist: VecDeque::from(vec![0.0; HISTORY]),
                        ..Default::default()
                    });
                }
            }
        }
    }
    if let Ok(rd) = fs::read_dir("/sys/class/drm") {
        let mut cards: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.len() > 4 && n.starts_with("card") && n[4..].chars().all(|c| c.is_ascii_digit()))
            .collect();
        cards.sort();
        for card in cards {
            let dev = format!("/sys/class/drm/{card}/device");
            let vendor = fs::read_to_string(format!("{dev}/vendor")).unwrap_or_default().trim().to_string();
            let (kind, name) = match vendor.as_str() {
                "0x1002" => ("AMD", "AMD Radeon Graphics"),
                "0x8086" => ("Intel", "Intel Graphics"),
                _ => continue, // NVIDIA handled above; skip other vendors
            };
            let bus = fs::read_link(&dev)
                .ok()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
                .unwrap_or_default()
                .to_uppercase();
            gpus.push(GpuInfo {
                kind: kind.into(),
                name: name.into(),
                bus,
                drm_path: dev,
                util_hist: VecDeque::from(vec![0.0; HISTORY]),
                mem_hist: VecDeque::from(vec![0.0; HISTORY]),
                ..Default::default()
            });
        }
    }
    gpus.sort_by(|a, b| a.bus.cmp(&b.bus));
    gpus
}

fn gather_gpus_static(sh: &Arc<Mutex<Shared>>) {
    let g = enumerate_gpus();
    if let Ok(mut s) = sh.lock() {
        s.gpus = g;
    }
}

fn gpu_signature(gpus: &[GpuInfo]) -> String {
    let mut s = String::new();
    for g in gpus {
        s.push_str(&g.kind);
        s.push(':');
        s.push_str(&g.bus);
        s.push(';');
    }
    s
}

// Periodic re-scan preserving live counters/history for surviving GPUs.
fn refresh_gpu_list(sh: &Arc<Mutex<Shared>>) {
    let fresh = enumerate_gpus();
    if let Ok(mut s) = sh.lock() {
        let mut old: std::collections::HashMap<String, GpuInfo> =
            s.gpus.drain(..).map(|g| (g.bus.clone(), g)).collect();
        let mut merged = Vec::with_capacity(fresh.len());
        for mut f in fresh {
            if let Some(o) = old.remove(&f.bus) {
                f.util_hist = o.util_hist;
                f.mem_hist = o.mem_hist;
                f.util = o.util;
                f.mem_used = o.mem_used;
                f.mem_total = o.mem_total;
                f.temp = o.temp;
                f.power_draw = o.power_draw;
                f.power_limit = o.power_limit;
                f.clock_cur = o.clock_cur;
                f.clock_max = o.clock_max;
                f.mem_clock_cur = o.mem_clock_cur;
                f.mem_clock_max = o.mem_clock_max;
                f.pcie = o.pcie;
                f.enc_util = o.enc_util;
                f.dec_util = o.dec_util;
            }
            merged.push(f);
        }
        s.gpus = merged;
    }
}

// Per-tick GPU sampling. NVIDIA (heavy nvidia-smi) only when do_nvidia; AMD/Intel
// sysfs is cheap and read every tick.
fn sample_gpus(sh: &Arc<Mutex<Shared>>, do_nvidia: bool) {
    let mut nv: std::collections::HashMap<u32, [f64; 12]> = std::collections::HashMap::new();
    let mut nv_pcie: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    if do_nvidia {
        if let Ok(out) = Command::new("nvidia-smi")
            .args([
                "--query-gpu=index,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw,power.limit,clocks.sm,clocks.max.sm,clocks.mem,clocks.max.mem,utilization.encoder,utilization.decoder,pcie.link.gen.gpucurrent,pcie.link.gen.max,pcie.link.width.current",
                "--format=csv,noheader,nounits",
            ])
            .output()
        {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let f: Vec<String> = line.split(',').map(|s| s.trim().to_string()).collect();
                if f.len() < 16 {
                    continue;
                }
                let idx = match f[0].parse::<u32>() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let p = |i: usize| f.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(-1.0);
                nv.insert(
                    idx,
                    [p(1), p(2), p(3), p(4), p(5), p(6), p(7), p(8), p(9), p(10), p(11), p(12)],
                );
                nv_pcie.insert(
                    idx,
                    format!("PCIe Gen {} x{} (maks Gen {})", f[13], f[15], f[14]),
                );
            }
        }
    }
    if let Ok(mut s) = sh.lock() {
        for g in s.gpus.iter_mut() {
            if let Some(idx) = g.nvidia_index {
                if let Some(v) = nv.get(&idx) {
                    g.util = v[0];
                    g.mem_used = (v[1].max(0.0) * 1048576.0) as u64;
                    g.mem_total = (v[2].max(0.0) * 1048576.0) as u64;
                    g.temp = v[3];
                    g.power_draw = v[4];
                    g.power_limit = v[5];
                    g.clock_cur = v[6];
                    g.clock_max = v[7];
                    g.mem_clock_cur = v[8];
                    g.mem_clock_max = v[9];
                    g.enc_util = v[10];
                    g.dec_util = v[11];
                    if let Some(p) = nv_pcie.get(&idx) {
                        g.pcie = p.clone();
                    }
                }
            } else if !g.drm_path.is_empty() {
                let drm = g.drm_path.clone();
                let rf = |name: &str| {
                    fs::read_to_string(format!("{drm}/{name}")).ok().map(|s| s.trim().to_string())
                };
                if let Some(u) = rf("gpu_busy_percent").and_then(|s| s.parse::<f64>().ok()) {
                    g.util = u;
                }
                g.mem_used = rf("mem_info_vram_used").and_then(|s| s.parse::<u64>().ok()).unwrap_or(g.mem_used);
                g.mem_total = rf("mem_info_vram_total").and_then(|s| s.parse::<u64>().ok()).unwrap_or(g.mem_total);
                if let Ok(hw) = fs::read_dir(format!("{drm}/hwmon")) {
                    if let Some(h) = hw.filter_map(|e| e.ok()).next() {
                        let hp = h.path();
                        if let Some(t) = fs::read_to_string(hp.join("temp1_input"))
                            .ok()
                            .and_then(|s| s.trim().parse::<f64>().ok())
                        {
                            g.temp = t / 1000.0;
                        }
                        let pw = fs::read_to_string(hp.join("power1_average"))
                            .ok()
                            .and_then(|s| s.trim().parse::<f64>().ok())
                            .or_else(|| {
                                fs::read_to_string(hp.join("power1_input"))
                                    .ok()
                                    .and_then(|s| s.trim().parse::<f64>().ok())
                            });
                        if let Some(p) = pw {
                            g.power_draw = p / 1e6;
                        }
                    }
                }
                let (sc, sm) = parse_dpm(&rf("pp_dpm_sclk").unwrap_or_default());
                g.clock_cur = sc;
                g.clock_max = sm;
                let (mc, mm) = parse_dpm(&rf("pp_dpm_mclk").unwrap_or_default());
                g.mem_clock_cur = mc;
                g.mem_clock_max = mm;
                let sp = rf("current_link_speed").unwrap_or_default().replace(" PCIe", "");
                let wd = rf("current_link_width").unwrap_or_default();
                if !wd.is_empty() {
                    g.pcie = format!("x{wd} @ {sp}");
                }
                g.enc_util = -1.0;
                g.dec_util = -1.0;
            }
            g.util_hist.push_back(g.util.max(0.0));
            while g.util_hist.len() > HISTORY {
                g.util_hist.pop_front();
            }
            let mempct = if g.mem_total > 0 {
                g.mem_used as f64 / g.mem_total as f64 * 100.0
            } else {
                0.0
            };
            g.mem_hist.push_back(mempct);
            while g.mem_hist.len() > HISTORY {
                g.mem_hist.pop_front();
            }
        }
    }
}

fn read_total_jiffies() -> u64 {
    if let Ok(s) = fs::read_to_string("/proc/stat") {
        if let Some(line) = s.lines().next() {
            return line.split_whitespace().skip(1).filter_map(|x| x.parse::<u64>().ok()).sum();
        }
    }
    0
}

// Sample all processes: CPU% (over the interval), RSS and swap. `prev`/`prev_total`
// persist between samples for the CPU delta. Returns (sorted list, total count).
// Map pid -> comma-joined listening ports via `ss` (own processes; root-owned
// sockets omit the pid unless we are root).
fn listening_ports() -> std::collections::HashMap<u32, Vec<u16>> {
    let mut m: std::collections::HashMap<u32, Vec<u16>> = std::collections::HashMap::new();
    if let Ok(o) = Command::new("ss").args(["-H", "-tulpn"]).output() {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 5 {
                continue;
            }
            let port = f[4].rsplit(':').next().and_then(|p| p.parse::<u16>().ok());
            let port = match port {
                Some(p) => p,
                None => continue,
            };
            for cap in line.split("pid=").skip(1) {
                let digits: String = cap.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(pid) = digits.parse::<u32>() {
                    let e = m.entry(pid).or_default();
                    if !e.contains(&port) {
                        e.push(port);
                    }
                }
            }
        }
    }
    m
}

fn sample_procs(
    prev: &mut std::collections::HashMap<u32, u64>,
    prev_total: &mut u64,
    io_prev: &mut std::collections::HashMap<u32, u64>,
) -> (Vec<ProcInfo>, usize) {
    let total = read_total_jiffies();
    let dtotal = total.saturating_sub(*prev_total).max(1) as f64;
    let mut cur: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    let mut io_cur: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    let port_map = listening_ports();
    let mut out: Vec<ProcInfo> = Vec::new();
    let mut count = 0usize;
    if let Ok(rd) = fs::read_dir("/proc") {
        for e in rd.flatten() {
            let fname = e.file_name();
            let s = fname.to_string_lossy();
            if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let pid: u32 = match s.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
                Ok(x) => x,
                Err(_) => continue,
            };
            count += 1;
            let (lp, rp) = (stat.find('('), stat.rfind(')'));
            let (name, rest) = match (lp, rp) {
                (Some(l), Some(r)) if r + 2 <= stat.len() && l + 1 <= r => {
                    (stat[l + 1..r].to_string(), &stat[r + 2..])
                }
                _ => continue,
            };
            let f: Vec<&str> = rest.split_whitespace().collect();
            // rest[0]=state (field 3); utime=field14=rest[11], stime=field15=rest[12]
            let utime = f.get(11).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
            let stime = f.get(12).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
            let jif = utime + stime;
            cur.insert(pid, jif);
            let dj = jif.saturating_sub(prev.get(&pid).copied().unwrap_or(jif)) as f64;
            let cpu = (100.0 * dj / dtotal).clamp(0.0, 100.0);
            let (mut rss, mut swap) = (0u64, 0u64);
            if let Ok(st) = fs::read_to_string(format!("/proc/{pid}/status")) {
                for line in st.lines() {
                    if let Some(v) = line.strip_prefix("VmRSS:") {
                        rss = v.trim().trim_end_matches("kB").trim().parse().unwrap_or(0);
                    } else if let Some(v) = line.strip_prefix("VmSwap:") {
                        swap = v.trim().trim_end_matches("kB").trim().parse().unwrap_or(0);
                    }
                }
            }
            // Per-process disk I/O (only readable for the user's own processes).
            let (mut io_bps, mut io_known) = (0.0, false);
            if let Ok(io) = fs::read_to_string(format!("/proc/{pid}/io")) {
                let mut bytes = 0u64;
                for line in io.lines() {
                    if let Some(v) = line.strip_prefix("read_bytes:").or_else(|| line.strip_prefix("write_bytes:")) {
                        bytes += v.trim().parse::<u64>().unwrap_or(0);
                    }
                }
                io_known = true;
                let d = bytes.saturating_sub(io_prev.get(&pid).copied().unwrap_or(bytes)) as f64;
                io_bps = d / 3.0; // sampled ~every 3s
                io_cur.insert(pid, bytes);
            }
            let ports = port_map
                .get(&pid)
                .map(|v| {
                    let mut s = v.clone();
                    s.sort_unstable();
                    s.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")
                })
                .unwrap_or_default();
            out.push(ProcInfo { pid, name, cpu, rss_kb: rss, swap_kb: swap, io_bps, io_known, ports });
        }
    }
    *prev = cur;
    *prev_total = total;
    *io_prev = io_cur;
    out.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.rss_kb.cmp(&a.rss_kb))
    });
    (out, count)
}

// List all systemd units (service/socket/mount/timer/target) for the system or
// user manager, with live memory from a single batched `systemctl show`.
fn list_services(is_user: bool) -> Vec<SvcUnit> {
    let run = |extra: &[&str]| {
        let mut c = Command::new("systemctl");
        if is_user {
            c.arg("--user");
        }
        c.args(extra).output().ok()
    };
    let out = match run(&[
        "list-units",
        "--type=service,socket,mount,timer,target",
        "--all",
        "--plain",
        "--no-legend",
    ]) {
        Some(o) => o,
        None => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut units: Vec<SvcUnit> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 4 {
            continue;
        }
        let unit = f[0].to_string();
        // f[1]=LOAD f[2]=ACTIVE f[3]=SUB, rest = description
        let active = f[2].to_string();
        let sub = f[3].to_string();
        ids.push(unit.clone());
        units.push(SvcUnit { unit, is_user, active, sub, mem: 0 });
    }
    if !ids.is_empty() {
        let mut args: Vec<String> = vec!["show".into(), "-p".into(), "Id".into(), "-p".into(), "MemoryCurrent".into()];
        args.extend(ids);
        let mut c = Command::new("systemctl");
        if is_user {
            c.arg("--user");
        }
        if let Ok(o) = c.args(&args).output() {
            let t = String::from_utf8_lossy(&o.stdout);
            let mut map: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
            let (mut id, mut mem) = (String::new(), 0u64);
            for line in t.lines() {
                if line.is_empty() {
                    if !id.is_empty() {
                        map.insert(std::mem::take(&mut id), mem);
                    }
                    mem = 0;
                } else if let Some(v) = line.strip_prefix("Id=") {
                    id = v.to_string();
                } else if let Some(v) = line.strip_prefix("MemoryCurrent=") {
                    let n: u64 = v.parse().unwrap_or(0);
                    mem = if n == u64::MAX { 0 } else { n };
                }
            }
            if !id.is_empty() {
                map.insert(id, mem);
            }
            for u in units.iter_mut() {
                if let Some(m) = map.get(&u.unit) {
                    u.mem = *m;
                }
            }
        }
    }
    units
}

fn enumerate_bats() -> Vec<BatInfo> {
    let mut bats: Vec<(i32, BatInfo)> = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/class/power_supply") {
        let mut names: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        for name in names {
            let path = format!("/sys/class/power_supply/{name}");
            let ty = fs::read_to_string(format!("{path}/type")).unwrap_or_default().trim().to_string();
            if ty != "Battery" {
                continue;
            }
            let scope = fs::read_to_string(format!("{path}/scope")).unwrap_or_default().trim().to_string();
            let is_system = scope != "Device";
            let model = fs::read_to_string(format!("{path}/model_name"))
                .unwrap_or_default()
                .trim()
                .to_string();
            let rank = if is_system { 0 } else { 1 };
            bats.push((
                rank,
                BatInfo {
                    name: name.clone(),
                    model,
                    path,
                    is_system,
                    pct_hist: VecDeque::from(vec![0.0; HISTORY]),
                    power_hist: VecDeque::from(vec![0.0; HISTORY]),
                    ..Default::default()
                },
            ));
        }
    }
    bats.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.name.cmp(&b.1.name)));
    bats.into_iter().map(|(_, b)| b).collect()
}

fn gather_bats_static(sh: &Arc<Mutex<Shared>>) {
    let b = enumerate_bats();
    if let Ok(mut s) = sh.lock() {
        s.bats = b;
    }
}

fn bat_signature(bats: &[BatInfo]) -> String {
    let mut s = String::new();
    for b in bats {
        s.push_str(&b.name);
        s.push(if b.is_system { 'S' } else { 'D' });
        s.push(';');
    }
    s
}

fn refresh_bat_list(sh: &Arc<Mutex<Shared>>) {
    let fresh = enumerate_bats();
    if let Ok(mut s) = sh.lock() {
        let mut old: std::collections::HashMap<String, BatInfo> =
            s.bats.drain(..).map(|b| (b.name.clone(), b)).collect();
        let mut merged = Vec::with_capacity(fresh.len());
        for mut f in fresh {
            if let Some(o) = old.remove(&f.name) {
                f.pct_hist = o.pct_hist;
                f.power_hist = o.power_hist;
            }
            merged.push(f);
        }
        s.bats = merged;
    }
}

// Per-tick battery sampling from /sys/class/power_supply (cheap).
fn sample_bats(sh: &Arc<Mutex<Shared>>) {
    if let Ok(mut s) = sh.lock() {
        for b in s.bats.iter_mut() {
            let p = b.path.clone();
            let rn = |f: &str| fs::read_to_string(format!("{p}/{f}")).ok().map(|s| s.trim().to_string());
            let num = |f: &str| rn(f).and_then(|s| s.parse::<f64>().ok());
            // percent (numeric, else capacity_level word)
            b.percent = num("capacity").unwrap_or_else(|| match rn("capacity_level").unwrap_or_default().as_str() {
                "Full" => 100.0,
                "High" => 80.0,
                "Normal" | "Good" => 55.0,
                "Low" => 20.0,
                "Critical" => 5.0,
                _ => b.percent,
            });
            let st = rn("status").unwrap_or_default();
            b.state = match st.as_str() {
                "Full" => "Full".into(),
                "Charging" => "Charging".into(),
                "Discharging" => "Discharging".into(),
                "Not charging" => "Not charging".into(),
                s if !s.is_empty() => s.to_string(),
                _ => "Unknown".into(),
            };
            if b.is_system {
                let volt = num("voltage_now").map(|v| v / 1e6).unwrap_or(0.0);
                b.voltage = volt;
                // power: energy-based power_now, else current_now * voltage
                b.power = num("power_now")
                    .map(|w| w / 1e6)
                    .or_else(|| num("current_now").map(|c| c / 1e6 * volt))
                    .map(|w| w.abs())
                    .unwrap_or(0.0);
                b.cycles = num("cycle_count").unwrap_or(0.0) as i64;
                b.serial = rn("serial_number").unwrap_or_default();
                b.technology = match rn("technology").unwrap_or_default().as_str() {
                    "Li-ion" | "Li-Ion" => "Lithium Ion".into(),
                    "Li-poly" => "Lithium Polymer".into(),
                    other => other.to_string(),
                };
                let vmin = num("voltage_min_design").map(|v| v / 1e6).unwrap_or(volt);
                b.voltage_min_design = vmin;
                // energy (Wh): prefer energy_* (µWh); else charge_* (µAh) * vmin
                let ef = num("energy_full")
                    .map(|e| e / 1e6)
                    .or_else(|| num("charge_full").map(|c| c / 1e6 * vmin))
                    .unwrap_or(0.0);
                let efd = num("energy_full_design")
                    .map(|e| e / 1e6)
                    .or_else(|| num("charge_full_design").map(|c| c / 1e6 * vmin))
                    .unwrap_or(0.0);
                b.energy_full = ef;
                b.energy_full_design = efd;
                b.capacity_health = if efd > 0.0 { ef / efd * 100.0 } else { 0.0 };
                b.charge_threshold = match num("charge_control_end_threshold") {
                    Some(t) if t < 100.0 => "Yes".into(),
                    Some(_) => "No".into(),
                    None => "—".into(),
                };
                b.power_hist.push_back(b.power.max(0.0));
                while b.power_hist.len() > HISTORY {
                    b.power_hist.pop_front();
                }
            } else {
                b.serial = rn("serial_number").unwrap_or_default();
            }
            b.pct_hist.push_back(b.percent.clamp(0.0, 100.0));
            while b.pct_hist.len() > HISTORY {
                b.pct_hist.pop_front();
            }
        }
    }
}

// Resolve an adapter marketing name from lspci for a PCI bus (e.g. "0000:03:00.0").
fn lspci_name(bus: &str) -> String {
    if bus.is_empty() {
        return String::new();
    }
    let short = bus.strip_prefix("0000:").unwrap_or(bus);
    if let Ok(out) = Command::new("lspci").args(["-mm", "-s", short]).output() {
        let line = String::from_utf8_lossy(&out.stdout);
        // Quoted fields: slot "class" "vendor" "device" ...  -> device is 3rd quote pair
        let quoted: Vec<&str> = line.split('"').collect();
        // indices 1=class,3=vendor,5=device
        if quoted.len() > 5 {
            return quoted[5].trim().to_string();
        }
    }
    String::new()
}

fn enumerate_nets() -> Vec<NetInfo> {
    let mut nets: Vec<(i32, NetInfo)> = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/class/net") {
        let mut ifs: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n != "lo" && !n.starts_with("veth"))
            .collect();
        ifs.sort();
        for iface in ifs {
            let base = format!("/sys/class/net/{iface}");
            let is_wireless = Path::new(&format!("{base}/wireless")).exists();
            let has_dev = Path::new(&format!("{base}/device")).exists();
            let (kind, rank) = if is_wireless {
                ("Wireless", 1)
            } else if has_dev {
                ("Wired", 0)
            } else {
                ("Other", 2)
            };
            let bus = fs::read_link(format!("{base}/device"))
                .ok()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
                .unwrap_or_default()
                .to_uppercase();
            let model = lspci_name(&bus);
            let mac = fs::read_to_string(format!("{base}/address")).unwrap_or_default().trim().to_string();
            nets.push((
                rank,
                NetInfo {
                    iface: iface.clone(),
                    kind: kind.into(),
                    model,
                    mac,
                    is_wireless,
                    rx_hist: VecDeque::from(vec![0.0; HISTORY]),
                    tx_hist: VecDeque::from(vec![0.0; HISTORY]),
                    ..Default::default()
                },
            ));
        }
    }
    nets.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.iface.cmp(&b.1.iface)));
    nets.into_iter().map(|(_, n)| n).collect()
}

fn gather_nets_static(sh: &Arc<Mutex<Shared>>) {
    let n = enumerate_nets();
    if let Ok(mut s) = sh.lock() {
        s.nets = n;
    }
}

fn net_signature(nets: &[NetInfo]) -> String {
    let mut s = String::new();
    for n in nets {
        s.push_str(&n.iface);
        s.push(';');
    }
    s
}

fn refresh_net_list(sh: &Arc<Mutex<Shared>>) {
    let fresh = enumerate_nets();
    if let Ok(mut s) = sh.lock() {
        let mut old: std::collections::HashMap<String, NetInfo> =
            s.nets.drain(..).map(|n| (n.iface.clone(), n)).collect();
        let mut merged = Vec::with_capacity(fresh.len());
        for mut f in fresh {
            if let Some(o) = old.remove(&f.iface) {
                f.rx_hist = o.rx_hist;
                f.tx_hist = o.tx_hist;
                f.prev = o.prev;
                f.rx_bps = o.rx_bps;
                f.tx_bps = o.tx_bps;
                f.total_rx = o.total_rx;
                f.total_tx = o.total_tx;
                f.status = o.status;
                f.ipv4 = o.ipv4;
                f.ipv6 = o.ipv6;
                f.ssid = o.ssid;
                f.signal = o.signal;
                f.freq = o.freq;
            }
            merged.push(f);
        }
        s.nets = merged;
    }
}

// Per-tick throughput sampling from /sys/class/net statistics (cheap).
fn sample_nets(sh: &Arc<Mutex<Shared>>) {
    if let Ok(mut s) = sh.lock() {
        for n in s.nets.iter_mut() {
            let base = format!("/sys/class/net/{}", n.iface);
            let rx = fs::read_to_string(format!("{base}/statistics/rx_bytes"))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0);
            let tx = fs::read_to_string(format!("{base}/statistics/tx_bytes"))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0);
            n.total_rx = rx;
            n.total_tx = tx;
            if let Some((prx, ptx)) = n.prev {
                n.rx_bps = rx.saturating_sub(prx) as f64;
                n.tx_bps = tx.saturating_sub(ptx) as f64;
            }
            n.prev = Some((rx, tx));
            n.rx_hist.push_back(n.rx_bps.max(0.0));
            while n.rx_hist.len() > HISTORY {
                n.rx_hist.pop_front();
            }
            n.tx_hist.push_back(n.tx_bps.max(0.0));
            while n.tx_hist.len() > HISTORY {
                n.tx_hist.pop_front();
            }
            let oper = fs::read_to_string(format!("{base}/operstate")).unwrap_or_default().trim().to_string();
            n.status = match oper.as_str() {
                "up" => "Connected".into(),
                "unknown" => {
                    if n.total_rx > 0 || n.total_tx > 0 {
                        "Connected".into()
                    } else {
                        "Unknown".into()
                    }
                }
                "down" => "Unavailable".into(),
                other if !other.is_empty() => other.to_string(),
                _ => "—".into(),
            };
        }
    }
}

// Heavier extras (IP addresses + wireless link) gathered every 3rd tick.
fn refresh_net_extra(sh: &Arc<Mutex<Shared>>) {
    let list: Vec<(String, bool)> = match sh.lock() {
        Ok(s) => s.nets.iter().map(|n| (n.iface.clone(), n.is_wireless)).collect(),
        Err(_) => return,
    };
    let mut info: std::collections::HashMap<String, (String, String, String, String, String)> =
        std::collections::HashMap::new(); // iface -> (v4, v6, ssid, signal, freq)
    for (iface, wl) in &list {
        let (mut v4, mut v6) = (String::new(), String::new());
        if let Ok(out) = Command::new("ip").args(["-o", "addr", "show", iface]).output() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if let Some(pos) = f.iter().position(|&x| x == "inet") {
                    if v4.is_empty() {
                        v4 = f.get(pos + 1).map(|s| s.split('/').next().unwrap_or("").to_string()).unwrap_or_default();
                    }
                } else if let Some(pos) = f.iter().position(|&x| x == "inet6") {
                    if let Some(a) = f.get(pos + 1) {
                        // prefer a global (non fe80) address
                        if !a.starts_with("fe80") && v6.is_empty() {
                            v6 = a.split('/').next().unwrap_or("").to_string();
                        }
                    }
                }
            }
        }
        let (mut ssid, mut signal, mut freq) = (String::new(), String::new(), String::new());
        if *wl {
            if let Ok(out) = Command::new("iw").args(["dev", iface, "link"]).output() {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    let t = line.trim();
                    if let Some(v) = t.strip_prefix("SSID:") {
                        ssid = v.trim().to_string();
                    } else if let Some(v) = t.strip_prefix("signal:") {
                        signal = v.trim().to_string();
                    } else if let Some(v) = t.strip_prefix("freq:") {
                        if let Ok(mhz) = v.trim().split('.').next().unwrap_or("").parse::<f64>() {
                            freq = format!("{:.2} GHz", mhz / 1000.0);
                        }
                    }
                }
            }
        }
        info.insert(iface.clone(), (v4, v6, ssid, signal, freq));
    }
    if let Ok(mut s) = sh.lock() {
        for n in s.nets.iter_mut() {
            if let Some((v4, v6, ssid, signal, freq)) = info.get(&n.iface) {
                n.ipv4 = v4.clone();
                n.ipv6 = v6.clone();
                n.ssid = ssid.clone();
                n.signal = signal.clone();
                n.freq = freq.clone();
            }
        }
    }

    // Tailnet device list — only query when a tailscale interface is present.
    let has_ts = list.iter().any(|(iface, _)| iface.starts_with("tailscale"));
    let (running, peers) = if has_ts { query_tailscale_peers() } else { (false, Vec::new()) };
    if let Ok(mut s) = sh.lock() {
        s.ts_peers = peers;
        s.ts_running = running;
    }
}

// Parse `tailscale status` plain output into the tailnet device list.
// Device names in the plain output are DNS labels (hyphenated, no spaces), so
// whitespace tokenisation is safe. Column layout:
//   <ip> <name> <user@> <os> <status...>
// The first data row is this machine (self); "offline" anywhere in the status
// tail marks a peer as offline.
fn query_tailscale_peers() -> (bool, Vec<TsPeer>) {
    let out = match Command::new("tailscale").arg("status").output() {
        Ok(o) => o,
        Err(_) => return (false, Vec::new()),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    // When the backend is stopped, `tailscale status` prints "Tailscale is
    // stopped." (and exits non-zero). Treat any self line / success as running.
    let running = out.status.success() && !text.contains("Tailscale is stopped");
    let mut peers: Vec<TsPeer> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let f: Vec<&str> = t.split_whitespace().collect();
        if f.len() < 4 {
            continue;
        }
        // First token must look like a Tailscale CGNAT IPv4 (100.x.y.z).
        let ip = f[0];
        if !ip.starts_with("100.") || ip.matches('.').count() != 3 {
            continue;
        }
        let online = !t.contains("offline");
        peers.push(TsPeer {
            name: f[1].to_string(),
            ip: ip.to_string(),
            os: f[3].to_string(),
            online,
            is_self: i == 0,
        });
    }
    // Online first, then self, then alphabetical.
    peers.sort_by(|a, b| {
        b.online
            .cmp(&a.online)
            .then(b.is_self.cmp(&a.is_self))
            .then(a.name.cmp(&b.name))
    });
    (running, peers)
}

// Enumerate fans from hwmon fanN_input (with fanN_label when present).
fn enumerate_fans() -> Vec<FanInfo> {
    let mut fans: Vec<FanInfo> = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/class/hwmon") {
        let mut hmons: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path().to_string_lossy().to_string())
            .collect();
        hmons.sort();
        for h in hmons {
            let hname = h.rsplit('/').next().unwrap_or("hwmon").to_string();
            if let Ok(entries) = fs::read_dir(&h) {
                let mut inputs: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| n.starts_with("fan") && n.ends_with("_input"))
                    .collect();
                inputs.sort();
                for inp in inputs {
                    let base = inp.trim_end_matches("_input").to_string();
                    let path = format!("{h}/{inp}");
                    let label = fs::read_to_string(format!("{h}/{base}_label"))
                        .ok()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| base.clone());
                    fans.push(FanInfo {
                        label,
                        path,
                        key: format!("{hname}:{base}"),
                        hist: VecDeque::from(vec![0.0; HISTORY]),
                        rpm: 0.0,
                    });
                }
            }
        }
    }
    fans
}

fn gather_fans_static(sh: &Arc<Mutex<Shared>>) {
    let f = enumerate_fans();
    if let Ok(mut s) = sh.lock() {
        s.fans = f;
    }
}

fn fan_signature(fans: &[FanInfo]) -> String {
    let mut s = String::new();
    for f in fans {
        s.push_str(&f.key);
        s.push(';');
    }
    s
}

fn refresh_fan_list(sh: &Arc<Mutex<Shared>>) {
    let fresh = enumerate_fans();
    if let Ok(mut s) = sh.lock() {
        let mut old: std::collections::HashMap<String, FanInfo> =
            s.fans.drain(..).map(|f| (f.key.clone(), f)).collect();
        let mut merged = Vec::with_capacity(fresh.len());
        for mut f in fresh {
            if let Some(o) = old.remove(&f.key) {
                f.hist = o.hist;
                f.rpm = o.rpm;
            }
            merged.push(f);
        }
        s.fans = merged;
    }
}

// Per-tick fan RPM sampling (cheap sysfs reads).
fn sample_fans(sh: &Arc<Mutex<Shared>>) {
    if let Ok(mut s) = sh.lock() {
        for f in s.fans.iter_mut() {
            if let Some(rpm) = fs::read_to_string(&f.path).ok().and_then(|s| s.trim().parse::<f64>().ok()) {
                f.rpm = rpm;
            }
            f.hist.push_back(f.rpm.max(0.0));
            while f.hist.len() > HISTORY {
                f.hist.pop_front();
            }
        }
    }
}

// ───────────────────────── Disk S.M.A.R.T. sampling ─────────────────────────
fn sample_smart(sh: &Arc<Mutex<Shared>>) {
    // Get the list of drive devices from Shared.
    let devs: Vec<String> = match sh.lock() {
        Ok(g) => g.drives.iter().map(|d| d.dev.clone()).collect(),
        Err(_) => return,
    };
    let script = script_path("asus-disk-smart.sh");
    let mut results = Vec::new();
    for dev in &devs {
        let out = Command::new("sudo")
            .args(["-n", &script, dev])
            .output();
        let mut info = DiskSmartInfo { dev: dev.clone(), ..Default::default() };
        if let Ok(o) = out {
            let text = String::from_utf8_lossy(&o.stdout);
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    let v = v.trim().to_string();
                    match k.trim() {
                        "SMARTCTL_MISSING" => { info.smartctl_missing = v == "1"; }
                        "HEALTH" => info.health = v,
                        "TEMP" => info.temp = v,
                        "POWER_ON_HOURS" => info.power_on_hours = v,
                        "POWER_CYCLES" => info.power_cycles = v,
                        "REALLOCATED" => info.reallocated = v,
                        "PERCENT_USED" => info.percent_used = v,
                        "DATA_WRITTEN" => info.data_written = v,
                        "MODEL" => info.model = v,
                        _ => {}
                    }
                }
            }
        }
        results.push(info);
    }
    if let Ok(mut g) = sh.lock() {
        g.smart_data = results;
    }
}

fn check_temp_alerts(sh: &Arc<Mutex<Shared>>) {
    const COOLDOWN: Duration = Duration::from_secs(300); // 5 minutes
    const CPU_THRESH: f64 = 95.0;
    const GPU_THRESH: f64 = 90.0;
    const DISK_THRESH: f64 = 70.0;

    let now = Instant::now();
    // Collect candidate alerts: (source_key, title, body)
    let mut candidates: Vec<(String, String, String)> = Vec::new();

    if let Ok(g) = sh.lock() {
        if !g.alerts_enabled {
            return;
        }
        // CPU temperature
        let cpu_temp = g.temp as f64;
        if cpu_temp > CPU_THRESH {
            candidates.push((
                "cpu".into(),
                "CPU Temperature Warning".into(),
                format!("CPU is at {}°C (threshold: {}°C)", cpu_temp as i32, CPU_THRESH as i32),
            ));
        }
        // GPU temperatures
        for gpu in &g.gpus {
            if gpu.temp > GPU_THRESH {
                candidates.push((
                    format!("gpu:{}", gpu.bus),
                    "GPU Temperature Warning".into(),
                    format!("{} is at {:.0}°C (threshold: {}°C)", gpu.name, gpu.temp, GPU_THRESH as i32),
                ));
            }
        }
        // Disk temperatures (from S.M.A.R.T. data)
        for disk in &g.smart_data {
            if let Ok(t) = disk.temp.trim_end_matches("°C").trim().parse::<f64>() {
                if t > DISK_THRESH {
                    let label = if disk.model.is_empty() { disk.dev.clone() } else { disk.model.clone() };
                    candidates.push((
                        format!("disk:{}", disk.dev),
                        "Disk Temperature Warning".into(),
                        format!("{} is at {:.0}°C (threshold: {}°C)", label, t, DISK_THRESH as i32),
                    ));
                }
            }
        }
    } // lock dropped

    if candidates.is_empty() {
        return;
    }

    // Now filter by cooldown and fire notifications.
    let mut to_fire: Vec<(String, String, String)> = Vec::new();
    if let Ok(mut g) = sh.lock() {
        for (key, title, body) in candidates {
            let last = g.alert_cooldowns.get(&key).copied();
            if last.is_none() || now.duration_since(last.unwrap()) >= COOLDOWN {
                g.alert_cooldowns.insert(key, now);
                to_fire.push((String::new(), title, body));
            }
        }
    }
    for (_key, title, body) in to_fire {
        run_user(vec![
            "notify-send".into(),
            "-a".into(),
            "Tweaks ASUS TUF".into(),
            "-i".into(),
            "com.rezkycodes.AsusTufCpu".into(),
            "-u".into(),
            "critical".into(),
            title,
            body,
        ]);
    }
}

fn spawn_sampler(sh: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || {
        let logical = sh.lock().map(|g| g.logical).unwrap_or(1);
        gather_static(&sh, logical);
        let temp_path = find_temp_path();
        let mut prev: Option<((u64, u64), Vec<(u64, u64)>)> = None;
        let mut tick: u64 = 0;
        let mut proc_prev: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
        let mut proc_total_prev: u64 = 0;
        let mut io_prev: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
        loop {
            if let Some((ov, per)) = read_stat() {
                if let Some((pov, pper)) = &prev {
                    let overall = pct(*pov, ov);
                    let core_vals: Vec<f64> = per
                        .iter()
                        .enumerate()
                        .map(|(i, c)| pct(pper.get(i).copied().unwrap_or((0, 0)), *c))
                        .collect();
                    let speed = avg_speed_ghz(logical);
                    let temp = temp_path.as_ref().and_then(|p| rd_u64(p)).map(|mc| mc as f64 / 1000.0);
                    let gov = rd("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor").unwrap_or_default();
                    let drv = rd("/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver").unwrap_or_default();
                    let boost = rd("/sys/devices/system/cpu/cpufreq/boost")
                        .map(|b| if b == "1" { "Active" } else { "Inactive" })
                        .unwrap_or("Inactive")
                        .to_string();
                    let profile = rd("/sys/firmware/acpi/platform_profile")
                        .map(|s| {
                            let mut c = s.chars();
                            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or(s)
                        })
                        .unwrap_or_default();
                    let handles = rd("/proc/sys/fs/file-nr")
                        .and_then(|s| s.split_whitespace().next().map(|x| x.to_string()))
                        .unwrap_or_default();
                    let uptime = rd("/proc/uptime")
                        .and_then(|s| s.split_whitespace().next().and_then(|x| x.parse::<f64>().ok()))
                        .map(|u| fmt_uptime(u as u64))
                        .unwrap_or_default();

                    // power/battery/fan (fast sysfs)
                    let acp = ac_online();
                    let bd = bat_dir();
                    let bat_cap = bd.as_ref().and_then(|d| rd(d.join("capacity").to_str().unwrap())).unwrap_or_default();
                    let bat_status = bd.as_ref().and_then(|d| rd(d.join("status").to_str().unwrap())).unwrap_or_default();
                    let threshold = bd
                        .as_ref()
                        .and_then(|d| rd(d.join("charge_control_end_threshold").to_str().unwrap()))
                        .unwrap_or_else(|| "100".into());
                    let fan1 = fan_input("fan1_input").unwrap_or_else(|| "--".into());
                    let fan2 = fan_input("fan2_input").unwrap_or_else(|| "--".into());
                    let fan_policy = rd("/sys/devices/platform/asus-nb-wmi/throttle_thermal_policy").unwrap_or_else(|| "0".into());
                    let pci_status = rd("/sys/bus/pci/devices/0000:01:00.0/power/runtime_status")
                        .map(|s| {
                            let mut c = s.chars();
                            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or(s)
                        })
                        .unwrap_or_else(|| "Active".into());
                    let gpu_mode = read_kv("/etc/asus-power-manager/gpu.conf", "GPU_MODE").unwrap_or_else(|| "hybrid".into());
                    let power_mode = read_kv("/etc/asus-power-manager/power_mode.conf", "POWER_MODE")
                        .unwrap_or_else(|| {
                            if gov == "powersave" && rd("/sys/devices/system/cpu/cpufreq/boost").as_deref() == Some("0") {
                                "powersave".into()
                            } else if gov == "performance" {
                                "performance".into()
                            } else {
                                "auto".into()
                            }
                        });

                    // Mouse (fast: sysfs battery + cache conf, no solaar in loop)
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
                    let mconf = format!("{home}/.config/asus-power-manager/logitech.conf");
                    let m_hz = read_kv(&mconf, "HZ").unwrap_or_else(|| "1000".into());
                    let m_dpi = read_kv(&mconf, "DPI").unwrap_or_else(|| "1600".into());
                    let m_onboard = read_kv(&mconf, "ONBOARD").map(|v| v == "on").unwrap_or(false);
                    let m_led_mode = read_kv(&mconf, "LED_MODE").unwrap_or_else(|| "1".into());
                    let m_led_color = read_kv(&mconf, "LED_COLOR").unwrap_or_else(|| "0x00c8ff".into());
                    let m_led_period = read_kv(&mconf, "LED_PERIOD").unwrap_or_else(|| "3000".into());
                    let m_led_intensity = read_kv(&mconf, "LED_INTENSITY").unwrap_or_else(|| "100".into());
                    let m_bat = rd("/sys/class/power_supply/hidpp_battery_0/capacity")
                        .or_else(|| read_kv(&mconf, "BATTERY"))
                        .unwrap_or_else(|| "90".into());
                    let m_status = rd("/sys/class/power_supply/hidpp_battery_0/status")
                        .unwrap_or_else(|| "Unknown".into());

                    // Memory (/proc/meminfo, values in kB)
                    let mut mi = std::collections::HashMap::new();
                    if let Ok(s) = fs::read_to_string("/proc/meminfo") {
                        for line in s.lines() {
                            if let Some((k, v)) = line.split_once(':') {
                                let num: f64 = v.trim().split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
                                mi.insert(k.trim().to_string(), num);
                            }
                        }
                    }
                    let g_ = |k: &str| *mi.get(k).unwrap_or(&0.0);
                    let mem_total = g_("MemTotal");
                    let mem_avail = if mi.contains_key("MemAvailable") { g_("MemAvailable") } else { g_("MemFree") + g_("Cached") + g_("Buffers") };
                    let mem_used = (mem_total - mem_avail).max(0.0);
                    let mem_cached = g_("Cached") + g_("SReclaimable") + g_("Buffers");
                    let swap_total = g_("SwapTotal");
                    let swap_used = (swap_total - g_("SwapFree")).max(0.0);
                    let committed = g_("Committed_AS");
                    let mem_pct = if mem_total > 0.0 { (mem_used / mem_total * 100.0).round() as u32 } else { 0 };
                    let mem_pct_f = if mem_total > 0.0 { mem_used / mem_total * 100.0 } else { 0.0 };
                    let swap_pct_f = if swap_total > 0.0 { swap_used / swap_total * 100.0 } else { 0.0 };

                    // heavy subprocess data every 3s
                    let counts = if tick % 3 == 0 { Some(count_procs_threads()) } else { None };
                    let svc_states = if tick % 3 == 0 {
                        let mut m = std::collections::HashMap::new();
                        for (u, _, _) in USER_SVC {
                            m.insert(format!("user:{u}"), systemctl_active(true, u));
                        }
                        for (u, _, _) in SYS_SVC {
                            m.insert(format!("sys:{u}"), systemctl_active(false, u));
                        }
                        Some(m)
                    } else {
                        None
                    };
                    let up = if tick % 3 == 0 { Some(upower_battery()) } else { None };
                    let gpu = if tick % 3 == 0 { nvidia_telemetry() } else { None };
                    let gpu_tel = gpu.map(|(t, p, v, ps)| {
                        format!("Temp: {} • Power: {} • VRAM: {} • P-State: {} ({})", t, p, v, ps, pci_status)
                    });

                    if let Ok(mut g) = sh.lock() {
                        g.overall = overall.round() as u32;
                        if let Some((s, mhz)) = speed {
                            g.speed_ghz = s;
                            g.freq_mhz = mhz;
                        }
                        if let Some(t) = temp {
                            g.temp = t.round() as i32;
                            g.temp_hist.push_back(t);
                            while g.temp_hist.len() > HISTORY {
                                g.temp_hist.pop_front();
                            }
                        }
                        if g.per_core.len() != core_vals.len() {
                            g.per_core = vec![VecDeque::from(vec![0.0; HISTORY]); core_vals.len()];
                        }
                        for (i, v) in core_vals.iter().enumerate() {
                            let dq = &mut g.per_core[i];
                            dq.push_back(*v);
                            while dq.len() > HISTORY {
                                dq.pop_front();
                            }
                        }
                        if !gov.is_empty() {
                            g.governor = gov;
                        }
                        if !drv.is_empty() {
                            g.driver = drv;
                        }
                        g.boost = boost;
                        g.profile = profile;
                        if !handles.is_empty() {
                            g.handles = handles;
                        }
                        if !uptime.is_empty() {
                            g.uptime = uptime;
                        }
                        g.ac_online = acp;
                        g.bat_cap = bat_cap;
                        g.bat_status = bat_status;
                        g.threshold = threshold;
                        g.fan1 = fan1;
                        g.fan2 = fan2;
                        g.fan_policy = fan_policy;
                        g.gpu_mode = gpu_mode;
                        g.power_mode = power_mode;
                        g.m_bat = m_bat;
                        g.m_status = m_status;
                        g.m_hz = m_hz;
                        g.m_dpi = m_dpi;
                        g.m_onboard = m_onboard;
                        g.m_led_mode = m_led_mode;
                        g.m_led_color = m_led_color;
                        g.m_led_period = m_led_period;
                        g.m_led_intensity = m_led_intensity;
                        g.mem_total = mem_total;
                        g.mem_used = mem_used;
                        g.mem_avail = mem_avail;
                        g.mem_cached = mem_cached;
                        g.swap_total = swap_total;
                        g.swap_used = swap_used;
                        g.committed = committed;
                        g.mem_pct = mem_pct;
                        g.mem_hist.push_back(mem_pct_f);
                        while g.mem_hist.len() > HISTORY {
                            g.mem_hist.pop_front();
                        }
                        g.swap_hist.push_back(swap_pct_f);
                        while g.swap_hist.len() > HISTORY {
                            g.swap_hist.pop_front();
                        }
                        if let Some((p, t)) = counts {
                            g.processes = p;
                            g.threads = t;
                        }
                        if let Some(m) = svc_states {
                            g.services = m;
                        }
                        if let Some((er, cap, ts)) = up {
                            g.energy_rate = er;
                            g.health_cap = cap;
                            g.time_str = ts;
                        }
                        if let Some(t) = gpu_tel {
                            g.gpu_tel = t;
                        }
                        g.ready = true;
                    }
                }
                prev = Some((ov, per));
            }
            sample_diskstats(&sh);
            sample_gpus(&sh, tick % 3 == 0);
            sample_fans(&sh);
            sample_nets(&sh);
            sample_bats(&sh);

            // Read adaptive-sampling gate once per tick; clear the force flag.
            let (pause_hidden, visible_tab, force) = match sh.lock() {
                Ok(mut g) => {
                    let f = g.force_heavy;
                    if f {
                        g.force_heavy = false;
                    }
                    (g.pause_hidden, g.visible_tab.clone(), f)
                }
                Err(_) => (true, String::new(), false),
            };
            // The two heaviest collections (process table + full unit list) only
            // sample when their tab is visible (or when pausing is disabled).
            let want_procs = !pause_hidden || visible_tab == "apps";
            let want_svc_all = !pause_hidden || visible_tab == "svcall";

            if tick % 3 == 0 {
                refresh_drive_list(&sh);
                refresh_gpu_list(&sh);
                refresh_fan_list(&sh);
                refresh_net_list(&sh);
                refresh_bat_list(&sh);
                refresh_net_extra(&sh);
                refresh_part_usage(&sh);
            }
            if want_procs && (tick % 3 == 0 || force) {
                let (pv, pc) = sample_procs(&mut proc_prev, &mut proc_total_prev, &mut io_prev);
                if let Ok(mut g) = sh.lock() {
                    g.procs = pv;
                    g.proc_total = pc;
                }
            } else if !want_procs {
                // Free the process table's memory while its tab is hidden.
                if let Ok(mut g) = sh.lock() {
                    if !g.procs.is_empty() {
                        g.procs = Vec::new();
                    }
                }
                proc_prev.clear();
                io_prev.clear();
            }
            if want_svc_all && (tick % 6 == 1 || force) {
                let mut all = list_services(false);
                all.extend(list_services(true));
                if let Ok(mut g) = sh.lock() {
                    g.svc_all = all;
                }
            } else if !want_svc_all {
                // Free the full unit list while its tab is hidden.
                if let Ok(mut g) = sh.lock() {
                    if !g.svc_all.is_empty() {
                        g.svc_all = Vec::new();
                    }
                }
            }
            // Disk S.M.A.R.T. — every 6th tick, gated by visibility.
            let want_smart = !pause_hidden || visible_tab == "drive";
            if want_smart && (tick % 6 == 2 || force) {
                sample_smart(&sh);
            }
            // ── Temperature alerts (every 3s to avoid lock contention) ──
            if tick % 3 == 0 {
                check_temp_alerts(&sh);
            }
            tick += 1;
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

// ───────────────────────── privileged script exec ─────────────────────────
fn script_path(name: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let candidates = [
        format!("/usr/libexec/asus-power-manager/scripts/{name}"),
        format!("{home}/bin/{name}"),
    ];
    for c in candidates.iter() {
        if Path::new(c).exists() {
            return c.clone();
        }
    }
    name.to_string()
}
fn run_priv(args: Vec<String>) {
    std::thread::spawn(move || {
        let _ = Command::new("sudo")
            .arg("-n")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}
fn run_user(args: Vec<String>) {
    std::thread::spawn(move || {
        if args.is_empty() {
            return;
        }
        let _ = Command::new(&args[0])
            .args(&args[1..])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

// Systemd services (unit, title, description)
const USER_SVC: [(&str, &str, &str); 6] = [
    ("9router.service", "9router AI Proxy Gateway", "Port 20128"),
    ("agentmemory.service", "AgentMemory Daemon", "Port 3111"),
    ("hermes-gateway.service", "Hermes Agent Gateway", "Telegram & Messaging"),
    ("hermes-webui.service", "Hermes Web UI", "Port 8787"),
    ("code-server.service", "VS Code Remote Server", "Port 8080"),
    ("ts-forward-watch.service", "Tailscale Port Forwarder", "Auto forward ports"),
];
const SYS_SVC: [(&str, &str, &str); 5] = [
    ("ollama.service", "Ollama Local LLM Engine", "Port 11434"),
    ("tailscaled.service", "Tailscale Mesh VPN", "Remote VPN"),
    ("sshd.service", "OpenSSH Server", "Port 22"),
    ("docker.service", "Docker Container Engine", "Runtime kontainer"),
    ("battery-charge-threshold.service", "Battery Limit 80% Service", "Hardware protection"),
];

fn systemctl_active(is_user: bool, unit: &str) -> String {
    let mut cmd = Command::new("systemctl");
    if is_user {
        cmd.arg("--user");
    }
    cmd.arg("is-active").arg(unit);
    match cmd.output() {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() {
                "inactive".into()
            } else {
                s
            }
        }
        Err(_) => "unknown".into(),
    }
}

#[allow(dead_code)]
struct SvcW {
    key: String,
    is_user: bool,
    unit: String,
    badge: gtk::Label,
    dot: gtk::Label,
    toggle: gtk::Button,
}

// ───────────────────────── drawing ─────────────────────────
fn draw_graph(cr: &gtk::cairo::Context, w: f64, h: f64, data: &VecDeque<f64>, maxv: f64) {
    cr.set_line_width(1.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.06);
    for frac in [0.25, 0.5, 0.75] {
        let y = h * frac;
        cr.move_to(0.0, y);
        cr.line_to(w, y);
        let _ = cr.stroke();
    }
    let n = data.len();
    if n < 2 || maxv <= 0.0 {
        return;
    }
    let stepx = w / (n as f64 - 1.0);
    let yv = |v: f64| h - (v.clamp(0.0, maxv) / maxv) * (h - 2.0) - 1.0;
    cr.set_line_width(1.6);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    for (i, v) in data.iter().enumerate() {
        let (x, y) = (i as f64 * stepx, yv(*v));
        if i == 0 {
            cr.move_to(x, y);
        } else {
            cr.line_to(x, y);
        }
    }
    let _ = cr.stroke_preserve();
    cr.line_to(w, h);
    cr.line_to(0.0, h);
    cr.close_path();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.15);
    let _ = cr.fill();
}

fn info_row(title: &str, group: &adw::PreferencesGroup) -> gtk::Label {
    let row = adw::ActionRow::builder().title(title).build();
    let lbl = gtk::Label::new(Some("--"));
    lbl.add_css_class("dim-label");
    lbl.set_valign(gtk::Align::Center);
    row.add_suffix(&lbl);
    group.add(&row);
    lbl
}
fn seg_button(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.set_valign(gtk::Align::Center);
    b
}

// All UI widgets that need periodic refresh.
struct Ui {
    // cpu
    row_model: adw::ActionRow,
    util_bar: gtk::LevelBar,
    core_areas: Vec<gtk::DrawingArea>,
    temp_area: gtk::DrawingArea,
    l: std::collections::HashMap<&'static str, gtk::Label>,
    // power
    row_bat: adw::ActionRow,
    bat_bar: gtk::LevelBar,
    row_drain: adw::ActionRow,
    row_health: adw::ActionRow,
    row_gpu_tel: adw::ActionRow,
    row_gpu_mode: adw::ActionRow,
    btn_gpu: [gtk::Button; 3],   // hybrid, integrated, dedicated
    btn_mode: [gtk::Button; 3],  // powersave, performance, auto
    row_fan_rpm: adw::ActionRow,
    row_fan_ctrl: adw::ActionRow,
    btn_fan: [gtk::Button; 3],   // silent(2), normal(0), turbo(1)
    switch_threshold: adw::SwitchRow,
    row_cpu_mon: adw::ActionRow,
    sync: Rc<Cell<bool>>,
    // mouse
    row_m_bat: adw::ActionRow,
    m_bat_bar: gtk::LevelBar,
    row_m_hz: adw::ActionRow,
    hz_btns: Vec<(u32, gtk::Button)>,
    row_m_dpi: adw::ActionRow,
    scale_dpi: gtk::Scale,
    switch_onboard: adw::SwitchRow,
    m_sync: Rc<Cell<bool>>,
    m_pending_dpi: Rc<Cell<Option<(u32, Instant)>>>,
    m_pending_hz: Rc<Cell<Option<(u32, Instant)>>>,
    m_pending_led_mode: Rc<Cell<Option<(u32, Instant)>>>,
    m_pending_led_color: Rc<Cell<Option<((u8, u8, u8), Instant)>>>,
    row_m_led_prev: adw::ActionRow,
    m_led_swatch: gtk::DrawingArea,
    m_led_btns: Vec<(u32, gtk::Button)>,
    row_m_led_bright: adw::ActionRow,
    scale_m_led_bright: gtk::Scale,
    row_m_led_speed: adw::ActionRow,
    scale_m_led_speed: gtk::Scale,
    services: Vec<SvcW>,
    // memory
    row_mem: adw::ActionRow,
    mem_bar: gtk::LevelBar,
    mem_area: gtk::DrawingArea,
    swap_area: gtk::DrawingArea,
    mem_lbl: std::collections::HashMap<&'static str, gtk::Label>,
    apps: AppsUi,
    svc_all: SvcAllUi,
    drives: RefCell<Vec<DriveUi>>,
    gpus: RefCell<Vec<GpuUi>>,
    fans: RefCell<Vec<FanUi>>,
    nets: RefCell<Vec<NetUi>>,
    bats: RefCell<Vec<BatUi>>,
    // hotplug rebuild handles
    shared: Arc<Mutex<Shared>>,
    stack: adw::ViewStack,
    sidebar: gtk::ListBox,
    dyn_rows: RefCell<Vec<gtk::ListBoxRow>>,
    dyn_pages: RefCell<Vec<gtk::Widget>>,
    dyn_sig: RefCell<String>,
}

impl Ui {
    fn set_active(btn: &gtk::Button, on: bool) {
        if on {
            btn.add_css_class("suggested-action");
        } else {
            btn.remove_css_class("suggested-action");
        }
    }
    fn refresh(&self, g: &Shared) {
        if !g.ready {
            return;
        }
        // CPU header + info
        self.row_model.set_title(&g.model);
        self.row_model.set_subtitle(&format!(
            "Utilization: {}% • Speed: {} GHz",
            g.overall,
            if g.speed_ghz.is_empty() { "--" } else { &g.speed_ghz }
        ));
        self.util_bar.set_value(g.overall as f64);
        let set = |k: &str, v: &str| {
            if let Some(lbl) = self.l.get(k) {
                lbl.set_text(v);
            }
        };
        set("speed", &format!("{} GHz", if g.speed_ghz.is_empty() { "--" } else { &g.speed_ghz }));
        set("base", &if g.base_ghz.is_empty() { "—".into() } else { format!("{} GHz", g.base_ghz) });
        set("logical", &g.logical_str);
        set("sockets", &g.sockets);
        set("virt", &g.virt);
        set("vm", &g.vm);
        set("l1", &g.l1);
        set("l2", &g.l2);
        set("l3", &g.l3);
        set("driver", if g.driver.is_empty() { "—" } else { &g.driver });
        set("gov", if g.governor.is_empty() { "—" } else { &g.governor });
        set("pth", &format!("{} / {} / {}", g.processes, g.threads, if g.handles.is_empty() { "--" } else { &g.handles }));
        set("uptime", if g.uptime.is_empty() { "--" } else { &g.uptime });
        set("temp", &format!("{} °C", g.temp));
        for a in &self.core_areas {
            a.queue_draw();
        }
        self.temp_area.queue_draw();

        // Battery
        let pct_num: f64 = g.bat_cap.parse().unwrap_or(0.0);
        self.bat_bar.set_value(pct_num);
        self.row_bat.set_title(&format!("Battery: {}%", g.bat_cap));
        if g.ac_online {
            if g.threshold == "80" && pct_num >= 79.0 {
                self.row_bat.set_subtitle("🔌 Charger Connected — Standby (80% Limit)");
                self.row_drain.set_subtitle("Powered by Adapter (Battery standby)");
            } else if g.bat_status.eq_ignore_ascii_case("charging") {
                let tgt = if g.threshold != "100" { format!(" (Target {}%)", g.threshold) } else { String::new() };
                self.row_bat.set_subtitle(&format!("⚡ Charging{}", tgt));
                self.row_drain.set_subtitle(&format!("{} (Charging)", if g.energy_rate.is_empty() { "Active" } else { &g.energy_rate }));
            } else {
                self.row_bat.set_subtitle(&format!("🔌 Charger Connected ({})", g.bat_status));
                self.row_drain.set_subtitle("Adapter Active");
            }
        } else {
            self.row_bat.set_subtitle("🔋 Battery Mode (Not Charging)");
            self.row_drain.set_subtitle(&format!("{} (Load Draw)", if g.energy_rate.is_empty() { "Active" } else { &g.energy_rate }));
        }
        let est = if !g.time_str.is_empty() && !g.ac_online { format!(" | Estimate: {}", g.time_str) } else { String::new() };
        self.row_health.set_subtitle(&format!("Cell Health: {}{}", if g.health_cap.is_empty() { "Normal" } else { &g.health_cap }, est));

        // Threshold switch (guarded)
        let want = g.threshold == "80";
        if self.switch_threshold.is_active() != want {
            self.sync.set(true);
            self.switch_threshold.set_active(want);
            self.sync.set(false);
        }

        // GPU
        if !g.gpu_tel.is_empty() {
            self.row_gpu_tel.set_subtitle(&g.gpu_tel);
        }
        for b in &self.btn_gpu {
            Self::set_active(b, false);
        }
        match g.gpu_mode.as_str() {
            "integrated" | "1" => {
                Self::set_active(&self.btn_gpu[1], true);
                self.row_gpu_mode.set_subtitle("Mode: AMD iGPU Only (NVIDIA standby)");
            }
            "dedicated" | "2" => {
                Self::set_active(&self.btn_gpu[2], true);
                self.row_gpu_mode.set_subtitle("Mode: NVIDIA Dedicated (Full performance)");
            }
            _ => {
                Self::set_active(&self.btn_gpu[0], true);
                self.row_gpu_mode.set_subtitle("Mode: Hybrid Optimus (On-Demand)");
            }
        }

        // CPU power mode buttons
        for b in &self.btn_mode {
            Self::set_active(b, false);
        }
        let mi = match g.power_mode.as_str() {
            "powersave" => 0,
            "performance" => 1,
            _ => 2,
        };
        Self::set_active(&self.btn_mode[mi], true);

        // Fan
        self.row_fan_rpm.set_subtitle(&format!("CPU Fan: {} RPM  |  GPU Fan: {} RPM", g.fan1, g.fan2));
        for b in &self.btn_fan {
            Self::set_active(b, false);
        }
        let (fi, flabel) = match g.fan_policy.as_str() {
            "2" => (0, "Silent (Low speed)"),
            "1" => (2, "Turbo / Overboost (Fast cooling)"),
            _ => (1, "Normal / Balanced (Automatic)"),
        };
        Self::set_active(&self.btn_fan[fi], true);
        self.row_fan_ctrl.set_subtitle(&format!("Active Status: {}", flabel));

        // CPU monitor
        self.row_cpu_mon.set_subtitle(&format!(
            "Governor: {} @ {} MHz | Turbo: {} | Profile: {}",
            g.governor.to_uppercase(),
            g.freq_mhz,
            g.boost,
            g.profile
        ));

        // ── Mouse ──
        let mbat: f64 = g.m_bat.parse().unwrap_or(90.0);
        self.m_bat_bar.set_value(mbat);
        self.row_m_bat.set_title(&format!("G304 Mouse Battery: {}%", g.m_bat));
        let mstat = if g.m_status.eq_ignore_ascii_case("discharging")
            || g.m_status.eq_ignore_ascii_case("charging")
            || g.m_status.eq_ignore_ascii_case("full")
        {
            "Connected (Active)"
        } else {
            "Standby / Sleep"
        };
        self.row_m_bat.set_subtitle(&format!("Status: {} • Connection: Lightspeed Receiver", mstat));

        // Hz (honor pending user choice)
        let mut hz: u32 = g.m_hz.parse().unwrap_or(1000);
        if let Some((p, ts)) = self.m_pending_hz.get() {
            if hz == p {
                self.m_pending_hz.set(None);
            } else if ts.elapsed().as_secs() < 6 {
                hz = p;
            } else {
                self.m_pending_hz.set(None);
            }
        }
        self.row_m_hz.set_subtitle(&format!(
            "Active: {} Hz ({})",
            hz,
            if hz == 1000 { "1ms Peak" } else { "Battery Saver" }
        ));
        for (v, b) in &self.hz_btns {
            Self::set_active(b, *v == hz);
        }

        // DPI (honor pending; skip while user dragging via m_sync)
        let mut dpi: u32 = g.m_dpi.parse().unwrap_or(1600);
        if let Some((p, ts)) = self.m_pending_dpi.get() {
            if dpi == p {
                self.m_pending_dpi.set(None);
            } else if ts.elapsed().as_secs() < 6 {
                dpi = p;
            } else {
                self.m_pending_dpi.set(None);
            }
        }
        self.row_m_dpi.set_subtitle(&format!("{} DPI", dpi));
        if !self.m_sync.get() && (self.scale_dpi.value() as u32) != dpi {
            self.m_sync.set(true);
            self.scale_dpi.set_value(dpi as f64);
            self.m_sync.set(false);
        }

        // Onboard switch (guarded)
        if self.switch_onboard.is_active() != g.m_onboard {
            self.m_sync.set(true);
            self.switch_onboard.set_active(g.m_onboard);
            self.m_sync.set(false);
        }

        // Mouse LED (honor pending state to eliminate race condition / flicker)
        let mut m_mode: u32 = g.m_led_mode.parse().unwrap_or(1);
        if let Some((p, ts)) = self.m_pending_led_mode.get() {
            if m_mode == p {
                self.m_pending_led_mode.set(None);
            } else if ts.elapsed().as_secs() < 6 {
                m_mode = p;
            } else {
                self.m_pending_led_mode.set(None);
            }
        }
        let mut color_rgb = parse_hex_color(&g.m_led_color);
        if let Some((p, ts)) = self.m_pending_led_color.get() {
            if color_rgb == p {
                self.m_pending_led_color.set(None);
            } else if ts.elapsed().as_secs() < 6 {
                color_rgb = p;
            } else {
                self.m_pending_led_color.set(None);
            }
        }
        let (mr, mg, mb) = color_rgb;
        let m_int: u32 = g.m_led_intensity.parse().unwrap_or(100);
        let m_per: u32 = g.m_led_period.parse().unwrap_or(3000);
        let m_mode_label = mouse_led_mode_name(m_mode);
        self.row_m_led_prev.set_subtitle(&format!(
            "Mode: {} • Hex: #{:02X}{:02X}{:02X} • Brightness: {}% • Speed: {:.1}s",
            m_mode_label, mr, mg, mb, m_int, m_per as f64 / 1000.0
        ));
        self.m_led_swatch.queue_draw();
        for (v, b) in &self.m_led_btns {
            Self::set_active(b, *v == m_mode);
        }
        if !self.m_sync.get() {
            self.m_sync.set(true);
            self.scale_m_led_bright.set_value(m_int as f64);
            self.row_m_led_bright.set_subtitle(&format!("{}%", m_int));
            self.scale_m_led_speed.set_value(m_per as f64 / 1000.0);
            self.row_m_led_speed.set_subtitle(&format!("{:.1}s ({} ms)", m_per as f64 / 1000.0, m_per));
            self.m_sync.set(false);
        }

        // ── Services ──
        for s in &self.services {
            let state = match g.services.get(&s.key) {
                Some(v) => v.as_str(),
                None => continue,
            };
            s.badge.remove_css_class("badge-run");
            s.badge.remove_css_class("badge-stop");
            s.badge.remove_css_class("badge-fail");
            s.dot.remove_css_class("dot-run");
            s.dot.remove_css_class("dot-stop");
            s.dot.remove_css_class("dot-fail");
            s.toggle.remove_css_class("destructive-action");
            s.toggle.remove_css_class("suggested-action");
            match state {
                "active" => {
                    s.badge.set_text("Active");
                    s.badge.add_css_class("badge-run");
                    s.dot.add_css_class("dot-run");
                    s.toggle.set_label("Stop");
                    s.toggle.add_css_class("destructive-action");
                }
                "failed" => {
                    s.badge.set_text("Failed");
                    s.badge.add_css_class("badge-fail");
                    s.dot.add_css_class("dot-fail");
                    s.toggle.set_label("Start");
                    s.toggle.add_css_class("suggested-action");
                }
                "unknown" => {
                    s.badge.set_text("Unknown");
                    s.badge.add_css_class("badge-stop");
                    s.dot.add_css_class("dot-stop");
                    s.toggle.set_label("Start");
                }
                _ => {
                    s.badge.set_text("Off");
                    s.badge.add_css_class("badge-stop");
                    s.dot.add_css_class("dot-stop");
                    s.toggle.set_label("Start");
                    s.toggle.add_css_class("suggested-action");
                }
            }
        }

        // ── Memory ──
        self.row_mem.set_title(&format!("Memory: {} total", fmt_gib(g.mem_total)));
        self.row_mem.set_subtitle(&format!("Used {} • {}%", fmt_gib(g.mem_used), g.mem_pct));
        self.mem_bar.set_value(g.mem_pct as f64);
        let mset = |k: &str, v: String| {
            if let Some(l) = self.mem_lbl.get(k) {
                l.set_text(&v);
            }
        };
        mset("used", fmt_gib(g.mem_used));
        mset("avail", fmt_gib(g.mem_avail));
        mset("committed", fmt_gib(g.committed));
        mset("cached", fmt_gib(g.mem_cached));
        mset("swapused", if g.swap_used == 0.0 { "0".into() } else { fmt_gib(g.swap_used) });
        mset("swapavail", fmt_gib((g.swap_total - g.swap_used).max(0.0)));
        mset("dtype", g.dimm_type.clone());
        mset("dform", g.dimm_form.clone());
        mset("dspeed", g.dimm_speed.clone());
        mset("dslots", g.dimm_slots.clone());
        self.mem_area.queue_draw();
        self.swap_area.queue_draw();

        // ── Aplikasi & Proses ──
        self.apps.row_hdr.set_subtitle(&format!("{} processes running", g.proc_total));
        {
            let q = self.apps.query.borrow().clone();
            let matches = |p: &ProcInfo| {
                q.is_empty()
                    || p.name.to_lowercase().contains(&q)
                    || p.pid.to_string().contains(&q)
                    || p.ports.contains(&q)
            };
            let filtered: Vec<&ProcInfo> = g.procs.iter().filter(|p| matches(p)).take(200).collect();
            let mut sig = String::with_capacity(1024);
            sig.push_str(&q);
            sig.push('|');
            for p in &filtered {
                sig.push_str(&format!("{}:{:.0}:{}:{:.0}:{};", p.pid, p.cpu, p.rss_kb, p.io_bps, p.ports));
            }
            if *self.apps.sig.borrow() != sig {
                *self.apps.sig.borrow_mut() = sig;
                while let Some(r) = self.apps.list.first_child() {
                    self.apps.list.remove(&r);
                }
                let want = self.apps.sel.get();
                for p in filtered.iter().take(150) {
                    let row = gtk::ListBoxRow::new();
                    row.set_widget_name(&p.pid.to_string());
                    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    bx.set_margin_start(12);
                    bx.set_margin_end(12);
                    bx.set_margin_top(6);
                    bx.set_margin_bottom(6);
                    let cell = |t: String, w: i32, x: f32, dim: bool| {
                        let l = gtk::Label::new(Some(&t));
                        l.set_xalign(x);
                        if w > 0 {
                            l.set_size_request(w, -1);
                        } else {
                            l.set_hexpand(true);
                            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        }
                        if dim {
                            l.add_css_class("dim-label");
                        }
                        l
                    };
                    bx.append(&cell(p.name.clone(), 0, 0.0, false));
                    bx.append(&cell(p.pid.to_string(), 56, 1.0, true));
                    bx.append(&cell(format!("{:.0}%", p.cpu), 48, 1.0, false));
                    bx.append(&cell(fmt_kib(p.rss_kb), 78, 1.0, false));
                    bx.append(&cell(if p.swap_kb == 0 { "0".into() } else { fmt_kib(p.swap_kb) }, 66, 1.0, true));
                    bx.append(&cell(if p.io_known { fmt_rate(p.io_bps) } else { "—".into() }, 78, 1.0, true));
                    let port_cell = cell(if p.ports.is_empty() { "—".into() } else { p.ports.clone() }, 96, 1.0, false);
                    if !p.ports.is_empty() {
                        port_cell.add_css_class("svc-run");
                    }
                    bx.append(&port_cell);
                    row.set_child(Some(&bx));
                    self.apps.list.append(&row);
                    if want == Some(p.pid) {
                        self.apps.list.select_row(Some(&row));
                    }
                }
            }
        }

        // ── Semua Layanan (full services) ──
        {
            let total = g.svc_all.len();
            let running = g.svc_all.iter().filter(|u| u.sub == "running").count();
            let failed = g.svc_all.iter().filter(|u| u.active == "failed" || u.sub == "failed").count();
            self.svc_all
                .row_hdr
                .set_subtitle(&format!("{total} units • {running} running • {failed} failed"));
            let filt = self.svc_all.filter.get();
            let pass = |u: &SvcUnit| match filt {
                1 => u.sub == "running",
                2 => u.active == "failed" || u.sub == "failed",
                _ => true,
            };
            let mut sig = format!("f{filt};");
            for u in &g.svc_all {
                if pass(u) {
                    sig.push_str(&u.unit);
                    sig.push(':');
                    sig.push_str(&u.sub);
                    sig.push(';');
                }
            }
            if *self.svc_all.sig.borrow() != sig {
                *self.svc_all.sig.borrow_mut() = sig;
                while let Some(r) = self.svc_all.list.first_child() {
                    self.svc_all.list.remove(&r);
                }
                let want = self.svc_all.sel.borrow().clone();
                for (label, grp_user) in [("User Services", true), ("System Services", false)] {
                    let mut items: Vec<&SvcUnit> =
                        g.svc_all.iter().filter(|u| u.is_user == grp_user && pass(u)).collect();
                    if items.is_empty() {
                        continue;
                    }
                    items.sort_by(|a, b| a.unit.cmp(&b.unit));
                    let hr = gtk::ListBoxRow::new();
                    hr.set_selectable(false);
                    hr.set_activatable(false);
                    let hl = gtk::Label::new(Some(label));
                    hl.set_xalign(0.0);
                    hl.add_css_class("dim-label");
                    hl.set_margin_top(8);
                    hl.set_margin_bottom(4);
                    hl.set_margin_start(12);
                    hr.set_child(Some(&hl));
                    self.svc_all.list.append(&hr);
                    for u in items {
                        let row = gtk::ListBoxRow::new();
                        row.set_widget_name(&format!("{}:{}", if u.is_user { "U" } else { "S" }, u.unit));
                        let bx = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                        bx.set_margin_start(12);
                        bx.set_margin_end(12);
                        bx.set_margin_top(6);
                        bx.set_margin_bottom(6);
                        let dot = gtk::Label::new(Some("●"));
                        dot.add_css_class(match (u.active.as_str(), u.sub.as_str()) {
                            (_, "running") => "svc-run",
                            ("failed", _) | (_, "failed") => "svc-fail",
                            _ => "svc-idle",
                        });
                        let name = gtk::Label::new(Some(&u.unit));
                        name.set_xalign(0.0);
                        name.set_hexpand(true);
                        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        let sub = gtk::Label::new(Some(&u.sub));
                        sub.set_size_request(90, -1);
                        sub.set_xalign(1.0);
                        sub.add_css_class("dim-label");
                        let mem = gtk::Label::new(Some(&if u.mem > 0 { fmt_bytes(u.mem) } else { "—".into() }));
                        mem.set_size_request(90, -1);
                        mem.set_xalign(1.0);
                        mem.add_css_class("dim-label");
                        bx.append(&dot);
                        bx.append(&name);
                        bx.append(&sub);
                        bx.append(&mem);
                        row.set_child(Some(&bx));
                        self.svc_all.list.append(&row);
                        if want.as_ref().map(|(un, iu)| un == &u.unit && *iu == u.is_user).unwrap_or(false) {
                            self.svc_all.list.select_row(Some(&row));
                        }
                    }
                }
            }
        }

        // ── Dynamic tabs (GPUs + fans + nets + drives + bats): rebuild on change, then update ──
        let sig = format!(
            "{}#{}#{}#{}#{}",
            gpu_signature(&g.gpus),
            fan_signature(&g.fans),
            net_signature(&g.nets),
            drive_signature(&g.drives),
            bat_signature(&g.bats)
        );
        if *self.dyn_sig.borrow() != sig {
            self.rebuild_dynamic(&g.gpus, &g.fans, &g.nets, &g.drives, &g.bats);
            *self.dyn_sig.borrow_mut() = sig;
        }
        for bu in self.bats.borrow().iter() {
            if let Some(b) = g.bats.get(bu.idx) {
                let bset = |k: &str, v: String| {
                    if let Some(l) = bu.lbl.get(k) {
                        l.set_text(&v);
                    }
                };
                bset("pct", format!("{:.0}%", b.percent));
                bset("state", if b.state.is_empty() { "—".into() } else { b.state.clone() });
                bset("serial", if b.serial.is_empty() { "—".into() } else { b.serial.clone() });
                if b.is_system {
                    bset("volt", format!("{:.2} V", b.voltage));
                    bset("power", format!("{:.2} W", b.power));
                    bset("cycles", format!("{}", b.cycles));
                    bset("tech", if b.technology.is_empty() { "—".into() } else { b.technology.clone() });
                    bset("health", format!("{:.0}%", b.capacity_health));
                    bset("ef", format!("{:.1} Wh", b.energy_full));
                    bset("efd", format!("{:.1} Wh", b.energy_full_design));
                    bset("vmin", format!("{:.2} V", b.voltage_min_design));
                    bset("thr", b.charge_threshold.clone());
                    if let Some(ps) = &bu.power_scale {
                        let mx = b.power_hist.iter().cloned().fold(0.0_f64, f64::max);
                        ps.set_text(&format!("{mx:.2} W"));
                    }
                    if let Some(pa) = &bu.power_area {
                        pa.queue_draw();
                    }
                }
                bu.pct_area.queue_draw();
            }
        }
        for nu in self.nets.borrow().iter() {
            if let Some(n) = g.nets.get(nu.idx) {
                let nset = |k: &str, v: String| {
                    if let Some(l) = nu.lbl.get(k) {
                        l.set_text(&v);
                    }
                };
                nset("rspeed", fmt_bitrate(n.rx_bps));
                nset("sspeed", fmt_bitrate(n.tx_bps));
                nset("trx", fmt_bits_total(n.total_rx));
                nset("ttx", fmt_bits_total(n.total_tx));
                nset("status", if n.status.is_empty() { "—".into() } else { n.status.clone() });
                nset("ipv4", if n.ipv4.is_empty() { "—".into() } else { n.ipv4.clone() });
                nset("ipv6", if n.ipv6.is_empty() { "—".into() } else { n.ipv6.clone() });
                if n.is_wireless {
                    nset("ssid", if n.ssid.is_empty() { "—".into() } else { n.ssid.clone() });
                    nset("signal", if n.signal.is_empty() { "—".into() } else { n.signal.clone() });
                    nset("freq", if n.freq.is_empty() { "—".into() } else { n.freq.clone() });
                }
                let mx = n.rx_hist.iter().chain(n.tx_hist.iter()).cloned().fold(0.0_f64, f64::max);
                nu.scale_lbl.set_text(&fmt_bitrate(mx));
                nu.area.queue_draw();
            }
            // Tailnet device list — rebuild rows only when the peer set changes.
            if let Some(grp) = &nu.ts_group {
                // Reflect backend state on the Connect/Disconnect buttons.
                if let Some(b) = &nu.ts_connect {
                    b.set_sensitive(!g.ts_running);
                }
                if let Some(b) = &nu.ts_disconnect {
                    b.set_sensitive(g.ts_running);
                }
                let mut sig = String::new();
                for p in g.ts_peers.iter() {
                    sig.push_str(&p.name);
                    sig.push('|');
                    sig.push_str(&p.ip);
                    sig.push('|');
                    sig.push(if p.online { '1' } else { '0' });
                    sig.push(';');
                }
                if *nu.ts_sig.borrow() != sig {
                    *nu.ts_sig.borrow_mut() = sig;
                    for r in nu.ts_rows.borrow_mut().drain(..) {
                        grp.remove(&r);
                    }
                    let mut rows = nu.ts_rows.borrow_mut();
                    if g.ts_peers.is_empty() {
                        let row = adw::ActionRow::builder()
                            .title("No devices found")
                            .subtitle("tailscale status returned no peers")
                            .build();
                        grp.add(&row);
                        rows.push(row);
                    } else {
                        for p in g.ts_peers.iter() {
                            let title = if p.is_self {
                                format!("{} (this device)", p.name)
                            } else {
                                p.name.clone()
                            };
                            let row = adw::ActionRow::builder()
                                .title(title)
                                .subtitle(format!("{} • {}", p.ip, p.os))
                                .build();
                            let badge = gtk::Label::new(Some(if p.online { "Online" } else { "Offline" }));
                            badge.add_css_class(if p.online { "svc-run" } else { "svc-idle" });
                            badge.set_valign(gtk::Align::Center);
                            row.add_suffix(&badge);
                            grp.add(&row);
                            rows.push(row);
                        }
                    }
                }
            }
        }
        for fu in self.fans.borrow().iter() {
            if let Some(f) = g.fans.get(fu.idx) {
                fu.rpm_lbl.set_text(&format!("{:.0} RPM", f.rpm.max(0.0)));
                let mx = f.hist.iter().cloned().fold(0.0_f64, f64::max);
                fu.scale_lbl.set_text(&format!("{mx:.0} RPM"));
                fu.area.queue_draw();
            }
        }
        for gu in self.gpus.borrow().iter() {
            if let Some(gp) = g.gpus.get(gu.idx) {
                let gset = |k: &str, v: String| {
                    if let Some(l) = gu.lbl.get(k) {
                        l.set_text(&v);
                    }
                };
                gset("util", format!("{:.0}%", gp.util.max(0.0)));
                gset("clock", format!("{} / {}", fmt_clock(gp.clock_cur), fmt_clock(gp.clock_max)));
                let power = if gp.power_draw <= 0.0 {
                    "—".to_string()
                } else if gp.power_limit > 0.0 {
                    format!("{:.2} W / {:.0} W", gp.power_draw, gp.power_limit)
                } else {
                    format!("{:.2} W", gp.power_draw)
                };
                gset("power", power);
                gset("mem", format!("{} / {}", fmt_bytes(gp.mem_used), fmt_bytes(gp.mem_total)));
                gset("memclk", format!("{} / {}", fmt_clock(gp.mem_clock_cur), fmt_clock(gp.mem_clock_max)));
                let encdec = if gp.enc_util < 0.0 {
                    "—".to_string()
                } else {
                    format!("{:.0}% / {:.0}%", gp.enc_util, gp.dec_util.max(0.0))
                };
                gset("encdec", encdec);
                gset("temp", if gp.temp > 0.0 { format!("{:.0} °C", gp.temp) } else { "—".into() });
                gset("pcie", if gp.pcie.is_empty() { "—".to_string() } else { gp.pcie.clone() });
                gu.util_area.queue_draw();
                gu.mem_area.queue_draw();
            }
        }
        for du in self.drives.borrow().iter() {
            if let Some(d) = g.drives.get(du.idx) {
                let dset = |k: &str, v: String| {
                    if let Some(l) = du.lbl.get(k) {
                        l.set_text(&v);
                    }
                };
                dset("rspeed", fmt_rate(d.read_bps));
                dset("wspeed", fmt_rate(d.write_bps));
                dset("tread", fmt_bytes(d.total_read));
                dset("twrite", fmt_bytes(d.total_written));
                dset("active", format!("{:.0}%", d.active_pct));
                dset("resp", format!("{:.2} ms", d.resp_ms));
                let mx = d.thru_hist.iter().cloned().fold(0.0_f64, f64::max);
                du.thru_scale.set_text(&fmt_rate(mx));
                for (i, (bar, lb)) in du.parts.iter().enumerate() {
                    if let Some(p) = d.partitions.get(i) {
                        if p.size > 0 && p.used > 0 {
                            bar.set_visible(true);
                            bar.set_value(p.used as f64 / p.size as f64 * 100.0);
                            lb.set_text(&format!("{} / {}", fmt_bytes(p.used), fmt_bytes(p.size)));
                        } else {
                            bar.set_visible(false);
                            lb.set_text(&fmt_bytes(p.size));
                        }
                    }
                }
                // ── S.M.A.R.T. Health Status for this drive ──
                if let Some(info) = g.smart_data.iter().find(|s| s.dev == du.dev).or_else(|| g.smart_data.get(du.idx)) {
                    let sset = |k: &str, v: &str| {
                        if let Some(l) = du.smart_lbl.get(k) {
                            l.set_text(v);
                        }
                    };
                    if info.smartctl_missing {
                        sset("temp", "smartmontools not installed");
                    } else {
                        if info.temp.is_empty() || info.temp == "Unknown" {
                            sset("temp", "—");
                        } else {
                            sset("temp", &format!("{}°C", info.temp));
                        }
                        let dash = |s: &str| if s.is_empty() || s == "Unknown" || s == "N/A" { "—".to_string() } else { s.to_string() };
                        sset("hours", &dash(&info.power_on_hours));
                        sset("cycles", &dash(&info.power_cycles));
                        sset("realloc", &dash(&info.reallocated));
                        if info.percent_used.is_empty() || info.percent_used == "N/A" {
                            sset("pct", "—");
                        } else {
                            sset("pct", &format!("{}%", info.percent_used));
                        }
                        sset("written", &dash(&info.data_written));
                    }
                    if info.smartctl_missing {
                        du.health_lbl.set_markup("<span foreground='#888888'>Install smartmontools</span>");
                    } else if info.health.contains("PASSED") {
                        du.health_lbl.set_markup("<span foreground='#4caf50'>PASSED</span>");
                    } else if info.health.contains("FAILED") {
                        du.health_lbl.set_markup("<span foreground='#f44336'>FAILED</span>");
                    } else {
                        du.health_lbl.set_text(&info.health);
                    }
                }
                du.active_area.queue_draw();
                du.thru_area.queue_draw();
            }
        }
    }

    // Rebuild the dynamic sidebar rows and stack pages (GPUs then drives) after a
    // hotplug/enumeration change. They live between Memory (index 1) and Power,
    // i.e. starting at index 2.
    fn rebuild_dynamic(&self, gpus: &[GpuInfo], fans: &[FanInfo], nets: &[NetInfo], drives: &[DriveInfo], bats: &[BatInfo]) {
        for r in self.dyn_rows.borrow().iter() {
            self.sidebar.remove(r);
        }
        self.dyn_rows.borrow_mut().clear();
        for p in self.dyn_pages.borrow().iter() {
            self.stack.remove(p);
        }
        self.dyn_pages.borrow_mut().clear();
        self.gpus.borrow_mut().clear();
        self.fans.borrow_mut().clear();
        self.nets.borrow_mut().clear();
        self.bats.borrow_mut().clear();
        self.drives.borrow_mut().clear();

        let mut pos = 3i32; // after CPU(0), Memory(1), Speed Test(2)
        if !gpus.is_empty() {
            let inner = adw::ViewStack::new();
            let mut tabs = Vec::new();
            for (i, info) in gpus.iter().enumerate() {
                let (page, gu) = build_gpu_page(&self.shared, i, info);
                let name = format!("g{i}");
                inner.add_titled(&page, Some(&name), &format!("GPU {i} ({})", info.kind));
                tabs.push((name, format!("GPU {i} ({})", info.kind)));
                self.gpus.borrow_mut().push(gu);
            }
            let container = group_container(&inner, &tabs, "lucide-gpu");
            self.stack.add_titled(&container, Some("gpu"), "GPU");
            let row = sidebar_row("gpu", "GPU", "lucide-gpu");
            self.sidebar.insert(&row, pos);
            pos += 1;
            self.dyn_rows.borrow_mut().push(row);
            self.dyn_pages.borrow_mut().push(container.upcast());
        }
        if !fans.is_empty() {
            let inner = adw::ViewStack::new();
            let mut tabs = Vec::new();
            for (i, info) in fans.iter().enumerate() {
                let (page, fu) = build_fan_page(&self.shared, i, info);
                let name = format!("f{i}");
                inner.add_titled(&page, Some(&name), &format!("Fan {i} ({})", info.label));
                tabs.push((name, format!("Fan {i} ({})", info.label)));
                self.fans.borrow_mut().push(fu);
            }
            let container = group_container(&inner, &tabs, "lucide-fan");
            self.stack.add_titled(&container, Some("fan"), "Fan");
            let row = sidebar_row("fan", "Fan", "lucide-fan");
            self.sidebar.insert(&row, pos);
            pos += 1;
            self.dyn_rows.borrow_mut().push(row);
            self.dyn_pages.borrow_mut().push(container.upcast());
        }
        if !nets.is_empty() {
            let inner = adw::ViewStack::new();
            let mut tabs = Vec::new();
            for (i, info) in nets.iter().enumerate() {
                let (page, nu) = build_net_page(&self.shared, i, info);
                let name = format!("n{i}");
                inner.add_titled(&page, Some(&name), &format!("{} ({})", info.kind, info.iface));
                tabs.push((name, format!("{} ({})", info.kind, info.iface)));
                self.nets.borrow_mut().push(nu);
            }
            let container = group_container(&inner, &tabs, "lucide-network");
            self.stack.add_titled(&container, Some("net"), "Network");
            let row = sidebar_row("net", "Network", "lucide-network");
            self.sidebar.insert(&row, pos);
            pos += 1;
            self.dyn_rows.borrow_mut().push(row);
            self.dyn_pages.borrow_mut().push(container.upcast());
        }
        if !drives.is_empty() {
            let inner = adw::ViewStack::new();
            let mut tabs = Vec::new();
            for (i, info) in drives.iter().enumerate() {
                let (page, du) = build_drive_page(&self.shared, i, info);
                let name = format!("d{i}");
                inner.add_titled(&page, Some(&name), &format!("{} {} ({})", info.kind, i, info.dev));
                tabs.push((name, format!("{} {} ({})", info.kind, i, info.dev)));
                self.drives.borrow_mut().push(du);
            }
            let container = group_container(&inner, &tabs, "lucide-hard-drive");
            self.stack.add_titled(&container, Some("drive"), "Drive");
            let row = sidebar_row("drive", "Drive", "lucide-hard-drive");
            self.sidebar.insert(&row, pos);
            pos += 1;
            self.dyn_rows.borrow_mut().push(row);
            self.dyn_pages.borrow_mut().push(container.upcast());
        }
        if !bats.is_empty() {
            let inner = adw::ViewStack::new();
            let mut tabs = Vec::new();
            for (i, info) in bats.iter().enumerate() {
                let (page, bu) = build_bat_page(&self.shared, i, info);
                let name = format!("b{i}");
                inner.add_titled(&page, Some(&name), &format!("Battery {i} ({})", info.name));
                tabs.push((name, format!("Battery {i} ({})", info.name)));
                self.bats.borrow_mut().push(bu);
            }
            let container = group_container(&inner, &tabs, "lucide-battery");
            self.stack.add_titled(&container, Some("bat"), "Battery");
            let row = sidebar_row("bat", "Battery", "lucide-battery");
            self.sidebar.insert(&row, pos);
            self.dyn_rows.borrow_mut().push(row);
            self.dyn_pages.borrow_mut().push(container.upcast());
        }
        // If the previously selected row was removed, fall back to the first tab.
        if self.sidebar.selected_row().is_none() {
            if let Some(first) = self.sidebar.row_at_index(0) {
                self.sidebar.select_row(Some(&first));
            }
        }
    }
}

fn build_cpu_page(shared: &Arc<Mutex<Shared>>, ui_core: &mut Vec<gtk::DrawingArea>) -> (adw::PreferencesPage, adw::ActionRow, gtk::LevelBar, gtk::DrawingArea, std::collections::HashMap<&'static str, gtk::Label>) {
    let logical = shared.lock().map(|g| g.logical).unwrap_or(1);
    let page = adw::PreferencesPage::new();

    let g_head = adw::PreferencesGroup::builder().title("Processor").build();
    let row_model = adw::ActionRow::builder()
        .title("Loading CPU model...")
        .subtitle("Utilization: --% • Speed: -- GHz")
        .build();
    let util_bar = gtk::LevelBar::builder().min_value(0.0).max_value(100.0).valign(gtk::Align::Center).build();
    util_bar.set_size_request(110, 16);
    row_model.add_suffix(&util_bar);
    g_head.add(&row_model);
    page.add(&g_head);

    let g_cores = adw::PreferencesGroup::builder()
        .title("Per-Core Utilization (1 minute)")
        .description("Realtime usage per logical core (0–100%)")
        .build();
    let grid = gtk::Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(8);
    grid.set_column_homogeneous(true);
    grid.set_margin_top(6);
    grid.set_margin_bottom(6);
    let cols = if logical >= 8 { 4 } else { 2 };
    for i in 0..logical {
        let area = gtk::DrawingArea::new();
        area.set_content_width(110);
        area.set_content_height(58);
        area.set_hexpand(true);
        area.add_css_class("cpu-graph-frame");
        let sh = shared.clone();
        area.set_draw_func(move |_a, cr, w, h| {
            if let Ok(g) = sh.lock() {
                if let Some(dq) = g.per_core.get(i) {
                    draw_graph(cr, w as f64, h as f64, dq, 100.0);
                }
            }
        });
        grid.attach(&area, (i % cols) as i32, (i / cols) as i32, 1, 1);
        ui_core.push(area);
    }
    g_cores.add(&grid);
    page.add(&g_cores);

    let g_temp = adw::PreferencesGroup::builder().title("CPU Temperature (1 minute)").build();
    let temp_area = gtk::DrawingArea::new();
    temp_area.set_content_height(140);
    temp_area.set_hexpand(true);
    temp_area.add_css_class("cpu-graph-frame");
    temp_area.set_margin_top(6);
    temp_area.set_margin_bottom(6);
    {
        let sh = shared.clone();
        temp_area.set_draw_func(move |_a, cr, w, h| {
            if let Ok(g) = sh.lock() {
                draw_graph(cr, w as f64, h as f64, &g.temp_hist, 100.0);
            }
        });
    }
    g_temp.add(&temp_area);
    page.add(&g_temp);

    let g_info = adw::PreferencesGroup::builder().title("Detailed Information").build();
    let mut l = std::collections::HashMap::new();
    l.insert("speed", info_row("Current Speed", &g_info));
    l.insert("base", info_row("Base Speed", &g_info));
    l.insert("logical", info_row("Logical Processors", &g_info));
    l.insert("sockets", info_row("Socket", &g_info));
    l.insert("virt", info_row("Virtualisasi", &g_info));
    l.insert("vm", info_row("Virtual Machine", &g_info));
    l.insert("l1", info_row("Cache L1 (data / instruksi)", &g_info));
    l.insert("l2", info_row("Cache L2", &g_info));
    l.insert("l3", info_row("Cache L3", &g_info));
    l.insert("driver", info_row("Cpufreq Driver", &g_info));
    l.insert("gov", info_row("Cpufreq Governor", &g_info));
    l.insert("pth", info_row("Processes / Threads / Handles", &g_info));
    l.insert("uptime", info_row("System Uptime", &g_info));
    l.insert("temp", info_row("CPU Temperature", &g_info));
    page.add(&g_info);

    (page, row_model, util_bar, temp_area, l)
}

// ───────────────────────── RGB tab ─────────────────────────
struct RgbState {
    mode: String,
    r: u8,
    g: u8,
    b: u8,
    speed: String,
    brightness: u8,
}

fn read_rgb_conf() -> RgbState {
    let c = "/etc/asus-power-manager/rgb.conf";
    let gv = |k: &str, d: &str| read_kv(c, k).unwrap_or_else(|| d.to_string());
    RgbState {
        mode: gv("MODE", "0"),
        r: gv("RED", "0").parse().unwrap_or(0),
        g: gv("GREEN", "200").parse().unwrap_or(200),
        b: gv("BLUE", "255").parse().unwrap_or(255),
        speed: gv("SPEED", "1"),
        brightness: gv("BRIGHTNESS", "3").parse().unwrap_or(3),
    }
}
fn rgb_mode_name(m: &str) -> &'static str {
    match m {
        "1" => "Breathing",
        "2" => "Color Cycle",
        "3" => "Strobing",
        "10" => "Pulse",
        _ => "Static",
    }
}
fn rounded_path(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, rad: f64) {
    use std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - rad, y + rad, rad, -PI / 2.0, 0.0);
    cr.arc(x + w - rad, y + h - rad, rad, 0.0, PI / 2.0);
    cr.arc(x + rad, y + h - rad, rad, PI / 2.0, PI);
    cr.arc(x + rad, y + rad, rad, PI, 3.0 * PI / 2.0);
    cr.close_path();
}
fn draw_swatch(cr: &gtk::cairo::Context, w: f64, h: f64, r: u8, g: u8, b: u8) {
    rounded_path(cr, 2.0, 2.0, w - 4.0, h - 4.0, 6.0);
    cr.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.6);
    cr.set_line_width(2.0);
    let _ = cr.stroke();
}
fn draw_circle(cr: &gtk::cairo::Context, w: f64, h: f64, r: u8, g: u8, b: u8) {
    let radius = (w.min(h)) / 2.0 - 2.0;
    cr.arc(w / 2.0, h / 2.0, radius, 0.0, 2.0 * std::f64::consts::PI);
    cr.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.45);
    cr.set_line_width(2.0);
    let _ = cr.stroke();
}

fn parse_hex_color(s: &str) -> (u8, u8, u8) {
    let clean = s.trim_start_matches("0x").trim_start_matches('#');
    if clean.len() >= 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&clean[2..4], 16).unwrap_or(200);
        let b = u8::from_str_radix(&clean[4..6], 16).unwrap_or(255);
        (r, g, b)
    } else {
        (0, 200, 255)
    }
}

fn mouse_led_mode_name(id: u32) -> &'static str {
    match id {
        0 => "Disabled (Off)",
        1 => "Static Color",
        3 => "Color Cycle",
        10 => "Breathing",
        _ => "Static Color",
    }
}

#[derive(Clone, Debug)]
struct MouseLedState {
    mode: u32,
    r: u8,
    g: u8,
    b: u8,
    period_ms: u32,
    intensity: u32,
}

fn read_mouse_led_conf() -> MouseLedState {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let p = format!("{home}/.config/asus-power-manager/logitech.conf");
    let mode: u32 = read_kv(&p, "LED_MODE").and_then(|v| v.parse().ok()).unwrap_or(1);
    let color_str = read_kv(&p, "LED_COLOR").unwrap_or_else(|| "0x00c8ff".into());
    let (r, g, b) = parse_hex_color(&color_str);
    let period_ms: u32 = read_kv(&p, "LED_PERIOD").and_then(|v| v.parse().ok()).unwrap_or(3000);
    let intensity: u32 = read_kv(&p, "LED_INTENSITY").and_then(|v| v.parse().ok()).unwrap_or(100);
    MouseLedState { mode, r, g, b, period_ms, intensity }
}

fn build_rgb_page() -> adw::PreferencesPage {
    let st = Rc::new(RefCell::new(read_rgb_conf()));
    let guard = Rc::new(Cell::new(false));
    let debounce: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    let page = adw::PreferencesPage::new();

    // Preview
    let g_prev = adw::PreferencesGroup::builder().title("Active Color &amp; Effect Status").build();
    let row_prev = adw::ActionRow::builder().title("Current Keyboard Color").build();
    let preview = gtk::DrawingArea::new();
    preview.set_content_width(52);
    preview.set_content_height(28);
    preview.set_valign(gtk::Align::Center);
    {
        let st = st.clone();
        preview.set_draw_func(move |_a, cr, w, h| {
            let s = st.borrow();
            draw_swatch(cr, w as f64, h as f64, s.r, s.g, s.b);
        });
    }
    row_prev.add_suffix(&preview);
    g_prev.add(&row_prev);
    page.add(&g_prev);

    // apply + refresh closures
    let apply: Rc<dyn Fn()> = {
        let st = st.clone();
        Rc::new(move || {
            let s = st.borrow();
            run_priv(vec![
                script_path("battery-set-rgb.sh"),
                s.mode.clone(),
                s.r.to_string(),
                s.g.to_string(),
                s.b.to_string(),
                s.speed.clone(),
                s.brightness.to_string(),
            ]);
        })
    };
    let refresh_prev: Rc<dyn Fn()> = {
        let st = st.clone();
        let row = row_prev.clone();
        let pv = preview.clone();
        Rc::new(move || {
            let s = st.borrow();
            row.set_subtitle(&format!(
                "RGB: ({}, {}, {}) | Hex: #{:02X}{:02X}{:02X} | Mode: {}",
                s.r, s.g, s.b, s.r, s.g, s.b, rgb_mode_name(&s.mode)
            ));
            pv.queue_draw();
        })
    };
    let schedule: Rc<dyn Fn()> = {
        let apply = apply.clone();
        let deb = debounce.clone();
        Rc::new(move || {
            if let Some(id) = deb.take() {
                id.remove();
            }
            let apply2 = apply.clone();
            let deb2 = deb.clone();
            let id = glib::timeout_add_local(Duration::from_millis(80), move || {
                apply2();
                deb2.set(None);
                glib::ControlFlow::Break
            });
            deb.set(Some(id));
        })
    };

    // Color picker
    let g_pick = adw::PreferencesGroup::builder().title("Custom Color (Color Wheel)").build();
    let row_pick = adw::ActionRow::builder().title("Color Spectrum Dialog").subtitle("Open GNOME color picker").build();
    let dialog = gtk::ColorDialog::new();
    dialog.set_with_alpha(false);
    let color_btn = gtk::ColorDialogButton::new(Some(dialog));
    color_btn.set_valign(gtk::Align::Center);
    {
        let s = st.borrow();
        color_btn.set_rgba(&gtk::gdk::RGBA::new(s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0, 1.0));
    }
    row_pick.add_suffix(&color_btn);
    g_pick.add(&row_pick);

    // Sliders
    let g_sl = adw::PreferencesGroup::builder().title("Manual Adjustment (RGB Sliders)").build();
    let mk_scale = || {
        let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 255.0, 1.0);
        s.set_size_request(180, -1);
        s.set_valign(gtk::Align::Center);
        s
    };
    let (scale_r, scale_g, scale_b) = (mk_scale(), mk_scale(), mk_scale());
    {
        let s = st.borrow();
        scale_r.set_value(s.r as f64);
        scale_g.set_value(s.g as f64);
        scale_b.set_value(s.b as f64);
    }
    for (title, css, scale) in [
        ("Merah (Red)", "red-slider", &scale_r),
        ("Hijau (Green)", "green-slider", &scale_g),
        ("Biru (Blue)", "blue-slider", &scale_b),
    ] {
        scale.add_css_class(css);
        let row = adw::ActionRow::builder().title(title).build();
        row.add_suffix(scale);
        g_sl.add(&row);
    }

    // channel change handler factory
    let attach_channel = |scale: &gtk::Scale, ch: char| {
        let st = st.clone();
        let guard = guard.clone();
        let sched = schedule.clone();
        let rp = refresh_prev.clone();
        let cbtn = color_btn.clone();
        let g2 = guard.clone();
        scale.connect_value_changed(move |s| {
            if guard.get() {
                return;
            }
            let v = s.value() as u8;
            {
                let mut stt = st.borrow_mut();
                match ch {
                    'r' => stt.r = v,
                    'g' => stt.g = v,
                    _ => stt.b = v,
                }
            }
            // sync color button (guarded)
            let s2 = st.borrow();
            g2.set(true);
            cbtn.set_rgba(&gtk::gdk::RGBA::new(s2.r as f32 / 255.0, s2.g as f32 / 255.0, s2.b as f32 / 255.0, 1.0));
            g2.set(false);
            drop(s2);
            rp();
            sched();
        });
    };
    attach_channel(&scale_r, 'r');
    attach_channel(&scale_g, 'g');
    attach_channel(&scale_b, 'b');

    // color button handler
    {
        let st = st.clone();
        let guard = guard.clone();
        let sched = schedule.clone();
        let rp = refresh_prev.clone();
        let (sr, sg, sb) = (scale_r.clone(), scale_g.clone(), scale_b.clone());
        color_btn.connect_rgba_notify(move |b| {
            if guard.get() {
                return;
            }
            let c = b.rgba();
            let (r, g, bl) = ((c.red() * 255.0) as u8, (c.green() * 255.0) as u8, (c.blue() * 255.0) as u8);
            {
                let mut s = st.borrow_mut();
                s.r = r;
                s.g = g;
                s.b = bl;
            }
            guard.set(true);
            sr.set_value(r as f64);
            sg.set_value(g as f64);
            sb.set_value(bl as f64);
            guard.set(false);
            rp();
            sched();
        });
    }

    // Palette presets
    let g_pal = adw::PreferencesGroup::builder()
        .title("Quick Color Palette (Preset)")
        .description("Click a color to apply it instantly")
        .build();
    let pal_row = adw::PreferencesRow::builder().activatable(false).build();
    let pal_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    pal_box.set_halign(gtk::Align::Center);
    pal_box.set_margin_top(10);
    pal_box.set_margin_bottom(10);
    let palette: [(u8, u8, u8); 9] = [
        (0, 200, 255),
        (0, 100, 255),
        (160, 0, 255),
        (255, 0, 127),
        (255, 0, 0),
        (255, 120, 0),
        (255, 216, 0),
        (0, 230, 118),
        (255, 255, 255),
    ];
    for (r, g, b) in palette {
        let btn = gtk::Button::new();
        btn.add_css_class("flat");
        let area = gtk::DrawingArea::new();
        area.set_content_width(34);
        area.set_content_height(34);
        area.set_draw_func(move |_a, cr, w, h| draw_circle(cr, w as f64, h as f64, r, g, b));
        btn.set_child(Some(&area));
        let st = st.clone();
        let guard = guard.clone();
        let apply = apply.clone();
        let rp = refresh_prev.clone();
        let (sr, sg, sb) = (scale_r.clone(), scale_g.clone(), scale_b.clone());
        let cbtn = color_btn.clone();
        btn.connect_clicked(move |_| {
            {
                let mut s = st.borrow_mut();
                s.r = r;
                s.g = g;
                s.b = b;
            }
            guard.set(true);
            sr.set_value(r as f64);
            sg.set_value(g as f64);
            sb.set_value(b as f64);
            cbtn.set_rgba(&gtk::gdk::RGBA::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0));
            guard.set(false);
            rp();
            apply();
        });
        pal_box.append(&btn);
    }
    pal_row.set_child(Some(&pal_box));
    g_pal.add(&pal_row);

    page.add(&g_pal);
    page.add(&g_pick);
    page.add(&g_sl);

    // Effects
    let g_eff = adw::PreferencesGroup::builder().title("Animation Effects (Aura Lighting)").build();
    let effects = [
        ("0", "Static (Fixed Color)"),
        ("1", "Breathing"),
        ("10", "Pulse"),
        ("2", "Color Cycle (Rainbow)"),
        ("3", "Strobing"),
    ];
    let eff_btns: Rc<Vec<(String, gtk::Button)>> = Rc::new(
        effects.iter().map(|(id, _)| (id.to_string(), gtk::Button::with_label("Select"))).collect(),
    );
    let highlight_eff: Rc<dyn Fn(&str)> = {
        let eb = eff_btns.clone();
        Rc::new(move |active: &str| {
            for (id, b) in eb.iter() {
                if id == active {
                    b.add_css_class("suggested-action");
                    b.set_label("✓ Active");
                } else {
                    b.remove_css_class("suggested-action");
                    b.set_label("Select");
                }
            }
        })
    };
    for (idx, (id, title)) in effects.iter().enumerate() {
        let row = adw::ActionRow::builder().title(*title).build();
        let btn = eff_btns[idx].1.clone();
        btn.set_valign(gtk::Align::Center);
        let id_s = id.to_string();
        let st = st.clone();
        let apply = apply.clone();
        let rp = refresh_prev.clone();
        let hl = highlight_eff.clone();
        btn.connect_clicked(move |_| {
            st.borrow_mut().mode = id_s.clone();
            hl(&id_s);
            rp();
            apply();
        });
        row.add_suffix(&btn);
        g_eff.add(&row);
    }
    page.add(&g_eff);

    // Brightness + Speed
    let g_bs = adw::PreferencesGroup::builder().title("Brightness &amp; Effect Speed").build();
    // brightness
    let row_b = adw::ActionRow::builder().title("Keyboard Backlight Brightness").build();
    let box_b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    box_b.set_valign(gtk::Align::Center);
    let bri_btns: Rc<Vec<(u8, gtk::Button)>> = Rc::new(
        [(0u8, "Off"), (1, "Dim"), (2, "Medium"), (3, "Bright")]
            .iter()
            .map(|(v, l)| (*v, gtk::Button::with_label(l)))
            .collect(),
    );
    let hl_bri: Rc<dyn Fn(u8)> = {
        let bb = bri_btns.clone();
        Rc::new(move |active: u8| {
            for (v, b) in bb.iter() {
                if *v == active {
                    b.add_css_class("suggested-action");
                } else {
                    b.remove_css_class("suggested-action");
                }
            }
        })
    };
    for (v, btn) in bri_btns.iter() {
        let v = *v;
        let btn = btn.clone();
        let st = st.clone();
        let apply = apply.clone();
        let hl = hl_bri.clone();
        btn.connect_clicked(move |_| {
            st.borrow_mut().brightness = v;
            hl(v);
            apply();
        });
        box_b.append(&btn);
    }
    row_b.add_suffix(&box_b);
    g_bs.add(&row_b);
    // speed
    let row_s = adw::ActionRow::builder().title("Effect Animation Speed").build();
    let box_s = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    box_s.set_valign(gtk::Align::Center);
    let spd_btns: Rc<Vec<(String, gtk::Button)>> = Rc::new(
        [("0", "Slow"), ("1", "Medium"), ("2", "Fast")]
            .iter()
            .map(|(v, l)| (v.to_string(), gtk::Button::with_label(l)))
            .collect(),
    );
    let hl_spd: Rc<dyn Fn(&str)> = {
        let sb = spd_btns.clone();
        Rc::new(move |active: &str| {
            for (v, b) in sb.iter() {
                if v == active {
                    b.add_css_class("suggested-action");
                } else {
                    b.remove_css_class("suggested-action");
                }
            }
        })
    };
    for (v, btn) in spd_btns.iter() {
        let v = v.clone();
        let btn = btn.clone();
        let st = st.clone();
        let apply = apply.clone();
        let hl = hl_spd.clone();
        btn.connect_clicked(move |_| {
            st.borrow_mut().speed = v.clone();
            hl(&v);
            apply();
        });
        box_s.append(&btn);
    }
    row_s.add_suffix(&box_s);
    g_bs.add(&row_s);
    page.add(&g_bs);

    // initial highlight + preview
    {
        let s = st.borrow();
        highlight_eff(&s.mode);
        hl_bri(s.brightness);
        hl_spd(&s.speed);
    }
    refresh_prev();

    page
}

fn sysctl_capture(is_user: bool, verb: &str, unit: &str) -> String {
    let mut c = Command::new("systemctl");
    if is_user {
        c.arg("--user");
    }
    c.arg(verb).arg(unit);
    if verb == "status" {
        c.arg("--no-pager");
    }
    match c.output() {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).to_string();
            let e = String::from_utf8_lossy(&o.stderr);
            if !e.trim().is_empty() {
                s.push_str(&e);
            }
            let s = s.trim_end().to_string();
            if s.is_empty() {
                "(empty)".into()
            } else {
                s
            }
        }
        Err(e) => format!("Error: {e}"),
    }
}
fn journal_capture(is_user: bool, unit: &str) -> String {
    let mut c = Command::new("journalctl");
    if is_user {
        c.arg("--user");
    }
    c.arg("-u").arg(unit).arg("-n").arg("30").arg("--no-pager");
    match c.output() {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).trim_end().to_string();
            if s.is_empty() {
                "(no log entries yet)".into()
            } else {
                s
            }
        }
        Err(e) => format!("Error: {e}"),
    }
}

fn detail_textview(monospace: bool) -> gtk::TextView {
    let tv = gtk::TextView::new();
    tv.set_editable(false);
    tv.set_cursor_visible(false);
    tv.set_monospace(monospace);
    tv.set_wrap_mode(gtk::WrapMode::WordChar);
    tv.set_left_margin(8);
    tv.set_top_margin(6);
    tv
}
fn scrolled(child: &impl IsA<gtk::Widget>, min_h: i32) -> gtk::ScrolledWindow {
    let sw = gtk::ScrolledWindow::new();
    sw.set_min_content_height(min_h);
    sw.set_child(Some(child));
    sw.add_css_class("card");
    sw
}

fn open_service_detail(parent: &adw::ApplicationWindow, is_user: bool, unit: &str) {
    let unit = unit.to_string();
    let win = adw::Window::new();
    win.set_title(Some(&format!("Details — {unit}")));
    win.set_default_size(620, 740);
    win.set_modal(true);
    win.set_transient_for(Some(parent));

    let tv_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let refresh_btn = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_tooltip_text(Some("Refresh"));
    header.pack_end(&refresh_btn);
    tv_view.add_top_bar(&header);

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 10);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let mk_label = |t: &str| {
        let l = gtk::Label::new(Some(t));
        l.set_xalign(0.0);
        l.add_css_class("heading");
        l
    };

    vbox.append(&mk_label(&format!(
        "{} — {}",
        unit,
        if is_user { "User Service (--user)" } else { "System Service (root)" }
    )));
    vbox.append(&mk_label("Status Operasional"));
    let tv_status = detail_textview(true);
    vbox.append(&scrolled(&tv_status, 130));
    vbox.append(&mk_label("File Konfigurasi Unit (systemctl cat)"));
    let tv_cat = detail_textview(true);
    vbox.append(&scrolled(&tv_cat, 220));
    vbox.append(&mk_label("Log Aktivitas Terbaru (journald)"));
    let tv_logs = detail_textview(true);
    vbox.append(&scrolled(&tv_logs, 180));

    // action buttons
    let hb = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    hb.set_halign(gtk::Align::End);
    hb.set_margin_top(4);
    let b_start = gtk::Button::with_label("Start");
    b_start.add_css_class("suggested-action");
    let b_stop = gtk::Button::with_label("Stop");
    b_stop.add_css_class("destructive-action");
    let b_restart = gtk::Button::with_label("Restart");
    let run_action = |is_user: bool, action: &'static str, unit: String| {
        if is_user {
            run_user(vec!["systemctl".into(), "--user".into(), action.into(), unit]);
        } else {
            run_priv(vec!["systemctl".into(), action.into(), unit]);
        }
    };
    {
        let u = unit.clone();
        b_start.connect_clicked(move |_| run_action(is_user, "start", u.clone()));
    }
    {
        let u = unit.clone();
        b_stop.connect_clicked(move |_| run_action(is_user, "stop", u.clone()));
    }
    {
        let u = unit.clone();
        b_restart.connect_clicked(move |_| run_action(is_user, "restart", u.clone()));
    }
    hb.append(&b_start);
    hb.append(&b_stop);
    hb.append(&b_restart);
    vbox.append(&hb);

    tv_view.set_content(Some(&vbox));
    win.set_content(Some(&tv_view));

    let refresh: Rc<dyn Fn()> = {
        let unit = unit.clone();
        let s = tv_status.clone();
        let c = tv_cat.clone();
        let l = tv_logs.clone();
        Rc::new(move || {
            s.buffer().set_text(&sysctl_capture(is_user, "status", &unit));
            c.buffer().set_text(&sysctl_capture(is_user, "cat", &unit));
            l.buffer().set_text(&journal_capture(is_user, &unit));
        })
    };
    refresh();
    {
        let r = refresh.clone();
        refresh_btn.connect_clicked(move |_| r());
    }

    win.present();
}

fn build_svc_group(
    shared: &Arc<Mutex<Shared>>,
    window: &adw::ApplicationWindow,
    defs: &[(&str, &str, &str)],
    is_user: bool,
    group_title: &str,
    out: &mut Vec<SvcW>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(group_title).build();
    for (unit, title, sub) in defs.iter() {
        let row = adw::ActionRow::builder()
            .title(*title)
            .subtitle(&format!("Unit: {} • {}", unit, sub).replace('&', "&amp;"))
            .build();
        // Leading status dot (green = active, red = failed) like `systemctl status`.
        let dot = gtk::Label::new(Some("●"));
        dot.add_css_class("svc-dot");
        dot.add_css_class("dot-stop");
        dot.set_valign(gtk::Align::Center);
        row.add_prefix(&dot);
        let bx = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bx.set_valign(gtk::Align::Center);
        let badge = gtk::Label::new(Some("Loading"));
        badge.add_css_class("badge-stop");
        let toggle = seg_button("Start");
        let restart = seg_button("Restart");
        let key = format!("{}:{}", if is_user { "user" } else { "sys" }, unit);
        {
            let sh = shared.clone();
            let unit = unit.to_string();
            let key = key.clone();
            toggle.connect_clicked(move |_| {
                let active = sh
                    .lock()
                    .ok()
                    .and_then(|g| g.services.get(&key).cloned())
                    .map(|s| s == "active")
                    .unwrap_or(false);
                let action = if active { "stop" } else { "start" };
                if is_user {
                    run_user(vec!["systemctl".into(), "--user".into(), action.into(), unit.clone()]);
                } else {
                    run_priv(vec!["systemctl".into(), action.into(), unit.clone()]);
                }
            });
        }
        {
            let unit = unit.to_string();
            restart.connect_clicked(move |_| {
                if is_user {
                    run_user(vec!["systemctl".into(), "--user".into(), "restart".into(), unit.clone()]);
                } else {
                    run_priv(vec!["systemctl".into(), "restart".into(), unit.clone()]);
                }
            });
        }
        bx.append(&badge);
        bx.append(&toggle);
        bx.append(&restart);
        let detail = seg_button("Details");
        {
            let win = window.clone();
            let unit = unit.to_string();
            detail.connect_clicked(move |_| open_service_detail(&win, is_user, &unit));
        }
        bx.append(&detail);
        row.add_suffix(&bx);
        group.add(&row);
        out.push(SvcW {
            key,
            is_user,
            unit: unit.to_string(),
            badge,
            dot,
            toggle,
        });
    }
    group
}

fn fmt_gib(kb: f64) -> String {
    let gib = kb / 1048576.0;
    if gib < 0.1 && kb > 0.0 {
        format!("{:.0} MiB", kb / 1024.0)
    } else if kb == 0.0 {
        "0".to_string()
    } else {
        format!("{:.1} GiB", gib)
    }
}

fn fmt_bytes(b: u64) -> String {
    let x = b as f64;
    let (t, g, m, k) = (
        1024.0_f64.powi(4),
        1024.0_f64.powi(3),
        1024.0_f64.powi(2),
        1024.0_f64,
    );
    if x >= t {
        format!("{:.1} TiB", x / t)
    } else if x >= g {
        format!("{:.1} GiB", x / g)
    } else if x >= m {
        format!("{:.1} MiB", x / m)
    } else if x >= k {
        format!("{:.0} KiB", x / k)
    } else {
        format!("{} B", b)
    }
}

fn fmt_rate(bps: f64) -> String {
    let b = bps.max(0.0);
    let (g, m) = (1024.0_f64.powi(3), 1024.0_f64.powi(2));
    if b >= g {
        format!("{:.1} GiB/s", b / g)
    } else if b >= m {
        format!("{:.1} MiB/s", b / m)
    } else {
        format!("{:.0} KiB/s", b / 1024.0)
    }
}

fn lucide(name: &str, px: i32) -> gtk::Image {
    let home = std::env::var("HOME").unwrap_or_default();
    let cands = [
        format!("{home}/Development/asus-power-manager/rust-gui/icons/{name}.svg"),
        format!("/usr/share/asus-tuf-cpu/icons/{name}.svg"),
        format!("icons/{name}.svg"),
    ];
    for c in &cands {
        if std::path::Path::new(c).exists() {
            let im = gtk::Image::from_file(c);
            im.set_pixel_size(px);
            return im;
        }
    }
    let im = gtk::Image::from_icon_name("image-missing");
    im.set_pixel_size(px);
    im
}

type MemPage = (
    adw::PreferencesPage,
    adw::ActionRow,
    gtk::LevelBar,
    gtk::DrawingArea,
    gtk::DrawingArea,
    std::collections::HashMap<&'static str, gtk::Label>,
);
fn build_memory_page(shared: &Arc<Mutex<Shared>>) -> MemPage {
    let page = adw::PreferencesPage::new();

    let g_head = adw::PreferencesGroup::builder().title("System Memory").build();
    let row_mem = adw::ActionRow::builder().title("Memory").subtitle("Loading...").build();
    let mem_bar = gtk::LevelBar::builder().min_value(0.0).max_value(100.0).valign(gtk::Align::Center).build();
    mem_bar.set_size_request(110, 16);
    row_mem.add_suffix(&mem_bar);
    g_head.add(&row_mem);
    page.add(&g_head);

    let g_mem = adw::PreferencesGroup::builder().title("Memory Usage (1 minute)").build();
    let mem_area = gtk::DrawingArea::new();
    mem_area.set_content_height(150);
    mem_area.set_hexpand(true);
    mem_area.add_css_class("cpu-graph-frame");
    mem_area.set_margin_top(6);
    mem_area.set_margin_bottom(6);
    {
        let sh = shared.clone();
        mem_area.set_draw_func(move |_a, cr, w, h| {
            if let Ok(g) = sh.lock() {
                draw_graph(cr, w as f64, h as f64, &g.mem_hist, 100.0);
            }
        });
    }
    g_mem.add(&mem_area);
    page.add(&g_mem);

    let g_swap = adw::PreferencesGroup::builder().title("Swap (1 minute)").build();
    let swap_area = gtk::DrawingArea::new();
    swap_area.set_content_height(110);
    swap_area.set_hexpand(true);
    swap_area.add_css_class("cpu-graph-frame");
    swap_area.set_margin_top(6);
    swap_area.set_margin_bottom(6);
    {
        let sh = shared.clone();
        swap_area.set_draw_func(move |_a, cr, w, h| {
            if let Ok(g) = sh.lock() {
                draw_graph(cr, w as f64, h as f64, &g.swap_hist, 100.0);
            }
        });
    }
    g_swap.add(&swap_area);
    page.add(&g_swap);

    let g_det = adw::PreferencesGroup::builder().title("Details").build();
    let mut mem_lbl = std::collections::HashMap::new();
    mem_lbl.insert("used", info_row("In Use", &g_det));
    mem_lbl.insert("avail", info_row("Available", &g_det));
    mem_lbl.insert("committed", info_row("Committed", &g_det));
    mem_lbl.insert("cached", info_row("Cached", &g_det));
    mem_lbl.insert("swapused", info_row("Swap Used", &g_det));
    mem_lbl.insert("swapavail", info_row("Swap Available", &g_det));
    page.add(&g_det);

    let g_hw = adw::PreferencesGroup::builder()
        .title("Hardware (DIMM)")
        .build();
    mem_lbl.insert("dtype", info_row("Type", &g_hw));
    mem_lbl.insert("dform", info_row("Form Factor", &g_hw));
    mem_lbl.insert("dspeed", info_row("Speed", &g_hw));
    mem_lbl.insert("dslots", info_row("Slots Used", &g_hw));
    page.add(&g_hw);

    (page, row_mem, mem_bar, mem_area, swap_area, mem_lbl)
}

struct DriveUi {
    idx: usize,
    dev: String,
    active_area: gtk::DrawingArea,
    thru_area: gtk::DrawingArea,
    thru_scale: gtk::Label,
    lbl: std::collections::HashMap<&'static str, gtk::Label>,
    parts: Vec<(gtk::LevelBar, gtk::Label)>,
    smart_lbl: std::collections::HashMap<&'static str, gtk::Label>,
    health_lbl: gtk::Label,
}

// Build one drive detail page (mirrors Mission Center's Disk view layout using
// this app's card style: Active-time + Throughput graphs, stats, S.M.A.R.T. health,
// details, partitions with usage bars).
fn build_drive_page(shared: &Arc<Mutex<Shared>>, idx: usize, info: &DriveInfo) -> (adw::PreferencesPage, DriveUi) {
    let page = adw::PreferencesPage::new();

    let g_head = adw::PreferencesGroup::builder()
        .title(format!("Drive {} ({})", idx, info.dev))
        .build();
    let row = adw::ActionRow::builder()
        .title(if info.model.is_empty() { info.dev.clone() } else { info.model.clone() })
        .subtitle(format!("{} • {}", info.kind, fmt_bytes(info.capacity)))
        .build();
    g_head.add(&row);
    page.add(&g_head);

    let g_active = adw::PreferencesGroup::builder().title("Active Time (1 minute)").build();
    let active_area = gtk::DrawingArea::new();
    active_area.set_content_height(130);
    active_area.set_hexpand(true);
    active_area.add_css_class("cpu-graph-frame");
    active_area.set_margin_top(6);
    active_area.set_margin_bottom(6);
    {
        let sh = shared.clone();
        active_area.set_draw_func(move |_a, cr, w, h| {
            if let Ok(g) = sh.lock() {
                if let Some(d) = g.drives.get(idx) {
                    draw_graph(cr, w as f64, h as f64, &d.active_hist, 100.0);
                }
            }
        });
    }
    g_active.add(&active_area);
    page.add(&g_active);

    let g_thru = adw::PreferencesGroup::builder().title("Throughput (1 minute)").build();
    let thru_scale = gtk::Label::new(Some("0 KiB/s"));
    thru_scale.add_css_class("dim-label");
    g_thru.set_header_suffix(Some(&thru_scale));
    let thru_area = gtk::DrawingArea::new();
    thru_area.set_content_height(130);
    thru_area.set_hexpand(true);
    thru_area.add_css_class("cpu-graph-frame");
    thru_area.set_margin_top(6);
    thru_area.set_margin_bottom(6);
    {
        let sh = shared.clone();
        thru_area.set_draw_func(move |_a, cr, w, h| {
            if let Ok(g) = sh.lock() {
                if let Some(d) = g.drives.get(idx) {
                    let mx = d.thru_hist.iter().cloned().fold(1.0_f64, f64::max) * 1.15;
                    draw_graph(cr, w as f64, h as f64, &d.thru_hist, mx);
                }
            }
        });
    }
    g_thru.add(&thru_area);
    page.add(&g_thru);

    let mut lbl = std::collections::HashMap::new();
    let g_stat = adw::PreferencesGroup::builder().title("Statistics").build();
    lbl.insert("rspeed", info_row("Read Speed", &g_stat));
    lbl.insert("wspeed", info_row("Write Speed", &g_stat));
    lbl.insert("tread", info_row("Total Read", &g_stat));
    lbl.insert("twrite", info_row("Total Written", &g_stat));
    lbl.insert("active", info_row("Active Time", &g_stat));
    lbl.insert("resp", info_row("Avg Response", &g_stat));
    page.add(&g_stat);

    // ── S.M.A.R.T. Health Card inside Drive tab ──
    let mut smart_lbl = std::collections::HashMap::new();
    let g_smart = adw::PreferencesGroup::builder().title("S.M.A.R.T. Health Status").build();
    let row_h = adw::ActionRow::builder().title("Health Status").build();
    let health_lbl = gtk::Label::new(Some("--"));
    health_lbl.add_css_class("dim-label");
    health_lbl.set_valign(gtk::Align::Center);
    row_h.add_suffix(&health_lbl);
    g_smart.add(&row_h);

    smart_lbl.insert("temp", info_row("Temperature", &g_smart));
    smart_lbl.insert("hours", info_row("Power-On Hours", &g_smart));
    smart_lbl.insert("cycles", info_row("Power Cycles", &g_smart));
    smart_lbl.insert("realloc", info_row("Reallocated Sectors", &g_smart));
    smart_lbl.insert("pct", info_row("Percentage Used (SSD)", &g_smart));
    smart_lbl.insert("written", info_row("Total Data Written", &g_smart));
    page.add(&g_smart);

    let g_det = adw::PreferencesGroup::builder().title("Details").build();
    let det = |t: &str, v: &str, gr: &adw::PreferencesGroup| {
        let l = info_row(t, gr);
        // Long IDs (e.g. WWN) must not steal the row width and wrap the title.
        // Middle-ellipsize like Mission Center; selectable keeps the full value.
        l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        l.set_max_width_chars(28);
        l.set_selectable(true);
        l.set_text(v);
    };
    det("Capacity", &fmt_bytes(info.capacity), &g_det);
    det("Formatted", &fmt_bytes(info.formatted), &g_det);
    det("System Disk", if info.is_system { "Yes" } else { "No" }, &g_det);
    det("Type", &info.kind, &g_det);
    det("WWN", if info.wwn.is_empty() { "—" } else { &info.wwn }, &g_det);
    det("Serial", if info.serial.is_empty() { "—" } else { &info.serial }, &g_det);
    if info.rotational {
        det("Rotation", "HDD (spinning)", &g_det);
    }
    page.add(&g_det);

    let mut parts = Vec::new();
    if !info.partitions.is_empty() {
        let g_part = adw::PreferencesGroup::builder().title("Partitions").build();
        for p in &info.partitions {
            let sub = if p.mount.is_empty() {
                if p.fstype.is_empty() { "not mounted".to_string() } else { p.fstype.clone() }
            } else {
                format!("{} • {}", if p.fstype.is_empty() { "?" } else { &p.fstype }, p.mount)
            };
            let r = adw::ActionRow::builder()
                .title(format!("/dev/{}", p.name))
                .subtitle(sub)
                .build();
            let bx = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let bar = gtk::LevelBar::builder()
                .min_value(0.0)
                .max_value(100.0)
                .valign(gtk::Align::Center)
                .build();
            bar.set_size_request(90, 12);
            let sz = gtk::Label::new(Some(&fmt_bytes(p.size)));
            sz.add_css_class("dim-label");
            bx.append(&bar);
            bx.append(&sz);
            r.add_suffix(&bx);
            g_part.add(&r);
            parts.push((bar, sz));
        }
        page.add(&g_part);
    }

    let du = DriveUi {
        idx,
        dev: info.dev.clone(),
        active_area,
        thru_area,
        thru_scale,
        lbl,
        parts,
        smart_lbl,
        health_lbl,
    };
    (page, du)
}

// Classify a sidebar row into a section, for the list header func.
fn sidebar_section(name: &str) -> &'static str {
    match name {
        "cpu" | "memory" | "gpu" | "fan" | "net" | "drive" | "bat" | "speedtest" => "Monitoring",
        _ => "Control & System",
    }
}

// Build a single sidebar navigation row (Lucide icon + label).
fn sidebar_row(name: &str, label: &str, icon: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_widget_name(name);
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bx.set_margin_top(10);
    bx.set_margin_bottom(10);
    bx.set_margin_start(10);
    bx.set_margin_end(10);
    let img = lucide(icon, 22);
    let lbl = gtk::Label::new(Some(label));
    lbl.set_xalign(0.0);
    bx.append(&img);
    bx.append(&lbl);
    row.set_child(Some(&bx));
    row
}

// Wrap an inner ViewStack in a vertical box. When there is more than one page,
// add an in-page segmented tab-bar of toggle buttons, each showing the parent
// family's lucide icon plus the page title (icon loaded from file, same as the
// sidebar, so it is never a greyed-out themed placeholder).
fn group_container(inner: &adw::ViewStack, tabs: &[(String, String)], icon: &str) -> gtk::Box {
    let c = gtk::Box::new(gtk::Orientation::Vertical, 0);
    if tabs.len() > 1 {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar.set_halign(gtk::Align::Center);
        bar.set_margin_top(8);
        bar.set_margin_bottom(4);
        bar.add_css_class("linked");
        let mut first: Option<gtk::ToggleButton> = None;
        for (name, title) in tabs {
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content.set_margin_start(4);
            content.set_margin_end(4);
            content.append(&lucide(icon, 16));
            content.append(&gtk::Label::new(Some(title)));
            let btn = gtk::ToggleButton::new();
            btn.set_child(Some(&content));
            match &first {
                Some(f) => btn.set_group(Some(f)),
                None => {
                    btn.set_active(true);
                    first = Some(btn.clone());
                }
            }
            let inner2 = inner.clone();
            let nm = name.clone();
            btn.connect_toggled(move |b| {
                if b.is_active() {
                    inner2.set_visible_child_name(&nm);
                }
            });
            bar.append(&btn);
        }
        c.append(&bar);
    }
    c.append(inner);
    c
}

fn fmt_clock(mhz: f64) -> String {
    if mhz <= 0.0 {
        "—".into()
    } else if mhz >= 1000.0 {
        format!("{:.2} GHz", mhz / 1000.0)
    } else {
        format!("{:.0} MHz", mhz)
    }
}

struct GpuUi {
    idx: usize,
    util_area: gtk::DrawingArea,
    mem_area: gtk::DrawingArea,
    lbl: std::collections::HashMap<&'static str, gtk::Label>,
}

// Build one GPU detail page (mirrors Mission Center's GPU view in this app's
// card style): Utilization + Memory graphs, stats, and details.
fn build_gpu_page(shared: &Arc<Mutex<Shared>>, idx: usize, info: &GpuInfo) -> (adw::PreferencesPage, GpuUi) {
    let page = adw::PreferencesPage::new();

    let g_head = adw::PreferencesGroup::builder().title(format!("GPU {idx}")).build();
    let row = adw::ActionRow::builder()
        .title(if info.name.is_empty() { info.kind.clone() } else { info.name.clone() })
        .subtitle(info.kind.clone())
        .build();
    g_head.add(&row);
    page.add(&g_head);

    let g_util = adw::PreferencesGroup::builder().title("Utilization (1 minute)").build();
    let util_area = gtk::DrawingArea::new();
    util_area.set_content_height(130);
    util_area.set_hexpand(true);
    util_area.add_css_class("cpu-graph-frame");
    util_area.set_margin_top(6);
    util_area.set_margin_bottom(6);
    {
        let sh = shared.clone();
        util_area.set_draw_func(move |_a, cr, w, h| {
            if let Ok(g) = sh.lock() {
                if let Some(d) = g.gpus.get(idx) {
                    draw_graph(cr, w as f64, h as f64, &d.util_hist, 100.0);
                }
            }
        });
    }
    g_util.add(&util_area);
    page.add(&g_util);

    let g_mem = adw::PreferencesGroup::builder().title("Memory Usage (1 minute)").build();
    let mem_area = gtk::DrawingArea::new();
    mem_area.set_content_height(130);
    mem_area.set_hexpand(true);
    mem_area.add_css_class("cpu-graph-frame");
    mem_area.set_margin_top(6);
    mem_area.set_margin_bottom(6);
    {
        let sh = shared.clone();
        mem_area.set_draw_func(move |_a, cr, w, h| {
            if let Ok(g) = sh.lock() {
                if let Some(d) = g.gpus.get(idx) {
                    draw_graph(cr, w as f64, h as f64, &d.mem_hist, 100.0);
                }
            }
        });
    }
    g_mem.add(&mem_area);
    page.add(&g_mem);

    let mut lbl = std::collections::HashMap::new();
    let g_stat = adw::PreferencesGroup::builder().title("Statistics").build();
    lbl.insert("util", info_row("Utilisasi", &g_stat));
    lbl.insert("clock", info_row("Clock", &g_stat));
    lbl.insert("power", info_row("Power", &g_stat));
    lbl.insert("mem", info_row("Memory", &g_stat));
    lbl.insert("memclk", info_row("Memory Speed", &g_stat));
    lbl.insert("encdec", info_row("Encode / Decode", &g_stat));
    lbl.insert("temp", info_row("Temperature", &g_stat));
    page.add(&g_stat);

    let g_det = adw::PreferencesGroup::builder().title("Details").build();
    let det = |t: &str, v: &str, gr: &adw::PreferencesGroup| {
        let l = info_row(t, gr);
        l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        l.set_max_width_chars(28);
        l.set_selectable(true);
        l.set_text(v);
    };
    det("Type", &info.kind, &g_det);
    det("Name", &info.name, &g_det);
    det("PCI Bus", if info.bus.is_empty() { "—" } else { &info.bus }, &g_det);
    let lbl_pcie = info_row("PCI Express", &g_det);
    lbl.insert("pcie", lbl_pcie);
    page.add(&g_det);

    let gu = GpuUi { idx, util_area, mem_area, lbl };
    (page, gu)
}

struct FanUi {
    idx: usize,
    area: gtk::DrawingArea,
    scale_lbl: gtk::Label,
    rpm_lbl: gtk::Label,
}

// Build one fan detail page (mirrors Mission Center's Fan view): a fan-speed
// graph (1 min) plus the current RPM.
fn build_fan_page(shared: &Arc<Mutex<Shared>>, idx: usize, info: &FanInfo) -> (adw::PreferencesPage, FanUi) {
    let page = adw::PreferencesPage::new();

    let g_head = adw::PreferencesGroup::builder().title(info.label.clone()).build();
    let row = adw::ActionRow::builder().title("Fan").subtitle(info.label.clone()).build();
    g_head.add(&row);
    page.add(&g_head);

    let g_graph = adw::PreferencesGroup::builder().title("Fan Speed (1 minute)").build();
    let scale_lbl = gtk::Label::new(Some("0 RPM"));
    scale_lbl.add_css_class("dim-label");
    g_graph.set_header_suffix(Some(&scale_lbl));
    let area = gtk::DrawingArea::new();
    area.set_content_height(150);
    area.set_hexpand(true);
    area.add_css_class("cpu-graph-frame");
    area.set_margin_top(6);
    area.set_margin_bottom(6);
    {
        let sh = shared.clone();
        area.set_draw_func(move |_a, cr, w, h| {
            if let Ok(g) = sh.lock() {
                if let Some(f) = g.fans.get(idx) {
                    let mx = f.hist.iter().cloned().fold(1.0_f64, f64::max) * 1.15;
                    draw_graph(cr, w as f64, h as f64, &f.hist, mx);
                }
            }
        });
    }
    g_graph.add(&area);
    page.add(&g_graph);

    let g_stat = adw::PreferencesGroup::builder().title("Statistics").build();
    let rpm_lbl = info_row("Fan Speed", &g_stat);
    page.add(&g_stat);

    let fu = FanUi { idx, area, scale_lbl, rpm_lbl };
    (page, fu)
}

fn fmt_bitrate(bytes_per_s: f64) -> String {
    let b = bytes_per_s.max(0.0) * 8.0;
    if b >= 1e9 {
        format!("{:.1} Gbps", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} Mbps", b / 1e6)
    } else {
        format!("{:.0} Kbps", b / 1e3)
    }
}

fn fmt_bits_total(bytes: u64) -> String {
    let b = bytes as f64 * 8.0;
    if b >= 1e9 {
        format!("{:.1} Gb", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} Mb", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.1} Kb", b / 1e3)
    } else {
        format!("{b:.0} b")
    }
}

struct NetUi {
    idx: usize,
    area: gtk::DrawingArea,
    scale_lbl: gtk::Label,
    lbl: std::collections::HashMap<&'static str, gtk::Label>,
    ts_group: Option<adw::PreferencesGroup>,
    ts_connect: Option<gtk::Button>,
    ts_disconnect: Option<gtk::Button>,
    ts_rows: RefCell<Vec<adw::ActionRow>>,
    ts_sig: RefCell<String>,
}

// Build one network interface page (mirrors Mission Center's Network view):
// throughput graph (RX filled + TX dashed), speeds/totals, and details.
fn build_net_page(shared: &Arc<Mutex<Shared>>, idx: usize, info: &NetInfo) -> (adw::PreferencesPage, NetUi) {
    let page = adw::PreferencesPage::new();

    let g_head = adw::PreferencesGroup::builder().title(info.kind.clone()).build();
    let row = adw::ActionRow::builder()
        .title(if info.model.is_empty() { info.iface.clone() } else { info.model.clone() })
        .subtitle(info.iface.clone())
        .build();
    g_head.add(&row);
    page.add(&g_head);

    let g_graph = adw::PreferencesGroup::builder().title("Throughput (1 minute)").build();
    let scale_lbl = gtk::Label::new(Some("0 Kbps"));
    scale_lbl.add_css_class("dim-label");
    g_graph.set_header_suffix(Some(&scale_lbl));
    let area = gtk::DrawingArea::new();
    area.set_content_height(150);
    area.set_hexpand(true);
    area.add_css_class("cpu-graph-frame");
    area.set_margin_top(6);
    area.set_margin_bottom(6);
    {
        let sh = shared.clone();
        area.set_draw_func(move |_a, cr, w, h| {
            if let Ok(g) = sh.lock() {
                if let Some(n) = g.nets.get(idx) {
                    let mx = n
                        .rx_hist
                        .iter()
                        .chain(n.tx_hist.iter())
                        .cloned()
                        .fold(1.0_f64, f64::max)
                        * 1.15;
                    // RX filled (shared blue style)
                    draw_graph(cr, w as f64, h as f64, &n.rx_hist, mx);
                    // TX dashed line on top
                    let (w, h) = (w as f64, h as f64);
                    let n2 = n.tx_hist.len();
                    if n2 >= 2 && mx > 0.0 {
                        let stepx = w / (n2 as f64 - 1.0);
                        let yv = |v: f64| h - (v.clamp(0.0, mx) / mx) * (h - 2.0) - 1.0;
                        cr.set_line_width(1.4);
                        cr.set_dash(&[4.0, 3.0], 0.0);
                        cr.set_source_rgb(0.6, 0.6, 0.6);
                        for (i, v) in n.tx_hist.iter().enumerate() {
                            let (x, y) = (i as f64 * stepx, yv(*v));
                            if i == 0 {
                                cr.move_to(x, y);
                            } else {
                                cr.line_to(x, y);
                            }
                        }
                        let _ = cr.stroke();
                        cr.set_dash(&[], 0.0);
                    }
                }
            }
        });
    }
    g_graph.add(&area);
    page.add(&g_graph);

    let mut lbl = std::collections::HashMap::new();
    let g_stat = adw::PreferencesGroup::builder().title("Statistics").build();
    lbl.insert("rspeed", info_row("Receive", &g_stat));
    lbl.insert("sspeed", info_row("Send", &g_stat));
    lbl.insert("trx", info_row("Total Received", &g_stat));
    lbl.insert("ttx", info_row("Total Sent", &g_stat));
    page.add(&g_stat);

    let g_det = adw::PreferencesGroup::builder().title("Details").build();
    let det = |t: &'static str, gr: &adw::PreferencesGroup, lm: &mut std::collections::HashMap<&'static str, gtk::Label>, key: &'static str| {
        let l = info_row(t, gr);
        l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        l.set_max_width_chars(30);
        l.set_selectable(true);
        lm.insert(key, l);
    };
    // static rows
    let l_iface = info_row("Interface Name", &g_det);
    l_iface.set_text(&info.iface);
    let l_type = info_row("Connection Type", &g_det);
    l_type.set_text(&info.kind);
    lbl.insert("status", info_row("Status", &g_det));
    if info.is_wireless {
        det("SSID", &g_det, &mut lbl, "ssid");
        lbl.insert("signal", info_row("Signal Strength", &g_det));
        lbl.insert("freq", info_row("Frequency", &g_det));
    }
    det("Hardware Address", &g_det, &mut lbl, "mac");
    if let Some(l) = lbl.get("mac") {
        l.set_text(if info.mac.is_empty() { "—" } else { &info.mac });
    }
    det("IPv4 Address", &g_det, &mut lbl, "ipv4");
    det("IPv6 Address", &g_det, &mut lbl, "ipv6");
    page.add(&g_det);

    // For the Tailscale interface, add a live tailnet device list (name + IP)
    // plus Connect / Disconnect controls (operator user → no sudo needed).
    let (ts_group, ts_connect, ts_disconnect) = if info.iface.starts_with("tailscale") {
        let g = adw::PreferencesGroup::builder()
            .title("Tailnet Devices")
            .description("Devices connected to your Tailscale network")
            .build();

        let ctrl = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let b_conn = gtk::Button::builder().label("Connect").valign(gtk::Align::Center).build();
        b_conn.add_css_class("suggested-action");
        b_conn.connect_clicked(|_| run_user(vec!["tailscale".into(), "up".into()]));
        let b_disc = gtk::Button::builder().label("Disconnect").valign(gtk::Align::Center).build();
        b_disc.connect_clicked(|_| run_user(vec!["tailscale".into(), "down".into()]));
        ctrl.append(&b_conn);
        ctrl.append(&b_disc);
        g.set_header_suffix(Some(&ctrl));

        page.add(&g);
        (Some(g), Some(b_conn), Some(b_disc))
    } else {
        (None, None, None)
    };

    let nu = NetUi {
        idx,
        area,
        scale_lbl,
        lbl,
        ts_group,
        ts_connect,
        ts_disconnect,
        ts_rows: RefCell::new(Vec::new()),
        ts_sig: RefCell::new(String::new()),
    };
    (page, nu)
}

struct BatUi {
    idx: usize,
    power_area: Option<gtk::DrawingArea>,
    power_scale: Option<gtk::Label>,
    pct_area: gtk::DrawingArea,
    lbl: std::collections::HashMap<&'static str, gtk::Label>,
}

// Build one battery page (mirrors Mission Center's Battery view): system battery
// shows a power-input graph + charge graph and full details; peripheral batteries
// show a percentage graph + basic info.
fn build_bat_page(shared: &Arc<Mutex<Shared>>, idx: usize, info: &BatInfo) -> (adw::PreferencesPage, BatUi) {
    let page = adw::PreferencesPage::new();

    let g_head = adw::PreferencesGroup::builder().title(format!("Battery {idx}")).build();
    let row = adw::ActionRow::builder()
        .title(if info.model.is_empty() { info.name.clone() } else { info.model.clone() })
        .subtitle(if info.is_system { "System Battery" } else { "Peripheral" })
        .build();
    g_head.add(&row);
    page.add(&g_head);

    // System battery: Power Input graph
    let (mut power_area, mut power_scale) = (None, None);
    if info.is_system {
        let g_pow = adw::PreferencesGroup::builder().title("Power Input (1 minute)").build();
        let ps = gtk::Label::new(Some("0.00 W"));
        ps.add_css_class("dim-label");
        g_pow.set_header_suffix(Some(&ps));
        let pa = gtk::DrawingArea::new();
        pa.set_content_height(130);
        pa.set_hexpand(true);
        pa.add_css_class("cpu-graph-frame");
        pa.set_margin_top(6);
        pa.set_margin_bottom(6);
        {
            let sh = shared.clone();
            pa.set_draw_func(move |_a, cr, w, h| {
                if let Ok(g) = sh.lock() {
                    if let Some(b) = g.bats.get(idx) {
                        let mx = b.power_hist.iter().cloned().fold(1.0_f64, f64::max) * 1.15;
                        draw_graph(cr, w as f64, h as f64, &b.power_hist, mx);
                    }
                }
            });
        }
        g_pow.add(&pa);
        page.add(&g_pow);
        power_area = Some(pa);
        power_scale = Some(ps);
    }

    // Charge / percentage graph
    let g_pct = adw::PreferencesGroup::builder()
        .title(if info.is_system { "Charge (1 minute)" } else { "Percentage (1 minute)" })
        .build();
    let pct_area = gtk::DrawingArea::new();
    pct_area.set_content_height(130);
    pct_area.set_hexpand(true);
    pct_area.add_css_class("cpu-graph-frame");
    pct_area.set_margin_top(6);
    pct_area.set_margin_bottom(6);
    {
        let sh = shared.clone();
        pct_area.set_draw_func(move |_a, cr, w, h| {
            if let Ok(g) = sh.lock() {
                if let Some(b) = g.bats.get(idx) {
                    draw_graph(cr, w as f64, h as f64, &b.pct_hist, 100.0);
                }
            }
        });
    }
    g_pct.add(&pct_area);
    page.add(&g_pct);

    let mut lbl = std::collections::HashMap::new();
    let g_stat = adw::PreferencesGroup::builder().title("Statistics").build();
    lbl.insert("pct", info_row("Persentase", &g_stat));
    if info.is_system {
        lbl.insert("volt", info_row("Voltage", &g_stat));
        lbl.insert("power", info_row("Power", &g_stat));
    }
    lbl.insert("state", info_row("Status", &g_stat));
    if info.is_system {
        lbl.insert("cycles", info_row("Charge Cycles", &g_stat));
    }
    page.add(&g_stat);

    let g_det = adw::PreferencesGroup::builder().title("Details").build();
    let det = |t: &'static str, gr: &adw::PreferencesGroup, lm: &mut std::collections::HashMap<&'static str, gtk::Label>, key: &'static str| {
        let l = info_row(t, gr);
        l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        l.set_max_width_chars(28);
        l.set_selectable(true);
        lm.insert(key, l);
    };
    det("Serial", &g_det, &mut lbl, "serial");
    let l_type = info_row("Type", &g_det);
    l_type.set_text(if info.is_system { "Battery" } else { "Peripheral" });
    let l_pw = info_row("Powering System", &g_det);
    l_pw.set_text(if info.is_system { "Yes" } else { "No" });
    if info.is_system {
        lbl.insert("tech", info_row("Technology", &g_det));
        lbl.insert("health", info_row("Capacity (Health)", &g_det));
        lbl.insert("ef", info_row("Energy Full", &g_det));
        lbl.insert("efd", info_row("Energy Full (desain)", &g_det));
        lbl.insert("vmin", info_row("Voltage min (desain)", &g_det));
        lbl.insert("thr", info_row("Charge Threshold", &g_det));
    }
    page.add(&g_det);

    let bu = BatUi { idx, power_area, power_scale, pct_area, lbl };
    (page, bu)
}

fn fmt_kib(kb: u64) -> String {
    let x = kb as f64;
    if x >= 1048576.0 {
        format!("{:.1} GiB", x / 1048576.0)
    } else if x >= 1024.0 {
        format!("{:.0} MiB", x / 1024.0)
    } else {
        format!("{kb} KiB")
    }
}

struct AppsUi {
    row_hdr: adw::ActionRow,
    list: gtk::ListBox,
    sel: Rc<Cell<Option<u32>>>,
    query: Rc<RefCell<String>>,
    sig: RefCell<String>,
}

// Build the Apps/Processes tab (Mission Center-style task manager): a live
// process table (Name, PID, CPU%, Memory, Swap) sorted by CPU, with Stop
// (SIGTERM) / Force stop (SIGKILL) acting on the selected row.
fn build_apps_page(_shared: &Arc<Mutex<Shared>>) -> (adw::PreferencesPage, AppsUi) {
    let page = adw::PreferencesPage::new();

    let g_head = adw::PreferencesGroup::builder().title("Applications &amp; Processes").build();
    let row_hdr = adw::ActionRow::builder().title("Processes").subtitle("Loading...").build();
    let sel: Rc<Cell<Option<u32>>> = Rc::new(Cell::new(None));
    let btn_stop = gtk::Button::builder().label("Stop").valign(gtk::Align::Center).build();
    let btn_kill = gtk::Button::builder().label("Force").valign(gtk::Align::Center).build();
    btn_kill.add_css_class("destructive-action");
    {
        let sel = sel.clone();
        btn_stop.connect_clicked(move |_| {
            if let Some(pid) = sel.get() {
                run_user(vec!["kill".into(), pid.to_string()]);
            }
        });
    }
    {
        let sel = sel.clone();
        btn_kill.connect_clicked(move |_| {
            if let Some(pid) = sel.get() {
                run_user(vec!["kill".into(), "-9".into(), pid.to_string()]);
            }
        });
    }
    row_hdr.add_suffix(&btn_stop);
    row_hdr.add_suffix(&btn_kill);
    g_head.add(&row_hdr);
    page.add(&g_head);

    let g_tbl = adw::PreferencesGroup::builder().title("Processes (by CPU usage)").build();
    let query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search name / PID / port..."));
    search.set_hexpand(true);
    search.set_margin_start(4);
    search.set_margin_end(4);
    search.set_margin_bottom(6);
    {
        let query = query.clone();
        search.connect_search_changed(move |e| {
            *query.borrow_mut() = e.text().to_string().to_lowercase();
        });
    }
    let col = |t: &str, w: i32, xalign: f32| {
        let l = gtk::Label::new(Some(t));
        l.set_xalign(xalign);
        if w > 0 {
            l.set_size_request(w, -1);
        } else {
            l.set_hexpand(true);
        }
        l.add_css_class("dim-label");
        l
    };
    let hdr = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    hdr.set_margin_start(12);
    hdr.set_margin_end(12);
    hdr.set_margin_top(6);
    hdr.set_margin_bottom(6);
    hdr.append(&col("Name", 0, 0.0));
    hdr.append(&col("PID", 56, 1.0));
    hdr.append(&col("CPU", 48, 1.0));
    hdr.append(&col("Memory", 78, 1.0));
    hdr.append(&col("Swap", 66, 1.0));
    hdr.append(&col("Drive", 78, 1.0));
    hdr.append(&col("Port", 96, 1.0));

    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    {
        let sel = sel.clone();
        list.connect_row_selected(move |_lb, row| {
            sel.set(row.and_then(|r| r.widget_name().parse::<u32>().ok()));
        });
    }
    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(&list));
    sw.set_min_content_height(440);
    sw.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);

    let vb = gtk::Box::new(gtk::Orientation::Vertical, 0);
    vb.append(&search);
    vb.append(&hdr);
    vb.append(&sw);
    g_tbl.add(&vb);
    page.add(&g_tbl);

    let ui = AppsUi { row_hdr, list, sel, query, sig: RefCell::new(String::new()) };
    (page, ui)
}

fn svc_action(action: &str, unit: &str, is_user: bool) {
    if is_user {
        run_user(vec!["systemctl".into(), "--user".into(), action.into(), unit.into()]);
    } else {
        // pkexec shows a graphical polkit prompt for privileged system-unit control
        run_user(vec!["pkexec".into(), "systemctl".into(), action.into(), unit.into()]);
    }
}

struct SvcAllUi {
    row_hdr: adw::ActionRow,
    list: gtk::ListBox,
    filter: Rc<Cell<u8>>, // 0 all, 1 running, 2 failed
    sel: Rc<RefCell<Option<(String, bool)>>>,
    sig: RefCell<String>,
}

// Full systemd Services view (Mission Center-style): all system + user units,
// grouped, with state dot, memory, filter chips, and Start/Stop/Restart on the
// selected unit (user units via `systemctl --user`, system units via pkexec).
fn build_services_all_page() -> (adw::PreferencesPage, SvcAllUi) {
    let page = adw::PreferencesPage::new();
    let sel: Rc<RefCell<Option<(String, bool)>>> = Rc::new(RefCell::new(None));
    let filter: Rc<Cell<u8>> = Rc::new(Cell::new(0));

    let g_head = adw::PreferencesGroup::builder().title("All Services").build();
    let row_hdr = adw::ActionRow::builder().title("Services").subtitle("Loading...").build();
    for (label, act) in [("Start", "start"), ("Stop", "stop"), ("Restart", "restart")] {
        let b = gtk::Button::builder().label(label).valign(gtk::Align::Center).build();
        if act == "stop" {
            b.add_css_class("destructive-action");
        }
        let sel2 = sel.clone();
        b.connect_clicked(move |_| {
            if let Some((unit, is_user)) = sel2.borrow().clone() {
                svc_action(act, &unit, is_user);
            }
        });
        row_hdr.add_suffix(&b);
    }
    g_head.add(&row_hdr);

    // filter chips
    let frow = adw::ActionRow::builder().title("Filter").build();
    let fbox = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    fbox.set_valign(gtk::Align::Center);
    let b_all = gtk::ToggleButton::with_label("All");
    let b_run = gtk::ToggleButton::with_label("Running");
    let b_fail = gtk::ToggleButton::with_label("Failed");
    b_run.set_group(Some(&b_all));
    b_fail.set_group(Some(&b_all));
    b_all.set_active(true);
    for (b, v) in [(&b_all, 0u8), (&b_run, 1), (&b_fail, 2)] {
        let filter = filter.clone();
        b.connect_toggled(move |btn| {
            if btn.is_active() {
                filter.set(v);
            }
        });
        fbox.append(b);
    }
    frow.add_suffix(&fbox);
    g_head.add(&frow);
    page.add(&g_head);

    let g_tbl = adw::PreferencesGroup::builder().title("Unit systemd").build();
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    {
        let sel = sel.clone();
        list.connect_row_selected(move |_lb, row| {
            let v = row.and_then(|r| {
                let n = r.widget_name();
                let n = n.as_str();
                n.split_once(':').map(|(p, u)| (u.to_string(), p == "U"))
            });
            *sel.borrow_mut() = v;
        });
    }
    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(&list));
    sw.set_min_content_height(480);
    sw.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    g_tbl.add(&sw);
    page.add(&g_tbl);

    let ui = SvcAllUi { row_hdr, list, filter, sel, sig: RefCell::new(String::new()) };
    (page, ui)
}

#[derive(Default, Clone)]
struct SshHost {
    alias: String,
    hostname: String,
    user: String,
    port: String,
    identity: String,
}

// Parse ~/.ssh/config into connectable host entries. Wildcard patterns and
// URL-looking "hosts" are skipped (they are not real ssh destinations).
fn parse_ssh_hosts() -> Vec<SshHost> {
    let home = std::env::var("HOME").unwrap_or_default();
    let text = match fs::read_to_string(format!("{home}/.ssh/config")) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut hosts: Vec<SshHost> = Vec::new();
    let mut cur: Option<SshHost> = None;
    let mut skip = false;
    let flush = |hosts: &mut Vec<SshHost>, cur: &mut Option<SshHost>, skip: bool| {
        if let Some(h) = cur.take() {
            if !skip && !h.alias.is_empty() {
                hosts.push(h);
            }
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let (key, val) = match t.split_once(char::is_whitespace) {
            Some((k, v)) => (k.to_ascii_lowercase(), v.trim()),
            None => (t.to_ascii_lowercase(), ""),
        };
        match key.as_str() {
            "host" => {
                flush(&mut hosts, &mut cur, skip);
                let alias = val.split_whitespace().next().unwrap_or("").to_string();
                // Skip wildcard patterns and URL-looking pseudo-hosts.
                skip = alias.is_empty()
                    || alias.contains('*')
                    || alias.contains('?')
                    || alias.contains("://");
                cur = Some(SshHost { alias, ..Default::default() });
            }
            "hostname" => {
                if let Some(h) = cur.as_mut() {
                    h.hostname = val.to_string();
                }
            }
            "user" => {
                if let Some(h) = cur.as_mut() {
                    h.user = val.to_string();
                }
            }
            "port" => {
                if let Some(h) = cur.as_mut() {
                    h.port = val.to_string();
                }
            }
            "identityfile" => {
                if let Some(h) = cur.as_mut() {
                    h.identity = val.to_string();
                }
            }
            _ => {}
        }
    }
    flush(&mut hosts, &mut cur, skip);
    hosts
}

// SSH manager tab: lists ~/.ssh/config hosts; Connect opens ssh in a new
// Ghostty window (`ghostty -e ssh <alias>`), so ssh_config resolves the
// user/port/identity automatically.
fn build_ssh_page() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    let g_head = adw::PreferencesGroup::builder()
        .title("SSH Connections")
        .description("Hosts from ~/.ssh/config — Connect opens a new Ghostty terminal")
        .build();
    let btn_term = gtk::Button::builder().label("Open Terminal").valign(gtk::Align::Center).build();
    btn_term.connect_clicked(|_| run_user(vec!["ghostty".into()]));
    g_head.set_header_suffix(Some(&btn_term));
    page.add(&g_head);

    let g_list = adw::PreferencesGroup::builder().title("Configured Hosts").build();
    page.add(&g_list);

    // (Re)build the host rows from ssh config.
    let rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let rebuild = {
        let g_list = g_list.clone();
        let rows = rows.clone();
        move || {
            for r in rows.borrow_mut().drain(..) {
                g_list.remove(&r);
            }
            let hosts = parse_ssh_hosts();
            let mut rlist = rows.borrow_mut();
            if hosts.is_empty() {
                let row = adw::ActionRow::builder()
                    .title("No hosts found")
                    .subtitle("Add entries to ~/.ssh/config")
                    .build();
                g_list.add(&row);
                rlist.push(row);
                return;
            }
            for h in hosts {
                let mut sub = String::new();
                if !h.user.is_empty() {
                    sub.push_str(&h.user);
                    sub.push('@');
                }
                sub.push_str(if h.hostname.is_empty() { &h.alias } else { &h.hostname });
                if !h.port.is_empty() && h.port != "22" {
                    sub.push(':');
                    sub.push_str(&h.port);
                }
                let row = adw::ActionRow::builder().title(&h.alias).subtitle(&sub).build();
                row.add_prefix(&lucide("lucide-terminal", 20));

                let btn = gtk::Button::builder()
                    .label("Connect")
                    .valign(gtk::Align::Center)
                    .build();
                btn.add_css_class("suggested-action");
                {
                    let alias = h.alias.clone();
                    btn.connect_clicked(move |_| {
                        run_user(vec!["ghostty".into(), "-e".into(), "ssh".into(), alias.clone()]);
                    });
                }
                row.add_suffix(&btn);
                row.set_activatable_widget(Some(&btn));
                g_list.add(&row);
                rlist.push(row);
            }
        }
    };
    rebuild();

    let btn_refresh = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .valign(gtk::Align::Center)
        .tooltip_text("Reload ~/.ssh/config")
        .build();
    {
        let rebuild = rebuild.clone();
        btn_refresh.connect_clicked(move |_| rebuild());
    }
    g_list.set_header_suffix(Some(&btn_refresh));

    page
}


// ─── Network Speed Test ───────────────────────────────────────────────────────
fn build_speedtest_page() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    let g_head = adw::PreferencesGroup::builder()
        .title("Network Speed Test")
        .description("Measure download, upload, and latency")
        .build();

    // Result labels
    let lbl_dl = gtk::Label::new(Some("-- Mbps"));
    lbl_dl.set_xalign(0.0);
    let row_dl = adw::ActionRow::builder().title("Download").build();
    row_dl.add_suffix(&lbl_dl);
    g_head.add(&row_dl);

    let lbl_ul = gtk::Label::new(Some("-- Mbps"));
    lbl_ul.set_xalign(0.0);
    let row_ul = adw::ActionRow::builder().title("Upload").build();
    row_ul.add_suffix(&lbl_ul);
    g_head.add(&row_ul);

    let lbl_ping = gtk::Label::new(Some("-- ms"));
    lbl_ping.set_xalign(0.0);
    let row_ping = adw::ActionRow::builder().title("Latency (Ping)").build();
    row_ping.add_suffix(&lbl_ping);
    g_head.add(&row_ping);

    // Status label for errors/messages
    let lbl_status = gtk::Label::new(None);
    lbl_status.set_xalign(0.0);
    lbl_status.add_css_class("dimmed");
    let row_status = adw::ActionRow::new();
    row_status.set_child(Some(&lbl_status));
    row_status.set_activatable(false);
    g_head.add(&row_status);

    // Run Test button
    let btn_test = gtk::Button::builder()
        .label("Run Test")
        .halign(gtk::Align::Start)
        .build();
    btn_test.add_css_class("suggested-action");
    let brow = adw::ActionRow::new();
    brow.set_child(Some(&btn_test));
    brow.set_activatable(false);
    g_head.add(&brow);
    page.add(&g_head);

    // ── History (last 5 results) ──
    let g_hist = adw::PreferencesGroup::builder()
        .title("History (last 5 tests)")
        .build();
    let hist_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let hist_row = adw::ActionRow::new();
    hist_row.set_child(Some(&hist_box));
    hist_row.set_activatable(false);
    g_hist.add(&hist_row);
    page.add(&g_hist);

    // Shared state: history entries
    let history: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    // Connect button
    {
        let lbl_dl = lbl_dl.clone();
        let lbl_ul = lbl_ul.clone();
        let lbl_ping = lbl_ping.clone();
        let lbl_status = lbl_status.clone();
        let btn_test = btn_test.clone();
        let hist_box = hist_box.clone();
        let history = history.clone();

        btn_test.connect_clicked(move |btn| {
            btn.set_label("Testing\u{2026}");
            btn.set_sensitive(false);
            lbl_status.set_text("");
            lbl_dl.set_text("measuring...");
            lbl_ul.set_text("measuring...");
            lbl_ping.set_text("measuring...");

            let result: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
            let result2 = result.clone();

            // Run speed test in background thread
            std::thread::spawn(move || {
                let output = if let Ok(st) = Command::new("which").arg("speedtest").output() {
                    if st.status.success() {
                        // Use speedtest CLI (Ookla)
                        Command::new("speedtest")
                            .arg("--simple")
                            .output()
                            .ok()
                            .map(|o| {
                                let raw = String::from_utf8_lossy(&o.stdout).to_string();
                                parse_speedtest_cli(&raw)
                            })
                    } else if let Ok(sc) = Command::new("which").arg("speedtest-cli").output() {
                        if sc.status.success() {
                            Command::new("speedtest-cli")
                                .arg("--simple")
                                .output()
                                .ok()
                                .map(|o| {
                                    let raw = String::from_utf8_lossy(&o.stdout).to_string();
                                    parse_speedtest_cli(&raw)
                                })
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                let final_result = output.unwrap_or_else(|| {
                    // Fallback: run our script
                    let sp = script_path("asus-speedtest.sh");
                    match Command::new("bash").arg(&sp).output() {
                        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
                        Err(_) => "TOOL_MISSING=1\n".to_string(),
                    }
                });

                if let Ok(mut r) = result2.lock() {
                    *r = Some(final_result);
                }
            });

            // Poll result on main thread
            let lbl_dl2 = lbl_dl.clone();
            let lbl_ul2 = lbl_ul.clone();
            let lbl_ping2 = lbl_ping.clone();
            let lbl_status2 = lbl_status.clone();
            let btn2 = btn.clone();
            let hist_box2 = hist_box.clone();
            let history2 = history.clone();

            glib::timeout_add_local(Duration::from_millis(200), move || {
                if let Ok(mut guard) = result.lock() {
                    if let Some(text) = guard.take() {
                        let mut dl = String::new();
                        let mut ul = String::new();
                        let mut ping = String::new();
                        let mut missing = false;

                        for line in text.lines() {
                            if let Some(v) = line.strip_prefix("DOWNLOAD_MBPS=") {
                                dl = v.trim().to_string();
                            } else if let Some(v) = line.strip_prefix("UPLOAD_MBPS=") {
                                ul = v.trim().to_string();
                            } else if let Some(v) = line.strip_prefix("PING_MS=") {
                                ping = v.trim().to_string();
                            } else if line.contains("TOOL_MISSING=1") {
                                missing = true;
                            }
                        }

                        if missing {
                            lbl_status2.set_text("No speed test tool or network available.");
                            lbl_dl2.set_text("-- Mbps");
                            lbl_ul2.set_text("-- Mbps");
                            lbl_ping2.set_text("-- ms");
                        } else {
                            let dl_disp = if dl.is_empty() { "N/A".into() } else { format!("{dl} Mbps") };
                            let ul_disp = if ul.is_empty() { "N/A".into() } else { format!("{ul} Mbps") };
                            let ping_disp = if ping.is_empty() { "N/A".into() } else { format!("{ping} ms") };
                            lbl_dl2.set_text(&dl_disp);
                            lbl_ul2.set_text(&ul_disp);
                            lbl_ping2.set_text(&ping_disp);

                            // Add to history
                            let now = glib::DateTime::now_local().map(|d| d.format("%H:%M:%S").unwrap_or_default().to_string()).unwrap_or_else(|_| "??:??:??".into());
                            let entry = format!("[{now}]  \u{2193}{dl_disp}  \u{2191}{ul_disp}  Ping: {ping_disp}");
                            let mut h = history2.borrow_mut();
                            h.push(entry);
                            if h.len() > 5 {
                                h.remove(0);
                            }
                            // Rebuild history display
                            while let Some(child) = hist_box2.first_child() {
                                hist_box2.remove(&child);
                            }
                            for e in h.iter() {
                                let l = gtk::Label::new(Some(e));
                                l.set_xalign(0.0);
                                l.add_css_class("monospace");
                                hist_box2.append(&l);
                            }
                        }

                        btn2.set_label("Run Test");
                        btn2.set_sensitive(true);
                        return glib::ControlFlow::Break;
                    }
                }
                glib::ControlFlow::Continue
            });
        });
    }

    page
}

/// Parse output from `speedtest --simple` / `speedtest-cli --simple`.
/// Example output:
///   Ping: 12.345 ms
///   Download: 94.52 Mbit/s
///   Upload: 45.23 Mbit/s
fn parse_speedtest_cli(raw: &str) -> String {
    let mut dl = String::new();
    let mut ul = String::new();
    let mut ping = String::new();
    for line in raw.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("download:") {
            // Extract number before "mbit" or "mbps"
            if let Some(num) = line.split_whitespace().nth(1) {
                dl = num.to_string();
            }
        } else if lower.starts_with("upload:") {
            if let Some(num) = line.split_whitespace().nth(1) {
                ul = num.to_string();
            }
        } else if lower.starts_with("ping:") {
            if let Some(num) = line.split_whitespace().nth(1) {
                ping = num.to_string();
            }
        }
    }
    format!("DOWNLOAD_MBPS={dl}\nUPLOAD_MBPS={ul}\nPING_MS={ping}\n")
}


// ─── System Log Viewer ────────────────────────────────────────────────────────
fn build_logs_page() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    let g_head = adw::PreferencesGroup::builder()
        .title("System Logs")
        .description("View journalctl logs filtered by priority &amp; time range")
        .build();

    // ── Filter bar ──
    let fbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    fbox.set_margin_top(4);
    fbox.set_margin_bottom(4);

    // Priority dropdown
    let prio_model = gtk::StringList::new(&["All", "Error", "Warning", "Info"]);
    let prio_dd = gtk::DropDown::builder()
        .model(&prio_model)
        .selected(0)
        .valign(gtk::Align::Center)
        .build();
    prio_dd.set_size_request(110, -1);

    // Time-range dropdown
    let time_model = gtk::StringList::new(&["1 hour", "6 hours", "24 hours", "7 days"]);
    let time_dd = gtk::DropDown::builder()
        .model(&time_model)
        .selected(0)
        .valign(gtk::Align::Center)
        .build();
    time_dd.set_size_request(110, -1);

    // Search entry
    let search_entry = gtk::Entry::builder()
        .placeholder_text("Search logs...")
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();

    fbox.append(&gtk::Label::new(Some("Priority:")));
    fbox.append(&prio_dd);
    fbox.append(&gtk::Label::new(Some("Time:")));
    fbox.append(&time_dd);
    fbox.append(&search_entry);

    let frow = adw::ActionRow::new();
    frow.set_child(Some(&fbox));
    frow.set_activatable(false);
    g_head.add(&frow);
    page.add(&g_head);

    // ── Log display area ──
    let g_logs = adw::PreferencesGroup::builder().title("Log Entries").build();

    let text_view = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    text_view.add_css_class("card");

    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(&text_view));
    sw.set_min_content_height(420);
    sw.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    g_logs.add(&sw);

    // ── Buttons (Refresh + Load More) ──
    let btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk::Align::End);
    btn_box.set_margin_top(8);
    let btn_refresh = gtk::Button::builder().label("Refresh").valign(gtk::Align::Center).build();
    btn_refresh.add_css_class("suggested-action");
    let btn_more = gtk::Button::builder().label("Load More").valign(gtk::Align::Center).build();
    btn_box.append(&btn_refresh);
    btn_box.append(&btn_more);
    let brow = adw::ActionRow::new();
    brow.set_child(Some(&btn_box));
    brow.set_activatable(false);
    g_logs.add(&brow);
    page.add(&g_logs);

    // ── Shared state for log offset (for Load More pagination) ──
    let log_offset: Rc<Cell<u32>> = Rc::new(Cell::new(0));

    // Fetch logs helper — runs journalctl in a background thread, sends text
    // back via Arc<Mutex<>> + glib::idle_add_local_once (GTK objects stay on
    // main thread).
    let fetch_logs = {
        let text_view = text_view.clone();
        let prio_dd = prio_dd.clone();
        let time_dd = time_dd.clone();
        let search_entry = search_entry.clone();
        let log_offset = log_offset.clone();
        move |append: bool| {
            let prio_idx = prio_dd.selected();
            let time_idx = time_dd.selected();
            let search_text = search_entry.text().to_string();
            let offset = if append { log_offset.get() } else { 0u32 };

            let result: Arc<Mutex<Option<(String, u32)>>> = Arc::new(Mutex::new(None));
            let result2 = result.clone();

            std::thread::spawn(move || {
                let mut args: Vec<&str> = vec![
                    "journalctl",
                    "--no-pager",
                    "-o", "short-iso",
                    "-n", "500",
                ];

                // Priority filter
                match prio_idx {
                    1 => { args.push("-p"); args.push("err"); }
                    2 => { args.push("-p"); args.push("warning"); }
                    3 => { args.push("-p"); args.push("info"); }
                    _ => {} // All
                }

                // Time range
                let since = match time_idx {
                    1 => "6 hours ago",
                    2 => "24 hours ago",
                    3 => "7 days ago",
                    _ => "1 hour ago",
                };
                args.push("--since");
                args.push(since);

                let output = Command::new(args[0])
                    .args(&args[1..])
                    .output();

                let text = match output {
                    Ok(o) => {
                        let raw = String::from_utf8_lossy(&o.stdout).to_string();
                        if raw.trim().is_empty() {
                            "(no log entries found)".to_string()
                        } else {
                            raw
                        }
                    }
                    Err(e) => format!("Error running journalctl: {e}"),
                };

                // Client-side search filter
                let filtered = if search_text.is_empty() {
                    text
                } else {
                    let lower = search_text.to_lowercase();
                    text.lines()
                        .filter(|l| l.to_lowercase().contains(&lower))
                        .collect::<Vec<_>>()
                        .join("\n")
                };

                // Pagination: if offset>0, skip first `offset` lines
                let final_text = if offset > 0 {
                    let lines: Vec<&str> = filtered.lines().collect();
                    let skip = offset as usize;
                    if skip >= lines.len() {
                        "(no more entries)".to_string()
                    } else {
                        lines[skip..].join("\n")
                    }
                } else {
                    filtered
                };

                let line_count = final_text.lines().count() as u32;
                if let Ok(mut r) = result2.lock() {
                    *r = Some((final_text, line_count));
                }
            });

            // Poll result from main thread via short idle timer
            let tv = text_view.clone();
            let lo = log_offset.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if let Ok(mut guard) = result.lock() {
                    if let Some((text, count)) = guard.take() {
                        let buf = tv.buffer();
                        if append {
                            let mut end = buf.end_iter();
                            buf.insert(&mut end, "\n");
                            buf.insert(&mut end, &text);
                        } else {
                            buf.set_text(&text);
                        }
                        lo.set(if append { lo.get() + count } else { count });
                        return glib::ControlFlow::Break;
                    }
                }
                glib::ControlFlow::Continue
            });
        }
    };

    // Wire Refresh button
    {
        let fetch = fetch_logs.clone();
        btn_refresh.connect_clicked(move |_| fetch(false));
    }

    // Wire Load More button
    {
        let fetch = fetch_logs.clone();
        btn_more.connect_clicked(move |_| fetch(true));
    }

    // Initial load
    fetch_logs(false);

    page
}

fn build_ui(app: &adw::Application) {
    // Force full-dark scheme regardless of the system theme.
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    // Keep tab padding identical across pages: overlay scrollbars never reserve
    // horizontal space, so a tall (scrolling) page and a short one align the same.
    if let Some(settings) = gtk::Settings::default() {
        settings.set_property("gtk-overlay-scrolling", true);
    }

    let logical = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let shared = Arc::new(Mutex::new(Shared {
        logical,
        per_core: vec![VecDeque::from(vec![0.0; HISTORY]); logical],
        temp_hist: VecDeque::from(vec![0.0; HISTORY]),
        mem_hist: VecDeque::from(vec![0.0; HISTORY]),
        swap_hist: VecDeque::from(vec![0.0; HISTORY]),
        visible_tab: "cpu".into(),
        pause_hidden: true,
        alerts_enabled: true,
        ..Default::default()
    }));
    gather_drives_static(&shared);
    gather_gpus_static(&shared);
    gather_fans_static(&shared);
    gather_nets_static(&shared);
    gather_bats_static(&shared);
    spawn_sampler(shared.clone());

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Tweaks ASUS TUF")
        .icon_name("com.rezkycodes.AsusTufCpu")
        .default_width(920)
        .default_height(860)
        .build();

    let stack = adw::ViewStack::new();

    // ── CPU page ──
    let mut core_areas: Vec<gtk::DrawingArea> = Vec::new();
    let (cpu_page, row_model, util_bar, temp_area, labels) = build_cpu_page(&shared, &mut core_areas);
    let sp = stack.add_titled(&cpu_page, Some("cpu"), "CPU");
    sp.set_icon_name(Some("computer-symbolic"));

    // ── Memory page (after CPU) ──
    let (memory_page, row_mem, mem_bar, mem_area, swap_area, mem_lbl) = build_memory_page(&shared);
    let mep = stack.add_titled(&memory_page, Some("memory"), "Memory");
    mep.set_icon_name(Some("drive-harddisk-symbolic"));

    // ── Apps / Processes page ──
    let (apps_page, apps_ui) = build_apps_page(&shared);
    stack.add_titled(&apps_page, Some("apps"), "Applications");

    // ── Full Services page ──
    let (svc_all_page, svc_all_ui) = build_services_all_page();
    stack.add_titled(&svc_all_page, Some("svcall"), "All Services");

    // ── SSH manager page ──
    let ssh_page = build_ssh_page();
    stack.add_titled(&ssh_page, Some("ssh"), "SSH Manager");

    // ── System Logs page ──
    let logs_page = build_logs_page();
    stack.add_titled(&logs_page, Some("logs"), "System Logs");

    // ── Network Speed Test page ──
    let speedtest_page = build_speedtest_page();
    stack.add_titled(&speedtest_page, Some("speedtest"), "Speed Test");

    // ── Power page ──
    let power_page = adw::PreferencesPage::new();

    let g_bat = adw::PreferencesGroup::builder().title("Battery &amp; Power Status").build();
    let row_bat = adw::ActionRow::builder().title("Battery: --%").subtitle("Loading...").build();
    let bat_bar = gtk::LevelBar::builder().min_value(0.0).max_value(100.0).valign(gtk::Align::Center).build();
    bat_bar.set_size_request(110, 16);
    row_bat.add_suffix(&bat_bar);
    g_bat.add(&row_bat);
    let row_drain = adw::ActionRow::builder().title("Power Source &amp; Watts").subtitle("Loading...").build();
    g_bat.add(&row_drain);
    let row_health = adw::ActionRow::builder().title("Battery Health (Factory)").subtitle("Loading...").build();
    g_bat.add(&row_health);
    power_page.add(&g_bat);

    // GPU
    let g_gpu = adw::PreferencesGroup::builder()
        .title("GPU Management &amp; Mode")
        .description("AMD Vega iGPU ↔ NVIDIA GTX 1660 Ti dGPU")
        .build();
    let row_gpu_tel = adw::ActionRow::builder().title("Status GPU NVIDIA").subtitle("Loading telemetry...").build();
    g_gpu.add(&row_gpu_tel);
    let row_gpu_mode = adw::ActionRow::builder().title("Graphics Mode Selection").subtitle("Mode: Hybrid").build();
    let box_gpu = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    box_gpu.set_valign(gtk::Align::Center);
    let btn_gpu = [seg_button("Hybrid"), seg_button("AMD Only"), seg_button("NVIDIA")];
    let gpu_args = ["hybrid", "integrated", "dedicated"];
    for (i, b) in btn_gpu.iter().enumerate() {
        let arg = gpu_args[i].to_string();
        b.connect_clicked(move |_| run_priv(vec![script_path("battery-set-gpu.sh"), arg.clone()]));
        box_gpu.append(b);
    }
    row_gpu_mode.add_suffix(&box_gpu);
    g_gpu.add(&row_gpu_mode);
    power_page.add(&g_gpu);

    // CPU power modes
    let g_mode = adw::PreferencesGroup::builder()
        .title("Performance &amp; Power Mode Control")
        .description("Blue = active")
        .build();
    let row_mode = adw::ActionRow::builder().title("CPU Performance Profile").subtitle("Powersave / Performance / Auto").build();
    let box_mode = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    box_mode.set_valign(gtk::Align::Center);
    let btn_mode = [seg_button("Saver"), seg_button("Performance"), seg_button("Auto")];
    let mode_scripts = ["battery-save.sh", "battery-on-ac.sh", "battery-udev-handler.sh"];
    for (i, b) in btn_mode.iter().enumerate() {
        let sc = mode_scripts[i].to_string();
        b.connect_clicked(move |_| run_priv(vec![script_path(&sc)]));
        box_mode.append(b);
    }
    row_mode.add_suffix(&box_mode);
    g_mode.add(&row_mode);
    power_page.add(&g_mode);

    // Fan
    let g_fan = adw::PreferencesGroup::builder().title("Fan &amp; Cooling Control (Dual Fan)").build();
    let row_fan_rpm = adw::ActionRow::builder().title("Fan Rotation Speed").subtitle("CPU Fan: -- RPM | GPU Fan: -- RPM").build();
    g_fan.add(&row_fan_rpm);
    let row_fan_ctrl = adw::ActionRow::builder().title("Fan Speed Profile").subtitle("Mode: Normal").build();
    let box_fan = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    box_fan.set_valign(gtk::Align::Center);
    let btn_fan = [seg_button("Silent"), seg_button("Normal"), seg_button("Turbo")];
    let fan_args = ["2", "0", "1"]; // silent, normal, turbo
    for (i, b) in btn_fan.iter().enumerate() {
        let arg = fan_args[i].to_string();
        b.connect_clicked(move |_| run_priv(vec![script_path("battery-set-fan.sh"), arg.clone()]));
        box_fan.append(b);
    }
    row_fan_ctrl.add_suffix(&box_fan);
    g_fan.add(&row_fan_ctrl);
    power_page.add(&g_fan);

    // Hardware / server
    let g_hw = adw::PreferencesGroup::builder().title("Fitur Hardware &amp; Server").build();
    let switch_threshold = adw::SwitchRow::builder()
        .title("Battery Charge Limit 80% (Battery Health)")
        .subtitle("Limits charging at 80% to protect the cells")
        .build();
    g_hw.add(&switch_threshold);
    let row_clamshell = adw::ActionRow::builder()
        .title("Lid-Close Mode (Clamshell Server)")
        .subtitle("Screen off when closed, CPU &amp; agent keep running")
        .build();
    let lbl_cs = gtk::Label::new(Some("Active"));
    lbl_cs.add_css_class("success");
    lbl_cs.set_valign(gtk::Align::Center);
    row_clamshell.add_suffix(&lbl_cs);
    g_hw.add(&row_clamshell);
    let row_cpu_mon = adw::ActionRow::builder().title("CPU Monitor Real-time").subtitle("Loading frequency...").build();
    g_hw.add(&row_cpu_mon);
    power_page.add(&g_hw);

    let pp = stack.add_titled(&power_page, Some("power"), "Power & Battery");
    pp.set_icon_name(Some("battery-symbolic"));

    // ── Keyboard RGB page ──
    let rgb_page = build_rgb_page();
    let rgbp = stack.add_titled(&rgb_page, Some("rgb"), "Keyboard RGB");
    rgbp.set_icon_name(Some("input-keyboard-symbolic"));

    // ── Mouse Logitech page ──
    let m_sync = Rc::new(Cell::new(false));
    let m_pending_dpi: Rc<Cell<Option<(u32, Instant)>>> = Rc::new(Cell::new(None));
    let m_pending_hz: Rc<Cell<Option<(u32, Instant)>>> = Rc::new(Cell::new(None));
    let m_pending_led_mode: Rc<Cell<Option<(u32, Instant)>>> = Rc::new(Cell::new(None));
    let m_pending_led_color: Rc<Cell<Option<((u8, u8, u8), Instant)>>> = Rc::new(Cell::new(None));
    let m_debounce: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    let mouse_page = adw::PreferencesPage::new();

    let g_m = adw::PreferencesGroup::builder()
        .title("Logitech G304 Lightspeed Wireless")
        .description("Receiver USB 046d:C53F • Protocol HID++ 4.2")
        .build();
    let row_m_bat = adw::ActionRow::builder().title("G304 Mouse Battery: --%").subtitle("Loading...").build();
    let m_bat_bar = gtk::LevelBar::builder().min_value(0.0).max_value(100.0).valign(gtk::Align::Center).build();
    m_bat_bar.set_size_request(110, 16);
    row_m_bat.add_suffix(&m_bat_bar);
    g_m.add(&row_m_bat);
    mouse_page.add(&g_m);

    // Polling rate
    let g_hz = adw::PreferencesGroup::builder()
        .title("Polling Rate (Data Transfer Frequency Hz)")
        .description("Higher is more responsive")
        .build();
    let row_m_hz = adw::ActionRow::builder().title("Current Polling Rate").subtitle("Loading...").build();
    let box_hz = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    box_hz.set_valign(gtk::Align::Center);
    let mut hz_btns: Vec<(u32, gtk::Button)> = Vec::new();
    for hz in [1000u32, 500, 250, 125] {
        let b = seg_button(&format!("{} Hz", hz));
        let pend = m_pending_hz.clone();
        b.connect_clicked(move |_| {
            run_user(vec![script_path("battery-mouse-logitech.sh"), "hz".into(), hz.to_string()]);
            pend.set(Some((hz, Instant::now())));
        });
        box_hz.append(&b);
        hz_btns.push((hz, b));
    }
    row_m_hz.add_suffix(&box_hz);
    g_hz.add(&row_m_hz);
    mouse_page.add(&g_hz);

    // DPI
    let g_dpi = adw::PreferencesGroup::builder().title("Optical Sensor Sensitivity (DPI)").build();
    let row_m_dpi = adw::ActionRow::builder().title("Current DPI Value").subtitle("-- DPI").build();
    let scale_dpi = gtk::Scale::with_range(gtk::Orientation::Horizontal, 200.0, 12000.0, 50.0);
    scale_dpi.set_size_request(180, -1);
    scale_dpi.set_valign(gtk::Align::Center);
    {
        let st = m_sync.clone();
        let pend = m_pending_dpi.clone();
        let deb = m_debounce.clone();
        scale_dpi.connect_value_changed(move |s| {
            if st.get() {
                return;
            }
            let dpi = s.value() as u32;
            pend.set(Some((dpi, Instant::now())));
            if let Some(id) = deb.take() {
                id.remove();
            }
            let deb2 = deb.clone();
            let id = glib::timeout_add_local(Duration::from_millis(90), move || {
                run_user(vec![script_path("battery-mouse-logitech.sh"), "dpi".into(), dpi.to_string()]);
                deb2.set(None);
                glib::ControlFlow::Break
            });
            deb.set(Some(id));
        });
    }
    row_m_dpi.add_suffix(&scale_dpi);
    g_dpi.add(&row_m_dpi);
    let row_dpi_presets = adw::ActionRow::builder().title("Popular DPI Presets").subtitle("Click to change sensitivity instantly").build();
    let box_dpi = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    box_dpi.set_valign(gtk::Align::Center);
    for dpi in [400u32, 800, 1200, 1600, 3200] {
        let b = seg_button(&dpi.to_string());
        let sc = scale_dpi.clone();
        let st = m_sync.clone();
        let pend = m_pending_dpi.clone();
        b.connect_clicked(move |_| {
            st.set(true);
            sc.set_value(dpi as f64);
            st.set(false);
            pend.set(Some((dpi, Instant::now())));
            run_user(vec![script_path("battery-mouse-logitech.sh"), "dpi".into(), dpi.to_string()]);
        });
        box_dpi.append(&b);
    }
    row_dpi_presets.add_suffix(&box_dpi);
    g_dpi.add(&row_dpi_presets);
    mouse_page.add(&g_dpi);

    // Onboard + USB
    let g_ob = adw::PreferencesGroup::builder().title("Onboard Memory Profile & USB Anti-Lag".replace('&', "&amp;").as_str()).build();
    let switch_onboard = adw::SwitchRow::builder()
        .title("Onboard Memory Profile (EEPROM)")
        .subtitle("Use the profile stored in the G304 mouse's physical memory")
        .build();
    {
        let st = m_sync.clone();
        switch_onboard.connect_active_notify(move |s| {
            if st.get() {
                return;
            }
            let val = if s.is_active() { "1" } else { "off" };
            run_user(vec![script_path("battery-mouse-logitech.sh"), "onboard".into(), val.to_string()]);
        });
    }
    g_ob.add(&switch_onboard);
    let row_usb = adw::ActionRow::builder()
        .title("USB Autosuspend Protection (Anti Micro-Stutter)")
        .subtitle("G304 receiver locked in Power ON mode (Lag-free)")
        .build();
    let lbl_usb = gtk::Label::new(Some("Active"));
    lbl_usb.add_css_class("success");
    lbl_usb.set_valign(gtk::Align::Center);
    row_usb.add_suffix(&lbl_usb);
    g_ob.add(&row_usb);
    mouse_page.add(&g_ob);

    // ── Mouse LED Control (Primary Zone - Solaar HID++ 4.2) ──
    let m_led_st = Rc::new(RefCell::new(read_mouse_led_conf()));
    let g_m_led = adw::PreferencesGroup::builder()
        .title("LED Lighting (Primary Zone)")
        .description("RGB indicator LED effects and colors via Logitech HID++ 4.2")
        .build();

    let row_m_led_prev = adw::ActionRow::builder().title("Active Mouse LED Status").build();
    let m_led_swatch = gtk::DrawingArea::new();
    m_led_swatch.set_content_width(52);
    m_led_swatch.set_content_height(28);
    m_led_swatch.set_valign(gtk::Align::Center);
    {
        let st = m_led_st.clone();
        m_led_swatch.set_draw_func(move |_a, cr, w, h| {
            let s = st.borrow();
            if s.mode == 0 {
                draw_swatch(cr, w as f64, h as f64, 30, 30, 30);
            } else {
                draw_swatch(cr, w as f64, h as f64, s.r, s.g, s.b);
            }
        });
    }
    row_m_led_prev.add_suffix(&m_led_swatch);
    g_m_led.add(&row_m_led_prev);

    // Apply helper for mouse LED
    let apply_m_led: Rc<dyn Fn()> = {
        let st = m_led_st.clone();
        Rc::new(move || {
            let s = st.borrow();
            let hex_color = format!("0x{:02x}{:02x}{:02x}", s.r, s.g, s.b);
            run_user(vec![
                script_path("battery-mouse-logitech.sh"),
                "led".into(),
                s.mode.to_string(),
                hex_color,
                s.period_ms.to_string(),
                s.intensity.to_string(),
            ]);
        })
    };
    let refresh_m_led_prev: Rc<dyn Fn()> = {
        let st = m_led_st.clone();
        let row = row_m_led_prev.clone();
        let pv = m_led_swatch.clone();
        Rc::new(move || {
            let s = st.borrow();
            row.set_subtitle(&format!(
                "Mode: {} • Hex: #{:02X}{:02X}{:02X} • Brightness: {}% • Speed: {:.1}s",
                mouse_led_mode_name(s.mode), s.r, s.g, s.b, s.intensity, s.period_ms as f64 / 1000.0
            ));
            pv.queue_draw();
        })
    };
    let m_led_deb: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    let schedule_m_led: Rc<dyn Fn()> = {
        let apply = apply_m_led.clone();
        let deb = m_led_deb.clone();
        Rc::new(move || {
            if let Some(id) = deb.take() {
                id.remove();
            }
            let apply2 = apply.clone();
            let deb2 = deb.clone();
            let id = glib::timeout_add_local(Duration::from_millis(80), move || {
                apply2();
                deb2.set(None);
                glib::ControlFlow::Break
            });
            deb.set(Some(id));
        })
    };

    // LED Mode buttons
    let row_m_led_mode = adw::ActionRow::builder()
        .title("Lighting Effect")
        .subtitle("Select pattern: Off, Static, Cycle, Breathe")
        .build();
    let box_m_led_mode = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    box_m_led_mode.set_valign(gtk::Align::Center);
    let mut m_led_btns: Vec<(u32, gtk::Button)> = Vec::new();
    let m_mode_choices = [
        (0u32, "Off"),
        (1u32, "Static"),
        (3u32, "Cycle"),
        (10u32, "Breathe"),
    ];
    for (mode_id, mode_lbl) in m_mode_choices {
        let b = seg_button(mode_lbl);
        let st = m_led_st.clone();
        let sch = schedule_m_led.clone();
        let ref_prev = refresh_m_led_prev.clone();
        let pend_m = m_pending_led_mode.clone();
        b.connect_clicked(move |_| {
            st.borrow_mut().mode = mode_id;
            pend_m.set(Some((mode_id, Instant::now())));
            ref_prev();
            sch();
        });
        box_m_led_mode.append(&b);
        m_led_btns.push((mode_id, b));
    }
    row_m_led_mode.add_suffix(&box_m_led_mode);
    g_m_led.add(&row_m_led_mode);

    // Color Swatches Palette for Mouse LED
    let card_m_swatches = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    card_m_swatches.set_halign(gtk::Align::Center);
    card_m_swatches.set_valign(gtk::Align::Center);
    card_m_swatches.set_margin_top(8);
    card_m_swatches.set_margin_bottom(8);

    let m_palette = [
        ("Cyan", 0u8, 200u8, 255u8),
        ("Blue", 0, 100, 255),
        ("Purple", 160, 0, 255),
        ("Magenta", 255, 0, 127),
        ("Red", 255, 0, 0),
        ("Orange", 255, 120, 0),
        ("Yellow", 255, 216, 0),
        ("Green", 0, 230, 118),
        ("White", 255, 255, 255),
    ];

    for (cname, pr, pg, pb) in m_palette {
        let btn = gtk::Button::new();
        btn.add_css_class("flat");
        btn.set_tooltip_text(Some(&format!("{cname} (#{pr:02X}{pg:02X}{pb:02X})")));
        let circle = gtk::DrawingArea::new();
        circle.set_content_width(30);
        circle.set_content_height(30);
        circle.set_draw_func(move |_a, cr, w, h| {
            draw_circle(cr, w as f64, h as f64, pr, pg, pb);
        });
        btn.set_child(Some(&circle));
        let st = m_led_st.clone();
        let sch = schedule_m_led.clone();
        let ref_prev = refresh_m_led_prev.clone();
        let pend_c = m_pending_led_color.clone();
        let pend_m = m_pending_led_mode.clone();
        btn.connect_clicked(move |_| {
            {
                let mut s = st.borrow_mut();
                s.r = pr;
                s.g = pg;
                s.b = pb;
                if s.mode == 0 {
                    s.mode = 1;
                    pend_m.set(Some((1, Instant::now())));
                }
            }
            pend_c.set(Some(((pr, pg, pb), Instant::now())));
            ref_prev();
            sch();
        });
        card_m_swatches.append(&btn);
    }
    let row_m_palette = adw::PreferencesRow::new();
    row_m_palette.set_child(Some(&card_m_swatches));
    g_m_led.add(&row_m_palette);

    // Color Dialog Button
    let row_m_color = adw::ActionRow::builder()
        .title("Custom Color (Spectrum Picker)")
        .subtitle("Select custom 24-bit RGB color for mouse LED")
        .build();
    let m_color_dialog = gtk::ColorDialog::new();
    m_color_dialog.set_with_alpha(false);
    let m_color_btn = gtk::ColorDialogButton::new(Some(m_color_dialog));
    m_color_btn.set_valign(gtk::Align::Center);
    {
        let s = m_led_st.borrow();
        m_color_btn.set_rgba(&gtk::gdk::RGBA::new(s.r as f32 / 255.0, s.g as f32 / 255.0, s.b as f32 / 255.0, 1.0));
    }
    {
        let st = m_led_st.clone();
        let sch = schedule_m_led.clone();
        let ref_prev = refresh_m_led_prev.clone();
        let pend_c = m_pending_led_color.clone();
        let pend_m = m_pending_led_mode.clone();
        m_color_btn.connect_rgba_notify(move |btn| {
            let rgba = btn.rgba();
            let cr = (rgba.red() * 255.0).round() as u8;
            let cg = (rgba.green() * 255.0).round() as u8;
            let cb = (rgba.blue() * 255.0).round() as u8;
            {
                let mut s = st.borrow_mut();
                s.r = cr;
                s.g = cg;
                s.b = cb;
                if s.mode == 0 {
                    s.mode = 1;
                    pend_m.set(Some((1, Instant::now())));
                }
            }
            pend_c.set(Some(((cr, cg, cb), Instant::now())));
            ref_prev();
            sch();
        });
    }
    row_m_color.add_suffix(&m_color_btn);
    g_m_led.add(&row_m_color);

    // Brightness / Intensity Slider
    let row_m_led_bright = adw::ActionRow::builder().title("LED Brightness").subtitle("100%").build();
    let scale_m_led_bright = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 5.0);
    scale_m_led_bright.set_size_request(180, -1);
    scale_m_led_bright.set_valign(gtk::Align::Center);
    {
        let s = m_led_st.borrow();
        scale_m_led_bright.set_value(s.intensity as f64);
    }
    {
        let st = m_led_st.clone();
        let sch = schedule_m_led.clone();
        let ref_prev = refresh_m_led_prev.clone();
        let row_b = row_m_led_bright.clone();
        let sync_guard = m_sync.clone();
        scale_m_led_bright.connect_value_changed(move |sc| {
            if sync_guard.get() {
                return;
            }
            let int_val = sc.value().round() as u32;
            row_b.set_subtitle(&format!("{}%", int_val));
            st.borrow_mut().intensity = int_val;
            ref_prev();
            sch();
        });
    }
    row_m_led_bright.add_suffix(&scale_m_led_bright);
    g_m_led.add(&row_m_led_bright);

    // Animation Speed / Period Slider (for Cycle and Breathe)
    let row_m_led_speed = adw::ActionRow::builder().title("Animation Speed (Period)").subtitle("3.0s (3000 ms)").build();
    let scale_m_led_speed = gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 10.0, 0.5);
    scale_m_led_speed.set_size_request(180, -1);
    scale_m_led_speed.set_valign(gtk::Align::Center);
    {
        let s = m_led_st.borrow();
        scale_m_led_speed.set_value(s.period_ms as f64 / 1000.0);
    }
    {
        let st = m_led_st.clone();
        let sch = schedule_m_led.clone();
        let ref_prev = refresh_m_led_prev.clone();
        let row_s = row_m_led_speed.clone();
        let sync_guard = m_sync.clone();
        scale_m_led_speed.connect_value_changed(move |sc| {
            if sync_guard.get() {
                return;
            }
            let sec_val = sc.value();
            let ms_val = (sec_val * 1000.0).round() as u32;
            row_s.set_subtitle(&format!("{:.1}s ({} ms)", sec_val, ms_val));
            st.borrow_mut().period_ms = ms_val;
            ref_prev();
            sch();
        });
    }
    row_m_led_speed.add_suffix(&scale_m_led_speed);
    g_m_led.add(&row_m_led_speed);

    mouse_page.add(&g_m_led);

    let mp = stack.add_titled(&mouse_page, Some("mouse"), "Mouse Logitech");
    mp.set_icon_name(Some("input-mouse-symbolic"));

    // ── Layanan Sistem page ──
    let services_page = adw::PreferencesPage::new();
    let mut svc_widgets: Vec<SvcW> = Vec::new();
    services_page.add(&build_svc_group(
        &shared,
        &window,
        &USER_SVC,
        true,
        "AI &amp; User Services (User Daemons)",
        &mut svc_widgets,
    ));
    services_page.add(&build_svc_group(
        &shared,
        &window,
        &SYS_SVC,
        false,
        "System Infrastructure Services (Root)",
        &mut svc_widgets,
    ));
    let svp = stack.add_titled(&services_page, Some("services"), "System Services");
    svp.set_icon_name(Some("emblem-system-symbolic"));

    // ── Left sidebar (adaptive AdwNavigationSplitView, Mission Center style) ──
    let sidebar = gtk::ListBox::new();
    sidebar.add_css_class("navigation-sidebar");
    sidebar.set_selection_mode(gtk::SelectionMode::Single);
    sidebar.append(&sidebar_row("cpu", "CPU", "lucide-cpu"));
    sidebar.append(&sidebar_row("memory", "Memory", "lucide-memory-stick"));
    sidebar.append(&sidebar_row("speedtest", "Speed Test", "lucide-gauge"));
    sidebar.append(&sidebar_row("power", "Power & Battery", "lucide-battery-charging"));
    sidebar.append(&sidebar_row("rgb", "Keyboard RGB", "lucide-keyboard"));
    sidebar.append(&sidebar_row("mouse", "Mouse Logitech", "lucide-mouse"));
    sidebar.append(&sidebar_row("services", "System Services", "lucide-server"));
    sidebar.append(&sidebar_row("apps", "Applications & Processes", "lucide-apps"));
    sidebar.append(&sidebar_row("svcall", "All Services", "lucide-server"));
    sidebar.append(&sidebar_row("ssh", "SSH Manager", "lucide-terminal"));
    sidebar.append(&sidebar_row("logs", "System Logs", "lucide-scroll-text"));

    // Section headers rendered above rows via the header func — this adds no
    // selectable rows, so the absolute index math in rebuild_dynamic (which
    // inserts the live monitor tabs after Memory) stays correct.
    sidebar.set_header_func(|row, before| {
        let section = sidebar_section(row.widget_name().as_str());
        let prev = before.map(|b| sidebar_section(b.widget_name().as_str()));
        if prev.as_deref() != Some(section) {
            let lbl = gtk::Label::new(Some(section));
            lbl.add_css_class("dimmed");
            lbl.add_css_class("caption-heading");
            lbl.set_xalign(0.0);
            lbl.set_margin_start(12);
            lbl.set_margin_end(12);
            lbl.set_margin_top(if before.is_none() { 10 } else { 16 });
            lbl.set_margin_bottom(4);
            row.set_header(Some(&lbl));
        } else {
            row.set_header(gtk::Widget::NONE);
        }
    });

    let side_scroll = gtk::ScrolledWindow::new();
    side_scroll.add_css_class("side-scroll");
    side_scroll.set_child(Some(&sidebar));
    side_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    side_scroll.set_vexpand(true);

    // Sidebar pane — its own header bar carries the app title.
    let side_tv = adw::ToolbarView::new();
    side_tv.add_css_class("side-tv");
    let side_header = adw::HeaderBar::new();
    side_header.add_css_class("side-header");
    side_header.set_title_widget(Some(&adw::WindowTitle::new("Tweaks ASUS TUF", "ASUS TUF Gaming")));
    side_tv.add_top_bar(&side_header);
    side_tv.set_content(Some(&side_scroll));

    // Content pane — header title tracks the selected page.
    stack.set_hexpand(true);
    stack.set_hhomogeneous(true);
    stack.set_vhomogeneous(false);
    let content_tv = adw::ToolbarView::new();
    let content_header = adw::HeaderBar::new();
    let content_title = adw::WindowTitle::new("CPU", "");
    content_header.set_title_widget(Some(&content_title));

    // Option: pause heavy sampling (process table / full unit list) for tabs that
    // are not currently visible. On by default to keep the footprint small.
    let pause_toggle = gtk::ToggleButton::new();
    pause_toggle.set_icon_name("media-playback-pause-symbolic");
    pause_toggle.set_active(true);
    pause_toggle.set_tooltip_text(Some("Pause sampling for hidden tabs (saves memory & CPU)"));
    content_header.pack_end(&pause_toggle);
    {
        let sh = shared.clone();
        pause_toggle.connect_toggled(move |b| {
            if let Ok(mut g) = sh.lock() {
                g.pause_hidden = b.is_active();
                if !b.is_active() {
                    // Resuming: refill hidden tabs on the next tick.
                    g.force_heavy = true;
                }
            }
        });
    }

    // Temperature alerts toggle — bell icon, default ON.
    let alert_toggle = gtk::ToggleButton::new();
    alert_toggle.set_active(true);
    alert_toggle.set_tooltip_text(Some("Temperature alerts (notify when CPU/GPU/Disk overheat)"));
    // Use lucide-bell SVG as a child image widget.
    let bell_img = lucide("lucide-bell", 16);
    alert_toggle.set_child(Some(&bell_img));
    content_header.pack_end(&alert_toggle);
    {
        let sh = shared.clone();
        alert_toggle.connect_toggled(move |b| {
            if let Ok(mut g) = sh.lock() {
                g.alerts_enabled = b.is_active();
            }
        });
    }

    content_tv.add_top_bar(&content_header);
    content_tv.set_content(Some(&stack));

    // Fixed side-by-side split view.
    let split = adw::OverlaySplitView::new();
    split.set_sidebar(Some(&side_tv));
    split.set_content(Some(&content_tv));
    split.set_min_sidebar_width(210.0);
    split.set_max_sidebar_width(260.0);
    split.set_collapsed(false);

    {
        let stack = stack.clone();
        let content_title = content_title.clone();
        let sh = shared.clone();
        sidebar.connect_row_selected(move |_lb, row| {
            if let Some(r) = row {
                let name = r.widget_name();
                stack.set_visible_child_name(name.as_str());
                if let Some(child) = stack.child_by_name(name.as_str()) {
                    let title = stack.page(&child).title().unwrap_or_default();
                    content_title.set_title(title.as_str());
                }
                // Tell the sampler which tab is live and sample it right away.
                if let Ok(mut g) = sh.lock() {
                    g.visible_tab = name.as_str().to_string();
                    g.force_heavy = true;
                }
            }
        });
    }
    if let Some(first) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&first));
    }

    window.set_content(Some(&split));

    // threshold switch handler (guarded against programmatic set)
    let sync = Rc::new(Cell::new(false));
    {
        let sync = sync.clone();
        switch_threshold.connect_active_notify(move |s| {
            if sync.get() {
                return;
            }
            let val = if s.is_active() { "80" } else { "100" };
            run_priv(vec![script_path("battery-set-threshold.sh"), val.to_string()]);
        });
    }

    // CSS
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        "@define-color accent_bg_color #e0e0e0; @define-color accent_fg_color #000000; \
         @define-color accent_color #ffffff; \
         window, .background, headerbar, .view, viewswitcher, stackswitcher, \
         scrolledwindow, textview, textview text, preferencespage, clamp, viewport { \
         background-color: #000000; } \
         headerbar { box-shadow: none; border-bottom: 1px solid rgba(255,255,255,0.06); } \
         .side-header, .side-tv, .side-scroll, .navigation-sidebar { background-color: #000000; background: #000000; } \
         .svc-run { color: #ffffff; } .svc-fail { color: #9a9a9a; } .svc-idle { color: #5e5c64; } \
         scrolledwindow, scrolledwindow > viewport, .navigation-sidebar { border: none; box-shadow: none; } \
         .navigation-sidebar { background-color: #000000; } \
         .navigation-sidebar > row { background-color: transparent; border-radius: 8px; margin: 2px 8px; padding: 2px; } \
         .navigation-sidebar > row:hover:not(:selected) { background-color: #141414; } \
         .navigation-sidebar > row:selected { background-color: #1c1c1c; color: #ffffff; } \
         .boxed-list, .card { background-color: #0a0a0a; border: 1px solid rgba(255,255,255,0.08); border-radius: 12px; } \
         .boxed-list > row, .card > row { background-color: transparent; } \
         .linked > togglebutton:checked { background-color: #2a2a2a; color: #ffffff; } \
         levelbar trough { background-color: #1a1a1a; border: none; } \
         levelbar block { background-color: #e0e0e0; border: none; } \
         levelbar block.low, levelbar block.high, levelbar block.full { background-color: #e0e0e0; } \
         button.suggested-action { background-color: #e6e6e6; color: #000000; box-shadow: none; } \
         button.suggested-action:hover { background-color: #f2f2f2; color: #000000; } \
         button.destructive-action { background-color: #333333; color: #ffffff; box-shadow: none; } \
         button.destructive-action:hover { background-color: #444444; color: #ffffff; } \
         .cpu-graph-frame { border: 1px solid rgba(255,255,255,0.08); border-radius: 8px; \
         background-color: #0a0a0a; } \
         scale.red-slider highlight { background: #ff3b30; } \
         scale.green-slider highlight { background: #34c759; } \
         scale.blue-slider highlight { background: #007aff; } \
         .badge-run { background-color: #e6e6e6; color: #000; border-radius: 6px; padding: 2px 10px; font-weight: bold; } \
         .badge-stop { background-color: #3a3a3a; color: #cfcfcf; border-radius: 6px; padding: 2px 10px; font-weight: bold; } \
         .badge-fail { background-color: #8a8a8a; color: #000; border-radius: 6px; padding: 2px 10px; font-weight: bold; } \
         .svc-dot { font-size: 14px; margin-right: 4px; } \
         .svc-dot.dot-run { color: #2ec27e; } \
         .svc-dot.dot-fail { color: #e01b24; } \
         .svc-dot.dot-stop { color: #5e5c64; }",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }

    let ui = Rc::new(Ui {
        row_model,
        util_bar,
        core_areas,
        temp_area,
        l: labels,
        row_bat,
        bat_bar,
        row_drain,
        row_health,
        row_gpu_tel,
        row_gpu_mode,
        btn_gpu,
        btn_mode,
        row_fan_rpm,
        row_fan_ctrl,
        btn_fan,
        switch_threshold,
        row_cpu_mon,
        sync,
        row_m_bat,
        m_bat_bar,
        row_m_hz,
        hz_btns,
        row_m_dpi,
        scale_dpi,
        switch_onboard,
        m_sync,
        m_pending_dpi,
        m_pending_hz,
        m_pending_led_mode,
        m_pending_led_color,
        row_m_led_prev,
        m_led_swatch,
        m_led_btns,
        row_m_led_bright,
        scale_m_led_bright,
        row_m_led_speed,
        scale_m_led_speed,
        services: svc_widgets,
        row_mem,
        mem_bar,
        mem_area,
        swap_area,
        mem_lbl,
        apps: apps_ui,
        svc_all: svc_all_ui,
        drives: RefCell::new(Vec::new()),
        gpus: RefCell::new(Vec::new()),
        fans: RefCell::new(Vec::new()),
        nets: RefCell::new(Vec::new()),
        bats: RefCell::new(Vec::new()),
        shared: shared.clone(),
        stack: stack.clone(),
        sidebar: sidebar.clone(),
        dyn_rows: RefCell::new(Vec::new()),
        dyn_pages: RefCell::new(Vec::new()),
        dyn_sig: RefCell::new(String::new()),
    });

    // Build the dynamic tabs (GPUs + fans + nets + drives + bats) now so they appear at startup.
    {
        let (gs, fs_, ns, ds, bs) = shared
            .lock()
            .map(|g| (g.gpus.clone(), g.fans.clone(), g.nets.clone(), g.drives.clone(), g.bats.clone()))
            .unwrap_or_default();
        ui.rebuild_dynamic(&gs, &fs_, &ns, &ds, &bs);
        *ui.dyn_sig.borrow_mut() = format!(
            "{}#{}#{}#{}#{}",
            gpu_signature(&gs),
            fan_signature(&fs_),
            net_signature(&ns),
            drive_signature(&ds),
            bat_signature(&bs)
        );
    }

    let sh = shared.clone();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        if let Ok(g) = sh.lock() {
            ui.refresh(&g);
        }
        glib::ControlFlow::Continue
    });

    window.present();
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id("com.rezkycodes.AsusTufCpu")
        .build();
    app.connect_activate(build_ui);
    app.run()
}
