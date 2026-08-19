// asus-tuf-cpu — realtime CPU monitor (clean-room Rust/GTK4 reimplementation).
// MIT licensed. Data sources mirror standard Linux sysfs/procfs; no third-party
// application source code is used.

use adw::prelude::*;
use gtk::glib;

use std::collections::VecDeque;
use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const HISTORY: usize = 60; // seconds of history

#[derive(Default)]
struct Shared {
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
    // static info
    model: String,
    base_ghz: String,
    sockets: String,
    logical_str: String,
    virt: String,
    vm: String,
    l1: String,
    l2: String,
    l3: String,
    ready: bool,
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
        let key = match it.next() {
            Some(k) => k,
            None => continue,
        };
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

fn avg_speed_ghz(logical: usize) -> Option<String> {
    let mut sum = 0u64;
    let mut n = 0u64;
    for i in 0..logical {
        if let Ok(s) =
            fs::read_to_string(format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq"))
        {
            if let Ok(v) = s.trim().parse::<u64>() {
                sum += v;
                n += 1;
            }
        }
    }
    if n > 0 {
        Some(format!("{:.2}", (sum as f64 / n as f64) / 1e6))
    } else {
        None
    }
}

fn find_temp_path() -> Option<String> {
    for entry in fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let p = entry.path();
        let name = fs::read_to_string(p.join("name")).unwrap_or_default();
        let name = name.trim();
        if name == "k10temp" || name == "coretemp" || name == "zenpower" {
            let cand = p.join("temp1_input");
            if cand.exists() {
                return Some(cand.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn read_first_u64(path: &str) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn fmt_uptime(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}:{:02}", d, h, m, s)
}

fn count_procs_threads() -> (u64, u64) {
    let mut procs = 0u64;
    let mut threads = 0u64;
    if let Ok(rd) = fs::read_dir("/proc") {
        for e in rd.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.chars().all(|c| c.is_ascii_digit()) && !name.is_empty() {
                procs += 1;
                if let Ok(t) = fs::read_dir(format!("/proc/{}/task", name)) {
                    threads += t.flatten().count() as u64;
                }
            }
        }
    }
    (procs, threads)
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
    let base_ghz = read_first_u64("/sys/devices/system/cpu/cpu0/cpufreq/base_frequency")
        .or_else(|| read_first_u64("/sys/devices/system/cpu/cpu0/cpufreq/bios_limit"))
        .map(|khz| format!("{:.2}", khz as f64 / 1e6))
        .unwrap_or_default();

    let mut sockets = "1".to_string();
    let mut virt = "—".to_string();
    let mut vm = "Tidak".to_string();
    let (mut l1d, mut l1i, mut l2, mut l3) =
        (String::new(), String::new(), String::new(), String::new());
    if let Ok(out) = Command::new("lscpu").output() {
        if let Ok(text) = String::from_utf8(out.stdout) {
            for line in text.lines() {
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
    }
    if model.is_empty() {
        model = "CPU".to_string();
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
        let logical = { sh.lock().map(|g| g.logical).unwrap_or(1) };
        gather_static(&sh, logical);
        let temp_path = find_temp_path();
        let mut prev: Option<((u64, u64), Vec<(u64, u64)>)> = None;
        let mut tick: u64 = 0;
        loop {
            if let Some((ov, per)) = read_stat() {
                if let Some((pov, pper)) = &prev {
                    let overall = pct(*pov, ov);
                    let mut core_vals = Vec::with_capacity(per.len());
                    for (i, c) in per.iter().enumerate() {
                        let p = pper.get(i).copied().unwrap_or((0, 0));
                        core_vals.push(pct(p, *c));
                    }
                    let speed = avg_speed_ghz(logical);
                    let temp = temp_path
                        .as_ref()
                        .and_then(|p| read_first_u64(p))
                        .map(|mc| mc as f64 / 1000.0);
                    let gov = fs::read_to_string(
                        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
                    )
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                    let drv = fs::read_to_string(
                        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver",
                    )
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                    let handles = fs::read_to_string("/proc/sys/fs/file-nr")
                        .ok()
                        .and_then(|s| s.split_whitespace().next().map(|x| x.to_string()))
                        .unwrap_or_default();
                    let uptime = fs::read_to_string("/proc/uptime")
                        .ok()
                        .and_then(|s| s.split_whitespace().next().and_then(|x| x.parse::<f64>().ok()))
                        .map(|u| fmt_uptime(u as u64))
                        .unwrap_or_default();
                    let counts = if tick % 3 == 0 {
                        Some(count_procs_threads())
                    } else {
                        None
                    };

                    if let Ok(mut g) = sh.lock() {
                        g.overall = overall.round() as u32;
                        if let Some(s) = speed {
                            g.speed_ghz = s;
                        }
                        if let Some(t) = temp {
                            g.temp = t.round() as i32;
                            g.temp_hist.push_back(t);
                            while g.temp_hist.len() > HISTORY {
                                g.temp_hist.pop_front();
                            }
                        }
                        if g.per_core.len() != core_vals.len() {
                            g.per_core =
                                vec![VecDeque::from(vec![0.0; HISTORY]); core_vals.len()];
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
                        if !handles.is_empty() {
                            g.handles = handles;
                        }
                        if !uptime.is_empty() {
                            g.uptime = uptime;
                        }
                        if let Some((p, t)) = counts {
                            g.processes = p;
                            g.threads = t;
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

fn draw_graph(cr: &gtk::cairo::Context, w: f64, h: f64, data: &VecDeque<f64>, maxv: f64) {
    // faint gridlines
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
    let yv = |v: f64| -> f64 {
        let v = v.clamp(0.0, maxv);
        h - (v / maxv) * (h - 2.0) - 1.0
    };
    cr.set_line_width(1.6);
    cr.set_source_rgb(0.16, 0.55, 0.96);
    for (i, v) in data.iter().enumerate() {
        let x = i as f64 * stepx;
        let y = yv(*v);
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

fn build_ui(app: &adw::Application) {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let shared = Arc::new(Mutex::new(Shared {
        logical,
        per_core: vec![VecDeque::from(vec![0.0; HISTORY]); logical],
        temp_hist: VecDeque::from(vec![0.0; HISTORY]),
        ..Default::default()
    }));
    spawn_sampler(shared.clone());

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Tweaks ASUS TUF — CPU (Rust)")
        .default_width(560)
        .default_height(840)
        .build();

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar.add_top_bar(&header);

    let page = adw::PreferencesPage::new();

    // Header group
    let g_head = adw::PreferencesGroup::builder().title("Prosesor").build();
    let row_model = adw::ActionRow::builder()
        .title("Memuat model CPU...")
        .subtitle("Utilisasi: --% • Kecepatan: -- GHz")
        .build();
    let util_bar = gtk::LevelBar::builder()
        .min_value(0.0)
        .max_value(100.0)
        .value(0.0)
        .valign(gtk::Align::Center)
        .build();
    util_bar.set_size_request(110, 16);
    row_model.add_suffix(&util_bar);
    g_head.add(&row_model);
    page.add(&g_head);

    // Per-core graphs
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
    let mut core_areas: Vec<gtk::DrawingArea> = Vec::with_capacity(logical);
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
        core_areas.push(area);
    }
    g_cores.add(&grid);
    page.add(&g_cores);

    // Temperature graph
    let g_temp = adw::PreferencesGroup::builder()
        .title("Suhu CPU (1 menit)")
        .build();
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

    // Detail info
    let g_info = adw::PreferencesGroup::builder()
        .title("Informasi Detail")
        .build();
    let l_speed = info_row("Kecepatan Saat Ini", &g_info);
    let l_base = info_row("Base Speed", &g_info);
    let l_logical = info_row("Prosesor Logis", &g_info);
    let l_sockets = info_row("Socket", &g_info);
    let l_virt = info_row("Virtualisasi", &g_info);
    let l_vm = info_row("Virtual Machine", &g_info);
    let l_l1 = info_row("Cache L1 (data / instruksi)", &g_info);
    let l_l2 = info_row("Cache L2", &g_info);
    let l_l3 = info_row("Cache L3", &g_info);
    let l_driver = info_row("Cpufreq Driver", &g_info);
    let l_gov = info_row("Cpufreq Governor", &g_info);
    let l_pth = info_row("Proses / Thread / Handle", &g_info);
    let l_uptime = info_row("Uptime Sistem", &g_info);
    let l_temp = info_row("Suhu CPU", &g_info);
    page.add(&g_info);

    toolbar.set_content(Some(&page));
    window.set_content(Some(&toolbar));

    // CSS for graph frames
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        ".cpu-graph-frame { border: 1px solid rgba(41,128,236,0.55); border-radius: 6px; \
         background-color: rgba(41,128,236,0.06); }",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    // 1s UI refresh
    let sh = shared.clone();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        if let Ok(g) = sh.lock() {
            if g.ready {
                row_model.set_title(&g.model);
                row_model.set_subtitle(&format!(
                    "Utilisasi: {}% • Kecepatan: {} GHz",
                    g.overall,
                    if g.speed_ghz.is_empty() { "--" } else { &g.speed_ghz }
                ));
                util_bar.set_value(g.overall as f64);
                l_speed.set_text(&format!(
                    "{} GHz",
                    if g.speed_ghz.is_empty() { "--" } else { &g.speed_ghz }
                ));
                l_base.set_text(&if g.base_ghz.is_empty() {
                    "—".to_string()
                } else {
                    format!("{} GHz", g.base_ghz)
                });
                l_logical.set_text(&g.logical_str);
                l_sockets.set_text(&g.sockets);
                l_virt.set_text(&g.virt);
                l_vm.set_text(&g.vm);
                l_l1.set_text(&g.l1);
                l_l2.set_text(&g.l2);
                l_l3.set_text(&g.l3);
                l_driver.set_text(if g.driver.is_empty() { "—" } else { &g.driver });
                l_gov.set_text(if g.governor.is_empty() { "—" } else { &g.governor });
                l_pth.set_text(&format!(
                    "{} / {} / {}",
                    g.processes,
                    g.threads,
                    if g.handles.is_empty() { "--" } else { &g.handles }
                ));
                l_uptime.set_text(if g.uptime.is_empty() { "--" } else { &g.uptime });
                l_temp.set_text(&format!("{} °C", g.temp));
            }
        }
        for a in &core_areas {
            a.queue_draw();
        }
        temp_area.queue_draw();
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
