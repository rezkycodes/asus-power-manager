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
    // ── Systemd services (key "user:unit"/"sys:unit" -> state) ──
    services: std::collections::HashMap<String, String>,
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
    let (mut sockets, mut virt, mut vm) = ("1".to_string(), "—".to_string(), "Tidak".to_string());
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
                    "Hypervisor vendor" => vm = "Ya".to_string(),
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

fn spawn_sampler(sh: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || {
        let logical = sh.lock().map(|g| g.logical).unwrap_or(1);
        gather_static(&sh, logical);
        let temp_path = find_temp_path();
        let mut prev: Option<((u64, u64), Vec<(u64, u64)>)> = None;
        let mut tick: u64 = 0;
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
                        .map(|b| if b == "1" { "Aktif" } else { "Nonaktif" })
                        .unwrap_or("Nonaktif")
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
                        format!("Suhu: {} • Daya: {} • VRAM: {} • P-State: {} ({})", t, p, v, ps, pci_status)
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
            if tick % 3 == 0 {
                refresh_drive_list(&sh);
                refresh_part_usage(&sh);
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
    ("battery-charge-threshold.service", "Batas Baterai 80% Service", "Proteksi hardware"),
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
    cr.set_source_rgb(0.16, 0.55, 0.96);
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
    cr.set_source_rgba(0.16, 0.55, 0.96, 0.22);
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
    services: Vec<SvcW>,
    // memory
    row_mem: adw::ActionRow,
    mem_bar: gtk::LevelBar,
    mem_area: gtk::DrawingArea,
    swap_area: gtk::DrawingArea,
    mem_lbl: std::collections::HashMap<&'static str, gtk::Label>,
    drives: RefCell<Vec<DriveUi>>,
    // hotplug rebuild handles
    shared: Arc<Mutex<Shared>>,
    stack: adw::ViewStack,
    sidebar: gtk::ListBox,
    drive_rows: RefCell<Vec<gtk::ListBoxRow>>,
    drive_pages: RefCell<Vec<adw::PreferencesPage>>,
    drive_sig: RefCell<String>,
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
            "Utilisasi: {}% • Kecepatan: {} GHz",
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
        self.row_bat.set_title(&format!("Baterai: {}%", g.bat_cap));
        if g.ac_online {
            if g.threshold == "80" && pct_num >= 79.0 {
                self.row_bat.set_subtitle("🔌 Tersambung Charger — Siaga (Batas 80%)");
                self.row_drain.set_subtitle("Daya dari Adaptor (Baterai standby)");
            } else if g.bat_status.eq_ignore_ascii_case("charging") {
                let tgt = if g.threshold != "100" { format!(" (Target {}%)", g.threshold) } else { String::new() };
                self.row_bat.set_subtitle(&format!("⚡ Mengisi Daya{}", tgt));
                self.row_drain.set_subtitle(&format!("{} (Pengisian)", if g.energy_rate.is_empty() { "Aktif" } else { &g.energy_rate }));
            } else {
                self.row_bat.set_subtitle(&format!("🔌 Tersambung Charger ({})", g.bat_status));
                self.row_drain.set_subtitle("Adaptor Aktif");
            }
        } else {
            self.row_bat.set_subtitle("🔋 Mode Baterai (Tidak Dicas)");
            self.row_drain.set_subtitle(&format!("{} (Konsumsi Beban)", if g.energy_rate.is_empty() { "Aktif" } else { &g.energy_rate }));
        }
        let est = if !g.time_str.is_empty() && !g.ac_online { format!(" | Estimasi: {}", g.time_str) } else { String::new() };
        self.row_health.set_subtitle(&format!("Kesehatan Sel: {}{}", if g.health_cap.is_empty() { "Normal" } else { &g.health_cap }, est));

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
                self.row_gpu_mode.set_subtitle("Mode: NVIDIA Dedicated (Performa penuh)");
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
            "2" => (0, "Silent / Hening (Kecepatan rendah)"),
            "1" => (2, "Turbo / Overboost (Pendinginan cepat)"),
            _ => (1, "Normal / Balanced (Otomatis)"),
        };
        Self::set_active(&self.btn_fan[fi], true);
        self.row_fan_ctrl.set_subtitle(&format!("Status Aktif: {}", flabel));

        // CPU monitor
        self.row_cpu_mon.set_subtitle(&format!(
            "Governor: {} @ {} MHz | Turbo: {} | Profil: {}",
            g.governor.to_uppercase(),
            g.freq_mhz,
            g.boost,
            g.profile
        ));

        // ── Mouse ──
        let mbat: f64 = g.m_bat.parse().unwrap_or(90.0);
        self.m_bat_bar.set_value(mbat);
        self.row_m_bat.set_title(&format!("Baterai Mouse G304: {}%", g.m_bat));
        let mstat = if g.m_status.eq_ignore_ascii_case("discharging")
            || g.m_status.eq_ignore_ascii_case("charging")
            || g.m_status.eq_ignore_ascii_case("full")
        {
            "Tersambung (Aktif)"
        } else {
            "Standby / Tidur"
        };
        self.row_m_bat.set_subtitle(&format!("Status: {} • Koneksi: Lightspeed Receiver", mstat));

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
            "Aktif: {} Hz ({})",
            hz,
            if hz == 1000 { "1ms Peak" } else { "Hemat Baterai" }
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

        // ── Services ──
        for s in &self.services {
            let state = match g.services.get(&s.key) {
                Some(v) => v.as_str(),
                None => continue,
            };
            s.badge.remove_css_class("badge-run");
            s.badge.remove_css_class("badge-stop");
            s.badge.remove_css_class("badge-fail");
            s.toggle.remove_css_class("destructive-action");
            s.toggle.remove_css_class("suggested-action");
            match state {
                "active" => {
                    s.badge.set_text("Aktif");
                    s.badge.add_css_class("badge-run");
                    s.toggle.set_label("Stop");
                    s.toggle.add_css_class("destructive-action");
                }
                "failed" => {
                    s.badge.set_text("Gagal");
                    s.badge.add_css_class("badge-fail");
                    s.toggle.set_label("Start");
                    s.toggle.add_css_class("suggested-action");
                }
                "unknown" => {
                    s.badge.set_text("Unknown");
                    s.badge.add_css_class("badge-stop");
                    s.toggle.set_label("Start");
                }
                _ => {
                    s.badge.set_text("Mati");
                    s.badge.add_css_class("badge-stop");
                    s.toggle.set_label("Start");
                    s.toggle.add_css_class("suggested-action");
                }
            }
        }

        // ── Memory ──
        self.row_mem.set_title(&format!("Memori: {} total", fmt_gib(g.mem_total)));
        self.row_mem.set_subtitle(&format!("Terpakai {} • {}%", fmt_gib(g.mem_used), g.mem_pct));
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

        // ── Drives (rebuild sidebar/pages on hotplug, then update values) ──
        let sig = drive_signature(&g.drives);
        if *self.drive_sig.borrow() != sig {
            self.rebuild_drives(&g.drives);
            *self.drive_sig.borrow_mut() = sig;
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
                du.active_area.queue_draw();
                du.thru_area.queue_draw();
            }
        }
    }

    // Rebuild the per-drive sidebar rows and stack pages after a hotplug change.
    // Drive rows live between Memory (index 1) and Power, i.e. starting at index 2.
    fn rebuild_drives(&self, drives: &[DriveInfo]) {
        for r in self.drive_rows.borrow().iter() {
            self.sidebar.remove(r);
        }
        self.drive_rows.borrow_mut().clear();
        for p in self.drive_pages.borrow().iter() {
            self.stack.remove(p);
        }
        self.drive_pages.borrow_mut().clear();
        self.drives.borrow_mut().clear();

        for (i, info) in drives.iter().enumerate() {
            let (page, du) = build_drive_page(&self.shared, i, info);
            self.stack.add_titled(&page, Some(&format!("drive{i}")), &format!("Drive {i}"));
            let row = sidebar_row(
                &format!("drive{i}"),
                &format!("{} {} ({})", info.kind, i, info.dev),
                "lucide-hard-drive",
            );
            self.sidebar.insert(&row, (2 + i) as i32);
            self.drive_rows.borrow_mut().push(row);
            self.drive_pages.borrow_mut().push(page);
            self.drives.borrow_mut().push(du);
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

    let g_head = adw::PreferencesGroup::builder().title("Prosesor").build();
    let row_model = adw::ActionRow::builder()
        .title("Memuat model CPU...")
        .subtitle("Utilisasi: --% • Kecepatan: -- GHz")
        .build();
    let util_bar = gtk::LevelBar::builder().min_value(0.0).max_value(100.0).valign(gtk::Align::Center).build();
    util_bar.set_size_request(110, 16);
    row_model.add_suffix(&util_bar);
    g_head.add(&row_model);
    page.add(&g_head);

    let g_cores = adw::PreferencesGroup::builder()
        .title("Utilisasi per-Core (1 menit)")
        .description("Grafik realtime penggunaan tiap core logis (0–100%)")
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

    let g_temp = adw::PreferencesGroup::builder().title("Suhu CPU (1 menit)").build();
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

    let g_info = adw::PreferencesGroup::builder().title("Informasi Detail").build();
    let mut l = std::collections::HashMap::new();
    l.insert("speed", info_row("Kecepatan Saat Ini", &g_info));
    l.insert("base", info_row("Base Speed", &g_info));
    l.insert("logical", info_row("Prosesor Logis", &g_info));
    l.insert("sockets", info_row("Socket", &g_info));
    l.insert("virt", info_row("Virtualisasi", &g_info));
    l.insert("vm", info_row("Virtual Machine", &g_info));
    l.insert("l1", info_row("Cache L1 (data / instruksi)", &g_info));
    l.insert("l2", info_row("Cache L2", &g_info));
    l.insert("l3", info_row("Cache L3", &g_info));
    l.insert("driver", info_row("Cpufreq Driver", &g_info));
    l.insert("gov", info_row("Cpufreq Governor", &g_info));
    l.insert("pth", info_row("Proses / Thread / Handle", &g_info));
    l.insert("uptime", info_row("Uptime Sistem", &g_info));
    l.insert("temp", info_row("Suhu CPU", &g_info));
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

fn build_rgb_page() -> adw::PreferencesPage {
    let st = Rc::new(RefCell::new(read_rgb_conf()));
    let guard = Rc::new(Cell::new(false));
    let debounce: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    let page = adw::PreferencesPage::new();

    // Preview
    let g_prev = adw::PreferencesGroup::builder().title("Status Warna &amp; Efek Aktif").build();
    let row_prev = adw::ActionRow::builder().title("Warna Keyboard Saat Ini").build();
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
    let g_pick = adw::PreferencesGroup::builder().title("Pilih Warna Bebas (Color Wheel)").build();
    let row_pick = adw::ActionRow::builder().title("Dialog Spektrum Warna").subtitle("Buka pemilih warna GNOME").build();
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
    let g_sl = adw::PreferencesGroup::builder().title("Penyesuaian Manual (Slider RGB)").build();
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
        .title("Palet Warna Cepat (Preset)")
        .description("Klik warna untuk menerapkannya seketika")
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
    let g_eff = adw::PreferencesGroup::builder().title("Efek Animasi (Aura Lighting)").build();
    let effects = [
        ("0", "Static (Warna Tetap)"),
        ("1", "Breathing (Pernapasan)"),
        ("10", "Pulse (Denyut)"),
        ("2", "Color Cycle (Rainbow)"),
        ("3", "Strobing (Berkedip)"),
    ];
    let eff_btns: Rc<Vec<(String, gtk::Button)>> = Rc::new(
        effects.iter().map(|(id, _)| (id.to_string(), gtk::Button::with_label("Pilih"))).collect(),
    );
    let highlight_eff: Rc<dyn Fn(&str)> = {
        let eb = eff_btns.clone();
        Rc::new(move |active: &str| {
            for (id, b) in eb.iter() {
                if id == active {
                    b.add_css_class("suggested-action");
                    b.set_label("✓ Aktif");
                } else {
                    b.remove_css_class("suggested-action");
                    b.set_label("Pilih");
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
    let g_bs = adw::PreferencesGroup::builder().title("Kecerahan &amp; Kecepatan Efek").build();
    // brightness
    let row_b = adw::ActionRow::builder().title("Kecerahan Lampu Keyboard").build();
    let box_b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    box_b.set_valign(gtk::Align::Center);
    let bri_btns: Rc<Vec<(u8, gtk::Button)>> = Rc::new(
        [(0u8, "Mati"), (1, "Redup"), (2, "Sedang"), (3, "Terang")]
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
    let row_s = adw::ActionRow::builder().title("Kecepatan Animasi Efek").build();
    let box_s = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    box_s.set_valign(gtk::Align::Center);
    let spd_btns: Rc<Vec<(String, gtk::Button)>> = Rc::new(
        [("0", "Lambat"), ("1", "Sedang"), ("2", "Cepat")]
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
                "(kosong)".into()
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
                "(belum ada catatan log)".into()
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
    win.set_title(Some(&format!("Detail — {unit}")));
    win.set_default_size(620, 740);
    win.set_modal(true);
    win.set_transient_for(Some(parent));

    let tv_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let refresh_btn = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_tooltip_text(Some("Segarkan"));
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
        if is_user { "Layanan Pengguna (--user)" } else { "Layanan Sistem (root)" }
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
        let bx = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bx.set_valign(gtk::Align::Center);
        let badge = gtk::Label::new(Some("Memuat"));
        badge.add_css_class("badge-stop");
        let toggle = seg_button("Mulai");
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
        let detail = seg_button("Detail");
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

    let g_head = adw::PreferencesGroup::builder().title("Memori Sistem").build();
    let row_mem = adw::ActionRow::builder().title("Memori").subtitle("Memuat...").build();
    let mem_bar = gtk::LevelBar::builder().min_value(0.0).max_value(100.0).valign(gtk::Align::Center).build();
    mem_bar.set_size_request(110, 16);
    row_mem.add_suffix(&mem_bar);
    g_head.add(&row_mem);
    page.add(&g_head);

    let g_mem = adw::PreferencesGroup::builder().title("Penggunaan Memori (1 menit)").build();
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

    let g_swap = adw::PreferencesGroup::builder().title("Swap (1 menit)").build();
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

    let g_det = adw::PreferencesGroup::builder().title("Rincian").build();
    let mut mem_lbl = std::collections::HashMap::new();
    mem_lbl.insert("used", info_row("Sedang Dipakai (In Use)", &g_det));
    mem_lbl.insert("avail", info_row("Tersedia (Available)", &g_det));
    mem_lbl.insert("committed", info_row("Committed", &g_det));
    mem_lbl.insert("cached", info_row("Cached", &g_det));
    mem_lbl.insert("swapused", info_row("Swap Terpakai", &g_det));
    mem_lbl.insert("swapavail", info_row("Swap Tersedia", &g_det));
    page.add(&g_det);

    let g_hw = adw::PreferencesGroup::builder()
        .title("Perangkat Keras (DIMM)")
        .build();
    mem_lbl.insert("dtype", info_row("Tipe", &g_hw));
    mem_lbl.insert("dform", info_row("Form Factor", &g_hw));
    mem_lbl.insert("dspeed", info_row("Kecepatan", &g_hw));
    mem_lbl.insert("dslots", info_row("Slot Terpakai", &g_hw));
    page.add(&g_hw);

    (page, row_mem, mem_bar, mem_area, swap_area, mem_lbl)
}

struct DriveUi {
    idx: usize,
    active_area: gtk::DrawingArea,
    thru_area: gtk::DrawingArea,
    thru_scale: gtk::Label,
    lbl: std::collections::HashMap<&'static str, gtk::Label>,
    parts: Vec<(gtk::LevelBar, gtk::Label)>,
}

// Build one drive detail page (mirrors Mission Center's Disk view layout using
// this app's card style: Active-time + Throughput graphs, stats, details,
// partitions with usage bars).
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

    let g_active = adw::PreferencesGroup::builder().title("Waktu Aktif (1 menit)").build();
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

    let g_thru = adw::PreferencesGroup::builder().title("Throughput (1 menit)").build();
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
    let g_stat = adw::PreferencesGroup::builder().title("Statistik").build();
    lbl.insert("rspeed", info_row("Kecepatan Baca", &g_stat));
    lbl.insert("wspeed", info_row("Kecepatan Tulis", &g_stat));
    lbl.insert("tread", info_row("Total Dibaca", &g_stat));
    lbl.insert("twrite", info_row("Total Ditulis", &g_stat));
    lbl.insert("active", info_row("Waktu Aktif", &g_stat));
    lbl.insert("resp", info_row("Rata-rata Respons", &g_stat));
    page.add(&g_stat);

    let g_det = adw::PreferencesGroup::builder().title("Detail").build();
    let det = |t: &str, v: &str, gr: &adw::PreferencesGroup| {
        let l = info_row(t, gr);
        // Long IDs (e.g. WWN) must not steal the row width and wrap the title.
        // Middle-ellipsize like Mission Center; selectable keeps the full value.
        l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        l.set_max_width_chars(28);
        l.set_selectable(true);
        l.set_text(v);
    };
    det("Kapasitas", &fmt_bytes(info.capacity), &g_det);
    det("Terformat", &fmt_bytes(info.formatted), &g_det);
    det("Disk Sistem", if info.is_system { "Ya" } else { "Tidak" }, &g_det);
    det("Tipe", &info.kind, &g_det);
    det("WWN", if info.wwn.is_empty() { "—" } else { &info.wwn }, &g_det);
    det("Serial", if info.serial.is_empty() { "—" } else { &info.serial }, &g_det);
    if info.rotational {
        det("Rotasi", "HDD (berputar)", &g_det);
    }
    page.add(&g_det);

    let mut parts = Vec::new();
    if !info.partitions.is_empty() {
        let g_part = adw::PreferencesGroup::builder().title("Partisi").build();
        for p in &info.partitions {
            let sub = if p.mount.is_empty() {
                if p.fstype.is_empty() { "tidak terpasang".to_string() } else { p.fstype.clone() }
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
        active_area,
        thru_area,
        thru_scale,
        lbl,
        parts,
    };
    (page, du)
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
        ..Default::default()
    }));
    gather_drives_static(&shared);
    spawn_sampler(shared.clone());

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Tweaks ASUS TUF (Rust)")
        .default_width(600)
        .default_height(860)
        .build();

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let stack = adw::ViewStack::new();
    header.set_title_widget(Some(&gtk::Label::new(Some("Tweaks ASUS TUF"))));
    toolbar.add_top_bar(&header);

    // ── CPU page ──
    let mut core_areas: Vec<gtk::DrawingArea> = Vec::new();
    let (cpu_page, row_model, util_bar, temp_area, labels) = build_cpu_page(&shared, &mut core_areas);
    let sp = stack.add_titled(&cpu_page, Some("cpu"), "CPU");
    sp.set_icon_name(Some("computer-symbolic"));

    // ── Memory page (after CPU) ──
    let (memory_page, row_mem, mem_bar, mem_area, swap_area, mem_lbl) = build_memory_page(&shared);
    let mep = stack.add_titled(&memory_page, Some("memory"), "Memory");
    mep.set_icon_name(Some("drive-harddisk-symbolic"));

    // ── Drive pages (after Memory, one per physical disk) ──
    let drive_snapshot: Vec<DriveInfo> = shared.lock().map(|g| g.drives.clone()).unwrap_or_default();
    let mut drive_uis: Vec<DriveUi> = Vec::new();
    let mut drive_pages_vec: Vec<adw::PreferencesPage> = Vec::new();
    let mut drive_tabs: Vec<(String, String)> = Vec::new(); // (stack name, sidebar label)
    for (i, info) in drive_snapshot.iter().enumerate() {
        let (dpage, du) = build_drive_page(&shared, i, info);
        let sname = format!("drive{i}");
        let dp = stack.add_titled(&dpage, Some(&sname), &format!("Drive {i}"));
        dp.set_icon_name(Some("drive-harddisk-symbolic"));
        drive_tabs.push((sname, format!("{} {} ({})", info.kind, i, info.dev)));
        drive_uis.push(du);
        drive_pages_vec.push(dpage);
    }

    // ── Power page ──
    let power_page = adw::PreferencesPage::new();

    let g_bat = adw::PreferencesGroup::builder().title("Status Baterai &amp; Daya").build();
    let row_bat = adw::ActionRow::builder().title("Baterai: --%").subtitle("Memuat...").build();
    let bat_bar = gtk::LevelBar::builder().min_value(0.0).max_value(100.0).valign(gtk::Align::Center).build();
    bat_bar.set_size_request(110, 16);
    row_bat.add_suffix(&bat_bar);
    g_bat.add(&row_bat);
    let row_drain = adw::ActionRow::builder().title("Sumber Daya &amp; Watt").subtitle("Memuat...").build();
    g_bat.add(&row_drain);
    let row_health = adw::ActionRow::builder().title("Kesehatan Baterai (Pabrik)").subtitle("Memuat...").build();
    g_bat.add(&row_health);
    power_page.add(&g_bat);

    // GPU
    let g_gpu = adw::PreferencesGroup::builder()
        .title("Manajemen &amp; Mode GPU")
        .description("AMD Vega iGPU ↔ NVIDIA GTX 1660 Ti dGPU")
        .build();
    let row_gpu_tel = adw::ActionRow::builder().title("Status GPU NVIDIA").subtitle("Memuat telemetri...").build();
    g_gpu.add(&row_gpu_tel);
    let row_gpu_mode = adw::ActionRow::builder().title("Pilihan Mode Grafis").subtitle("Mode: Hybrid").build();
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
        .title("Kontrol Mode Performa &amp; Daya")
        .description("Biru = aktif")
        .build();
    let row_mode = adw::ActionRow::builder().title("Profil Performa CPU").subtitle("Powersave / Performance / Auto").build();
    let box_mode = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    box_mode.set_valign(gtk::Align::Center);
    let btn_mode = [seg_button("Hemat"), seg_button("Performa"), seg_button("Auto")];
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
    let g_fan = adw::PreferencesGroup::builder().title("Kontrol Kipas &amp; Pendingin (Dual Fan)").build();
    let row_fan_rpm = adw::ActionRow::builder().title("Kecepatan Putaran Kipas").subtitle("CPU Fan: -- RPM | GPU Fan: -- RPM").build();
    g_fan.add(&row_fan_rpm);
    let row_fan_ctrl = adw::ActionRow::builder().title("Profil Kecepatan Kipas").subtitle("Mode: Normal").build();
    let box_fan = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    box_fan.set_valign(gtk::Align::Center);
    let btn_fan = [seg_button("Hening"), seg_button("Normal"), seg_button("Turbo")];
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
        .title("Batas Cas Baterai 80% (Battery Health)")
        .subtitle("Membatasi pengisian di 80% untuk melindungi sel")
        .build();
    g_hw.add(&switch_threshold);
    let row_clamshell = adw::ActionRow::builder()
        .title("Mode Tutup Layar (Clamshell Server)")
        .subtitle("Layar mati saat ditutup, CPU &amp; agent tetap jalan")
        .build();
    let lbl_cs = gtk::Label::new(Some("Aktif"));
    lbl_cs.add_css_class("success");
    lbl_cs.set_valign(gtk::Align::Center);
    row_clamshell.add_suffix(&lbl_cs);
    g_hw.add(&row_clamshell);
    let row_cpu_mon = adw::ActionRow::builder().title("CPU Monitor Real-time").subtitle("Memuat frekuensi...").build();
    g_hw.add(&row_cpu_mon);
    power_page.add(&g_hw);

    let pp = stack.add_titled(&power_page, Some("power"), "Daya & Baterai");
    pp.set_icon_name(Some("battery-symbolic"));

    // ── Keyboard RGB page ──
    let rgb_page = build_rgb_page();
    let rgbp = stack.add_titled(&rgb_page, Some("rgb"), "Keyboard RGB");
    rgbp.set_icon_name(Some("input-keyboard-symbolic"));

    // ── Mouse Logitech page ──
    let m_sync = Rc::new(Cell::new(false));
    let m_pending_dpi: Rc<Cell<Option<(u32, Instant)>>> = Rc::new(Cell::new(None));
    let m_pending_hz: Rc<Cell<Option<(u32, Instant)>>> = Rc::new(Cell::new(None));
    let m_debounce: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    let mouse_page = adw::PreferencesPage::new();

    let g_m = adw::PreferencesGroup::builder()
        .title("Logitech G304 Lightspeed Wireless")
        .description("Receiver USB 046d:C53F • Protocol HID++ 4.2")
        .build();
    let row_m_bat = adw::ActionRow::builder().title("Baterai Mouse G304: --%").subtitle("Memuat...").build();
    let m_bat_bar = gtk::LevelBar::builder().min_value(0.0).max_value(100.0).valign(gtk::Align::Center).build();
    m_bat_bar.set_size_request(110, 16);
    row_m_bat.add_suffix(&m_bat_bar);
    g_m.add(&row_m_bat);
    mouse_page.add(&g_m);

    // Polling rate
    let g_hz = adw::PreferencesGroup::builder()
        .title("Polling Rate (Frekuensi Transfer Data Hz)")
        .description("Semakin tinggi semakin responsif")
        .build();
    let row_m_hz = adw::ActionRow::builder().title("Kecepatan Polling Rate saat Ini").subtitle("Memuat...").build();
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
    let g_dpi = adw::PreferencesGroup::builder().title("Sensitivitas Sensor Optik (DPI)").build();
    let row_m_dpi = adw::ActionRow::builder().title("Nilai DPI Saat Ini").subtitle("-- DPI").build();
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
    let row_dpi_presets = adw::ActionRow::builder().title("Preset DPI Populer").subtitle("Klik untuk ubah sensitivitas seketika").build();
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
    let g_ob = adw::PreferencesGroup::builder().title("Profil Onboard Memory & Anti-Lag USB".replace('&', "&amp;").as_str()).build();
    let switch_onboard = adw::SwitchRow::builder()
        .title("Profil Onboard Memory (EEPROM)")
        .subtitle("Gunakan profil tersimpan di memori fisik mouse G304")
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
        .title("Proteksi USB Autosuspend (Anti Micro-Stutter)")
        .subtitle("Receiver G304 dikunci di mode Power ON (Bebas Lag)")
        .build();
    let lbl_usb = gtk::Label::new(Some("Aktif"));
    lbl_usb.add_css_class("success");
    lbl_usb.set_valign(gtk::Align::Center);
    row_usb.add_suffix(&lbl_usb);
    g_ob.add(&row_usb);
    mouse_page.add(&g_ob);

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
        "Layanan AI &amp; Pengguna (User Daemons)",
        &mut svc_widgets,
    ));
    services_page.add(&build_svc_group(
        &shared,
        &window,
        &SYS_SVC,
        false,
        "Layanan Infrastruktur Sistem (Root)",
        &mut svc_widgets,
    ));
    let svp = stack.add_titled(&services_page, Some("services"), "Layanan Sistem");
    svp.set_icon_name(Some("emblem-system-symbolic"));

    // ── Left sidebar (Mission Center style: icon + label, no graphs) ──
    let sidebar = gtk::ListBox::new();
    sidebar.add_css_class("navigation-sidebar");
    sidebar.set_selection_mode(gtk::SelectionMode::Single);
    sidebar.append(&sidebar_row("cpu", "CPU", "lucide-cpu"));
    sidebar.append(&sidebar_row("memory", "Memory", "lucide-memory-stick"));
    let mut drive_rows_vec: Vec<gtk::ListBoxRow> = Vec::new();
    for (sname, label) in &drive_tabs {
        let row = sidebar_row(sname, label, "lucide-hard-drive");
        sidebar.append(&row);
        drive_rows_vec.push(row);
    }
    sidebar.append(&sidebar_row("power", "Daya & Baterai", "lucide-battery-charging"));
    sidebar.append(&sidebar_row("rgb", "Keyboard RGB", "lucide-keyboard"));
    sidebar.append(&sidebar_row("mouse", "Mouse Logitech", "lucide-mouse"));
    sidebar.append(&sidebar_row("services", "Layanan Sistem", "lucide-server"));
    {
        let stack = stack.clone();
        sidebar.connect_row_selected(move |_lb, row| {
            if let Some(r) = row {
                stack.set_visible_child_name(r.widget_name().as_str());
            }
        });
    }
    if let Some(first) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&first));
    }
    let side_scroll = gtk::ScrolledWindow::new();
    side_scroll.set_child(Some(&sidebar));
    side_scroll.set_size_request(240, -1);
    side_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);

    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    hbox.append(&side_scroll);
    let nav_sep = gtk::Separator::new(gtk::Orientation::Vertical);
    nav_sep.add_css_class("nav-sep");
    hbox.append(&nav_sep);
    stack.set_hexpand(true);
    stack.set_hhomogeneous(true);
    stack.set_vhomogeneous(false);
    hbox.append(&stack);
    toolbar.set_content(Some(&hbox));
    window.set_content(Some(&toolbar));

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
        "window, .background, headerbar, .view, viewswitcher, stackswitcher, \
         scrolledwindow, textview, textview text, preferencespage, clamp, viewport { \
         background-color: #000000; } \
         headerbar { box-shadow: none; border-bottom: 1px solid rgba(255,255,255,0.06); } \
         separator.nav-sep { background-color: #0a0a0a; min-width: 1px; } \
         list, .boxed-list, .card, row { background-color: #0a0a0a; } \
         .boxed-list, .card { border: 1px solid rgba(255,255,255,0.08); border-radius: 10px; } \
         .cpu-graph-frame { border: 1px solid rgba(41,128,236,0.55); border-radius: 6px; \
         background-color: rgba(41,128,236,0.06); } \
         scale.red-slider highlight { background: #ff3b30; } \
         scale.green-slider highlight { background: #34c759; } \
         scale.blue-slider highlight { background: #007aff; } \
         .badge-run { background-color: #2ec27e; color: #fff; border-radius: 6px; padding: 2px 10px; font-weight: bold; } \
         .badge-stop { background-color: #5e5c64; color: #fff; border-radius: 6px; padding: 2px 10px; font-weight: bold; } \
         .badge-fail { background-color: #e01b24; color: #fff; border-radius: 6px; padding: 2px 10px; font-weight: bold; }",
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
        services: svc_widgets,
        row_mem,
        mem_bar,
        mem_area,
        swap_area,
        mem_lbl,
        drives: RefCell::new(drive_uis),
        shared: shared.clone(),
        stack: stack.clone(),
        sidebar: sidebar.clone(),
        drive_rows: RefCell::new(drive_rows_vec),
        drive_pages: RefCell::new(drive_pages_vec),
        drive_sig: RefCell::new(drive_signature(&drive_snapshot)),
    });

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
