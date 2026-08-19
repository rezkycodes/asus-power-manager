// asus-tuf-cpu — Tweaks ASUS TUF (Rust/GTK4 reimplementation, MIT).
// Clean-room: uses standard Linux sysfs/procfs + the project's own bash scripts.
// Tabs: CPU (realtime graphs) and Daya & Baterai (power/GPU/fan controls).

use adw::prelude::*;
use gtk::glib;

use std::cell::Cell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const HISTORY: usize = 60;

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

                    // heavy subprocess data every 3s
                    let counts = if tick % 3 == 0 { Some(count_procs_threads()) } else { None };
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
                        if let Some((p, t)) = counts {
                            g.processes = p;
                            g.threads = t;
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

fn build_ui(app: &adw::Application) {
    let logical = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let shared = Arc::new(Mutex::new(Shared {
        logical,
        per_core: vec![VecDeque::from(vec![0.0; HISTORY]); logical],
        temp_hist: VecDeque::from(vec![0.0; HISTORY]),
        ..Default::default()
    }));
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
    let switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    header.set_title_widget(Some(&switcher));
    toolbar.add_top_bar(&header);

    // ── CPU page ──
    let mut core_areas: Vec<gtk::DrawingArea> = Vec::new();
    let (cpu_page, row_model, util_bar, temp_area, labels) = build_cpu_page(&shared, &mut core_areas);
    let sp = stack.add_titled(&cpu_page, Some("cpu"), "CPU");
    sp.set_icon_name(Some("computer-symbolic"));

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

    toolbar.set_content(Some(&stack));
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
        ".cpu-graph-frame { border: 1px solid rgba(41,128,236,0.55); border-radius: 6px; \
         background-color: rgba(41,128,236,0.06); }",
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
