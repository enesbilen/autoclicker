#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use device_query::{DeviceQuery, DeviceState, Keycode};
use eframe::egui;
use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Mouse,
    Keyboard,
    Sequence,
    Record,
}

#[derive(Clone, Copy, PartialEq)]
enum Rep {
    Infinite,
    Count,
    Time,
}

#[derive(Clone, Copy, PartialEq)]
enum Btn {
    Left,
    Right,
    Middle,
}

#[derive(Clone, PartialEq)]
struct Step {
    x: i32,
    y: i32,
    btn: Btn,
    delay_ms: u32,
}

#[derive(Clone, PartialEq)]
struct Config {
    h: u32,
    m: u32,
    s: u32,
    ms: u32,
    mode: Mode,
    btn: Btn,
    double: bool,
    fixed_pos: bool,
    x: i32,
    y: i32,
    rep: Rep,
    repeat: u64,
    limit_min: u32,
    limit_sec: u32,
    key_idx: usize,
    mods: [bool; 4], // ctrl, shift, alt, meta
    hold_ms: u32,
    key_hold_ms: u32,
    notify: bool,
    steps: Vec<Step>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            h: 0,
            m: 0,
            s: 0,
            ms: 100,
            mode: Mode::Mouse,
            btn: Btn::Left,
            double: false,
            fixed_pos: false,
            x: 0,
            y: 0,
            rep: Rep::Infinite,
            repeat: 100,
            limit_min: 5,
            limit_sec: 0,
            key_idx: 0,
            mods: [false; 4],
            hold_ms: 0,
            key_hold_ms: 0,
            notify: true,
            steps: vec![],
        }
    }
}

impl Config {
    fn interval_ms(&self) -> u64 {
        let t = self.h as u64 * 3_600_000
            + self.m as u64 * 60_000
            + self.s as u64 * 1000
            + self.ms as u64;
        t.max(1)
    }
}

struct Shared {
    cfg: Mutex<Config>,
    running: AtomicBool,
    clicks: AtomicU64,
    started: Mutex<Option<Instant>>,
    elapsed_ms: AtomicU64, // son çalışmanın süresi
    start_key: Mutex<Keycode>,
    stop_key: Mutex<Keycode>,
    capturing: AtomicU8, // 0 yok, 1 başlat, 2 durdur
    pos: Mutex<(i32, i32)>,
    error: Mutex<String>,
    recording: AtomicBool,
    rec: Mutex<Vec<Step>>,
    rec_last: Mutex<Option<Instant>>,
}

fn lk<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn key_list() -> Vec<(&'static str, Key)> {
    let mut v: Vec<(&'static str, Key)> = vec![
        ("Enter", Key::Return),
        ("Space", Key::Space),
        ("Tab", Key::Tab),
        ("Esc", Key::Escape),
        ("Backspace", Key::Backspace),
        ("Delete", Key::Delete),
        ("Up", Key::UpArrow),
        ("Down", Key::DownArrow),
        ("Left", Key::LeftArrow),
        ("Right", Key::RightArrow),
        ("F1", Key::F1),
        ("F2", Key::F2),
        ("F3", Key::F3),
        ("F4", Key::F4),
        ("F5", Key::F5),
        ("F6", Key::F6),
        ("F7", Key::F7),
        ("F8", Key::F8),
        ("F9", Key::F9),
        ("F10", Key::F10),
        ("F11", Key::F11),
        ("F12", Key::F12),
    ];
    const CHARS: [&str; 36] = [
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u",
        "v", "w", "x", "y", "z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    ];
    for name in CHARS {
        if let Some(c) = name.chars().next() {
            v.push((name, Key::Unicode(c)));
        }
    }
    v
}

fn notify(count: u64) {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("afplay").arg("/System/Library/Sounds/Glass.aiff").spawn();
        let msg = format!("display notification \"{count} tıklama tamamlandı\" with title \"AutoClicker\"");
        let _ = Command::new("osascript").args(["-e", &msg]).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = count;
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", "[System.Media.SystemSounds]::Asterisk.Play(); Start-Sleep -Milliseconds 1000"])
            .creation_flags(0x0800_0000)
            .spawn();
    }
}

/// Başlat/durdur isteği; Kayıt sekmesinde motor `running` değişimini kayıt aç/kapa olarak yorumlar.
fn request_run(sh: &Shared, want: Option<bool>) {
    let record = lk(&sh.cfg).mode == Mode::Record;
    let cur = if record { sh.recording.load(Ordering::Relaxed) } else { sh.running.load(Ordering::Relaxed) };
    let want = want.unwrap_or(!cur);
    if want == cur {
        return;
    }
    if record {
        sh.running.store(true, Ordering::Relaxed); // motor kaydı tersine çevirir
    } else {
        sh.running.store(want, Ordering::Relaxed);
    }
}

fn release_all() {
    if let Ok(mut e) = Enigo::new(&Settings::default()) {
        for b in [Button::Left, Button::Right, Button::Middle] {
            let _ = e.button(b, Direction::Release);
        }
        for k in [Key::Control, Key::Shift, Key::Alt, Key::Meta] {
            let _ = e.key(k, Direction::Release);
        }
    }
}

fn to_button(b: Btn) -> Button {
    match b {
        Btn::Left => Button::Left,
        Btn::Right => Button::Right,
        Btn::Middle => Button::Middle,
    }
}

/// Hedef ana kadar bekler; durdurulursa veya süre dolarsa erken döner.
fn sleep_until(sh: &Shared, sl: &spin_sleep::SpinSleeper, until: Instant, deadline: Option<Instant>) {
    loop {
        if !sh.running.load(Ordering::Relaxed) {
            return;
        }
        let now = Instant::now();
        if deadline.is_some_and(|d| now >= d) {
            sh.running.store(false, Ordering::Relaxed);
            return;
        }
        if now >= until {
            return;
        }
        sl.sleep((until - now).min(Duration::from_millis(20)));
    }
}

fn click_btn(enigo: &mut Enigo, sh: &Shared, sl: &spin_sleep::SpinSleeper, b: Button, double: bool, hold_ms: u32) {
    for i in 0..(if double { 2 } else { 1 }) {
        if i > 0 && !sh.running.load(Ordering::Relaxed) {
            break;
        }
        if hold_ms > 0 {
            let _ = enigo.button(b, Direction::Press);
            sleep_until(sh, sl, Instant::now() + Duration::from_millis(hold_ms as u64), None);
            let _ = enigo.button(b, Direction::Release); // durdurulsa bile bırak
        } else {
            let _ = enigo.button(b, Direction::Click);
        }
    }
}

fn engine_run(sh: Arc<Shared>) {
    let keys = key_list();
    let sleeper = spin_sleep::SpinSleeper::default();
    let mut enigo: Option<Enigo> = None;
    loop {
        if !sh.running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        if lk(&sh.cfg).mode == Mode::Record {
            sh.running.store(false, Ordering::Relaxed);
            let r = sh.recording.load(Ordering::Relaxed);
            if !r {
                lk(&sh.rec).clear();
                *lk(&sh.rec_last) = None;
            }
            sh.recording.store(!r, Ordering::Relaxed);
            continue;
        }
        if enigo.is_none() {
            match Enigo::new(&Settings::default()) {
                Ok(e) => enigo = Some(e),
                Err(e) => {
                    *lk(&sh.error) = format!("Girdi başlatılamadı (izinleri kontrol et): {e}");
                    sh.running.store(false, Ordering::Relaxed);
                    continue;
                }
            }
        }
        let enigo = enigo.as_mut().unwrap();
        let cfg = lk(&sh.cfg).clone();
        if cfg.rep == Rep::Time && cfg.limit_min == 0 && cfg.limit_sec == 0 {
            *lk(&sh.error) = "Zaman kuralı: süre 0 olamaz.".into();
            sh.running.store(false, Ordering::Relaxed);
            continue;
        }
        if cfg.mode == Mode::Sequence && cfg.steps.is_empty() {
            *lk(&sh.error) = "Dizi boş: önce adım ekle.".into();
            sh.running.store(false, Ordering::Relaxed);
            continue;
        }
        lk(&sh.error).clear();
        sh.clicks.store(0, Ordering::Relaxed);
        let t0 = Instant::now();
        *lk(&sh.started) = Some(t0);
        let deadline = (cfg.rep == Rep::Time)
            .then(|| t0 + Duration::from_secs(cfg.limit_min as u64 * 60 + cfg.limit_sec as u64));
        let mut count: u64 = 0;
        let mut done_count = false;
        let b = to_button(cfg.btn);
        let failsafe = |sh: &Shared| -> bool {
            // sabit konumda değilken imleç sol üst köşedeyse acil durdur
            let (x, y) = *lk(&sh.pos);
            if (0..=2).contains(&x) && (0..=2).contains(&y) {
                *lk(&sh.error) = "Acil durdurma: imleç köşeye götürüldü.".into();
                sh.running.store(false, Ordering::Relaxed);
                true
            } else {
                false
            }
        };

        if cfg.mode == Mode::Sequence {
            let mut loops: u64 = 0;
            'outer: while sh.running.load(Ordering::Relaxed) {
                let n_steps = cfg.steps.len();
                for (si, st) in cfg.steps.iter().enumerate() {
                    if !sh.running.load(Ordering::Relaxed) {
                        break 'outer;
                    }
                    let _ = enigo.move_mouse(st.x, st.y, Coordinate::Abs);
                    click_btn(enigo, &sh, &sleeper, to_button(st.btn), false, 0);
                    count += 1;
                    sh.clicks.store(count, Ordering::Relaxed);
                    if si + 1 == n_steps && cfg.rep == Rep::Count && loops + 1 >= cfg.repeat {
                        done_count = true;
                        sh.running.store(false, Ordering::Relaxed);
                        break 'outer;
                    }
                    sleep_until(&sh, &sleeper, Instant::now() + Duration::from_millis(st.delay_ms as u64), deadline);
                }
                loops += 1;
                if cfg.rep == Rep::Count && loops >= cfg.repeat {
                    done_count = true;
                    sh.running.store(false, Ordering::Relaxed);
                }
            }
        } else {
            let interval = Duration::from_millis(cfg.interval_ms());
            let mut next = t0;
            while sh.running.load(Ordering::Relaxed) {
                sleep_until(&sh, &sleeper, next, deadline);
                if !sh.running.load(Ordering::Relaxed) {
                    break;
                }
                match cfg.mode {
                    Mode::Mouse => {
                        if cfg.fixed_pos {
                            let _ = enigo.move_mouse(cfg.x, cfg.y, Coordinate::Abs);
                        } else if failsafe(&sh) {
                            break;
                        }
                        click_btn(enigo, &sh, &sleeper, b, cfg.double, cfg.hold_ms);
                    }
                    Mode::Keyboard => {
                        if let Some((_, k)) = keys.get(cfg.key_idx) {
                            let mods = [Key::Control, Key::Shift, Key::Alt, Key::Meta];
                            for (i, m) in mods.iter().enumerate() {
                                if cfg.mods[i] {
                                    let _ = enigo.key(*m, Direction::Press);
                                }
                            }
                            if cfg.key_hold_ms > 0 {
                                let _ = enigo.key(*k, Direction::Press);
                                sleep_until(&sh, &sleeper, Instant::now() + Duration::from_millis(cfg.key_hold_ms as u64), None);
                                let _ = enigo.key(*k, Direction::Release);
                            } else {
                                let _ = enigo.key(*k, Direction::Click);
                            }
                            for (i, m) in mods.iter().enumerate().rev() {
                                if cfg.mods[i] {
                                    let _ = enigo.key(*m, Direction::Release);
                                }
                            }
                        }
                    }
                    Mode::Sequence | Mode::Record => {}
                }
                count += 1;
                sh.clicks.store(count, Ordering::Relaxed);
                if cfg.rep == Rep::Count && count >= cfg.repeat {
                    done_count = true;
                    sh.running.store(false, Ordering::Relaxed);
                    break;
                }
                next += interval;
                let now = Instant::now();
                if next < now {
                    next = now; // geride kaldıysak yığılma yapma
                }
            }
        }
        sh.elapsed_ms.store(t0.elapsed().as_millis() as u64, Ordering::Relaxed);
        *lk(&sh.started) = None;
        let done_time = deadline.is_some_and(|d| Instant::now() >= d);
        if cfg.notify && count > 0 && (done_count || done_time) {
            notify(count);
        }
    }
}

/// Thread çökerse hatayı göster ve yeniden başlat; uygulama kapanmasın.
fn supervised(sh: Arc<Shared>, name: &'static str, f: fn(Arc<Shared>)) {
    std::thread::spawn(move || loop {
        let s = sh.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(s)));
        if r.is_ok() {
            break;
        }
        sh.running.store(false, Ordering::Relaxed);
        *lk(&sh.started) = None;
        if name == "Motor" {
            release_all();
        }
        *lk(&sh.error) = format!("{name} beklenmedik şekilde durdu, yeniden başlatıldı.");
        std::thread::sleep(Duration::from_millis(500));
    });
}

fn hotkey_thread(sh: Arc<Shared>) {
    let ds = DeviceState::new();
    let mut prev: Vec<Keycode> = vec![];
    let mut prev_btn = [false; 3];
    loop {
        std::thread::sleep(Duration::from_millis(8));
        let keys = ds.get_keys();
        let pressed: Vec<Keycode> = keys.iter().filter(|k| !prev.contains(k)).cloned().collect();
        let ms = ds.get_mouse();
        let (mx, my) = ms.coords;
        *lk(&sh.pos) = (mx, my);
        let cur = [1usize, 2, 3].map(|i| ms.button_pressed.get(i).copied().unwrap_or(false));
        if sh.recording.load(Ordering::Relaxed) {
            for (i, b) in [Btn::Left, Btn::Right, Btn::Middle].iter().enumerate() {
                if cur[i] && !prev_btn[i] {
                    let now = Instant::now();
                    let last = *lk(&sh.rec_last);
                    let mut r = lk(&sh.rec);
                    if let (Some(t), Some(l)) = (last, r.last_mut()) {
                        l.delay_ms = now.duration_since(t).as_millis().min(3_600_000) as u32;
                    }
                    r.push(Step { x: mx, y: my, btn: *b, delay_ms: 500 });
                    drop(r);
                    *lk(&sh.rec_last) = Some(now);
                }
            }
        }
        prev_btn = cur;
        if !pressed.is_empty() {
            let cap = sh.capturing.load(Ordering::Relaxed);
            if cap != 0 {
                let k = pressed[0];
                if k != Keycode::Escape {
                    if cap == 1 {
                        *lk(&sh.start_key) = k;
                    } else {
                        *lk(&sh.stop_key) = k;
                    }
                }
                sh.capturing.store(0, Ordering::Relaxed);
            } else {
                let sk = *lk(&sh.start_key);
                let tk = *lk(&sh.stop_key);
                for k in &pressed {
                    if sk == tk && *k == sk {
                        request_run(&sh, None);
                    } else if *k == sk {
                        request_run(&sh, Some(true));
                    } else if *k == tk {
                        request_run(&sh, Some(false));
                    }
                }
            }
        }
        prev = keys;
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "enesbilen/autoclicker";

#[derive(Clone)]
struct Update {
    version: String,
    url: String,
    name: String,
}

#[derive(Default)]
struct UpdateState {
    available: Option<Update>,
    status: String,
}

fn parse_ver(v: &str) -> Vec<u32> {
    v.trim_start_matches('v')
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

fn check_update(st: Arc<Mutex<UpdateState>>) {
    std::thread::spawn(move || {
        let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
        let Ok(resp) = ureq::get(&url).set("User-Agent", "autoclicker").call() else { return };
        let Ok(v) = resp.into_json::<serde_json::Value>() else { return };
        let tag = v["tag_name"].as_str().unwrap_or("").to_string();
        if parse_ver(&tag) <= parse_ver(VERSION) {
            return;
        }
        let want = if cfg!(target_os = "macos") { ".dmg" } else { ".exe" };
        if let Some(a) = v["assets"]
            .as_array()
            .and_then(|l| l.iter().find(|a| a["name"].as_str().unwrap_or("").ends_with(want)))
        {
            lk(&st).available = Some(Update {
                version: tag,
                url: a["browser_download_url"].as_str().unwrap_or("").to_string(),
                name: a["name"].as_str().unwrap_or("update").to_string(),
            });
        }
    });
}

fn download_update(u: Update, st: Arc<Mutex<UpdateState>>) {
    lk(&st).status = "İndiriliyor...".into();
    std::thread::spawn(move || {
        let mut partial: Option<std::path::PathBuf> = None;
        let res = (|| -> Result<std::path::PathBuf, String> {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .map_err(|e| e.to_string())?;
            let path = std::path::Path::new(&home).join("Downloads").join(&u.name);
            let resp = ureq::get(&u.url).set("User-Agent", "autoclicker").call().map_err(|e| e.to_string())?;
            let expected: Option<u64> = resp.header("Content-Length").and_then(|v| v.parse().ok());
            let mut f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
            partial = Some(path.clone());
            let n = std::io::copy(&mut resp.into_reader(), &mut f).map_err(|e| e.to_string())?;
            if expected.is_some_and(|e| e != n) {
                return Err("indirme eksik kaldı".into());
            }
            partial = None;
            Ok(path)
        })();
        let msg = match res {
            Ok(p) => {
                #[cfg(target_os = "macos")]
                let _ = std::process::Command::new("open").arg(&p).spawn();
                #[cfg(target_os = "windows")]
                let _ = std::process::Command::new("explorer").arg("/select,").arg(&p).spawn();
                "İndirildi (İndirilenler klasörü). Eski sürümü kapatıp yenisini aç.".to_string()
            }
            Err(e) => {
                if let Some(p) = partial {
                    let _ = std::fs::remove_file(p);
                }
                format!("Hata: {e}")
            }
        };
        lk(&st).status = msg;
    });
}

// ---------- Tema ----------
use egui::{Color32, FontId, RichText, Stroke, Vec2};

const BG: Color32 = Color32::from_rgb(11, 13, 18);
const CARD: Color32 = Color32::from_rgb(18, 21, 28);
const BORDER: Color32 = Color32::from_rgb(33, 38, 50);
const TRACK: Color32 = Color32::from_rgb(9, 11, 15);
const TXT: Color32 = Color32::from_rgb(232, 235, 242);
const DIM: Color32 = Color32::from_rgb(140, 147, 163);
const GREEN: Color32 = Color32::from_rgb(16, 185, 129);
const BLUE: Color32 = Color32::from_rgb(59, 130, 246);
const AMBER: Color32 = Color32::from_rgb(245, 158, 11);
const RED: Color32 = Color32::from_rgb(239, 68, 68);

#[derive(Clone, Copy)]
enum Tone {
    Gray,
    Blue,
    Amber,
}

fn apply_style(ctx: &egui::Context) {
    let mut st = (*ctx.style()).clone();
    st.visuals = egui::Visuals::dark();
    st.visuals.panel_fill = BG;
    st.visuals.window_fill = CARD;
    st.visuals.selection.bg_fill = BLUE;
    st.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    for w in [
        &mut st.visuals.widgets.inactive,
        &mut st.visuals.widgets.hovered,
        &mut st.visuals.widgets.active,
    ] {
        w.rounding = 8.0.into();
    }
    st.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(30, 34, 44);
    st.visuals.widgets.inactive.bg_fill = Color32::from_rgb(30, 34, 44);
    st.spacing.item_spacing = Vec2::new(8.0, 6.0);
    st.spacing.button_padding = Vec2::new(10.0, 5.0);
    ctx.set_style(st);
}

fn dot(ui: &mut egui::Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
    ui.painter().circle_filled(r.center(), 4.0, color);
}

fn card(ui: &mut egui::Ui, color: Color32, title: &str, right: Option<String>, f: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(16.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                dot(ui, color);
                ui.label(RichText::new(title).size(12.5).strong().color(Color32::from_rgb(205, 210, 222)));
                if let Some(r) = right {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(r).monospace().size(12.0).color(DIM));
                    });
                }
            });
            ui.add_space(4.0);
            f(ui);
        });
}

fn seg<T: PartialEq + Copy>(ui: &mut egui::Ui, val: &mut T, items: &[(T, &str)], tone: Tone) {
    let (fill, stroke, txt) = match tone {
        Tone::Gray => (Color32::from_rgb(42, 46, 56), Color32::from_rgb(78, 84, 98), Color32::WHITE),
        Tone::Blue => (Color32::from_rgb(18, 38, 78), BLUE, Color32::from_rgb(140, 185, 255)),
        Tone::Amber => (Color32::from_rgb(58, 41, 10), AMBER, AMBER),
    };
    egui::Frame::none().fill(TRACK).rounding(12.0).inner_margin(4.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let n = items.len() as f32;
        let w = ((ui.available_width() - 4.0 * (n - 1.0)) / n).floor();
        ui.horizontal(|ui| {
            for (v, label) in items {
                let sel = *val == *v;
                let b = egui::Button::new(RichText::new(*label).size(14.0).color(if sel { txt } else { DIM }))
                    .fill(if sel { fill } else { Color32::TRANSPARENT })
                    .stroke(if sel { Stroke::new(1.0_f32, stroke) } else { Stroke::NONE })
                    .rounding(9.0);
                if ui.add_sized([w, 32.0], b).clicked() {
                    *val = *v;
                }
            }
        });
    });
}

fn field(ui: &mut egui::Ui, v: &mut u32, max: u32, label: &str, hot: bool) {
    let w = ui.available_width();
    egui::Frame::none()
        .fill(TRACK)
        .stroke(Stroke::new(1.0_f32, if hot { Color32::from_rgb(10, 110, 80) } else { BORDER }))
        .rounding(12.0)
        .show(ui, |ui| {
            let st = ui.style_mut();
            st.override_font_id = Some(FontId::monospace(22.0));
            st.visuals.override_text_color = Some(if hot { GREEN } else { Color32::WHITE });
            for wv in [&mut st.visuals.widgets.inactive, &mut st.visuals.widgets.hovered, &mut st.visuals.widgets.active] {
                wv.weak_bg_fill = Color32::TRANSPARENT;
                wv.bg_fill = Color32::TRANSPARENT;
                wv.bg_stroke = Stroke::NONE;
            }
            ui.add_sized([w - 2.0, 44.0], egui::DragValue::new(v).range(0..=max));
        });
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(label).color(if hot { GREEN } else { DIM }));
    });
}

fn hold_row(ui: &mut egui::Ui, hold: &mut u32) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new("Basılı Tutma Süresi").size(15.0).color(TXT));
            ui.label(RichText::new("0 ms = anlık standart tıklama").size(12.5).color(DIM));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new("ms").color(DIM));
            egui::Frame::none().fill(TRACK).stroke(Stroke::new(1.0_f32, BORDER)).rounding(10.0).show(ui, |ui| {
                let st = ui.style_mut();
                st.override_font_id = Some(FontId::monospace(16.0));
                for wv in [&mut st.visuals.widgets.inactive, &mut st.visuals.widgets.hovered, &mut st.visuals.widgets.active] {
                    wv.weak_bg_fill = Color32::TRANSPARENT;
                    wv.bg_fill = Color32::TRANSPARENT;
                    wv.bg_stroke = Stroke::NONE;
                }
                ui.add_sized([86.0, 32.0], egui::DragValue::new(hold).range(0..=60000));
            });
        });
    });
}

fn small_btn(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(egui::Button::new(RichText::new(text).size(13.0).color(TXT)).fill(Color32::from_rgb(30, 34, 44)).rounding(8.0))
        .clicked()
}

fn big_button(ui: &mut egui::Ui, label: &str, chip: &str, fill: Color32, text: Color32) -> bool {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 54.0), egui::Sense::click());
    let fill = if resp.hovered() { fill.gamma_multiply(1.12) } else { fill };
    ui.painter().rect_filled(rect, 16.0, fill);
    let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(19.0), text);
    let c = ui.painter().layout_no_wrap(chip.to_string(), FontId::monospace(13.0), text);
    let chip_w = c.size().x + 18.0;
    let total = g.size().x + 14.0 + chip_w;
    let x0 = rect.center().x - total / 2.0;
    let gp = egui::pos2(x0, rect.center().y - g.size().y / 2.0);
    ui.painter().galley(gp, g.clone(), text);
    let chip_rect = egui::Rect::from_min_size(
        egui::pos2(x0 + g.size().x + 14.0, rect.center().y - 12.0),
        Vec2::new(chip_w, 24.0),
    );
    ui.painter().rect_filled(chip_rect, 7.0, Color32::from_black_alpha(40));
    ui.painter().galley(
        egui::pos2(chip_rect.center().x - c.size().x / 2.0, chip_rect.center().y - c.size().y / 2.0),
        c,
        text,
    );
    resp.clicked()
}

struct App {
    _tray: Option<tray_icon::TrayIcon>,
    sh: Arc<Shared>,
    cfg: Config,
    keys: Vec<(&'static str, Key)>,
    pick_at: Option<(Instant, bool)>, // (zaman, yeni adım mı)
    upd: Arc<Mutex<UpdateState>>,
    icon: Option<egui::TextureHandle>,
    settings_open: bool,
    pinned: bool,
    was_rec: bool,
    frames: u32,
}

impl App {
    fn sync_cfg(&self) {
        if !self.sh.running.load(Ordering::Relaxed) {
            let mut g = lk(&self.sh.cfg);
            if *g != self.cfg {
                *g = self.cfg.clone();
            }
        }
    }

    fn finish_record(&mut self, pop_last: bool) {
        self.sh.recording.store(false, Ordering::Relaxed);
        let mut r = lk(&self.sh.rec).clone();
        if pop_last {
            r.pop(); // durdur düğmesine yapılan tık
        }
        if let Some(l) = r.last_mut() {
            l.delay_ms = 500;
        }
        if !r.is_empty() {
            self.cfg.steps = r;
            self.cfg.mode = Mode::Sequence;
        }
        self.was_rec = false;
    }

    fn header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            if let Some(t) = &self.icon {
                ui.add(egui::Image::new(egui::load::SizedTexture::new(t.id(), Vec2::splat(26.0))));
            }
            ui.label(RichText::new("AutoClicker").size(17.0).strong().color(TXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let pin_col = if self.pinned { GREEN } else { DIM };
                if ui
                    .add(egui::Button::new(RichText::new("📌").size(16.0).color(pin_col)).frame(false))
                    .on_hover_text("Her zaman üstte")
                    .clicked()
                {
                    self.pinned = !self.pinned;
                    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if self.pinned {
                        egui::WindowLevel::AlwaysOnTop
                    } else {
                        egui::WindowLevel::Normal
                    }));
                }
                let gear = if self.settings_open { GREEN } else { DIM };
                if ui
                    .add(egui::Button::new(RichText::new("⚙").size(17.0).color(gear)).frame(false))
                    .on_hover_text("Ayarlar")
                    .clicked()
                {
                    self.settings_open = !self.settings_open;
                }
                egui::Frame::none()
                    .fill(Color32::from_rgb(8, 40, 33))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(10, 100, 75)))
                    .rounding(14.0)
                    .inner_margin(egui::Margin::symmetric(10.0, 3.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("v{VERSION}")).monospace().size(12.5).color(GREEN));
                            dot(ui, GREEN);
                        });
                    });
            });
        });
    }

    fn settings_view(&mut self, ui: &mut egui::Ui) {
        let cap = self.sh.capturing.load(Ordering::Relaxed);
        card(ui, GREEN, "KISAYOL TUŞLARI", None, |ui| {
            let sk = format!("{:?}", *lk(&self.sh.start_key));
            let tk = format!("{:?}", *lk(&self.sh.stop_key));
            ui.horizontal(|ui| {
                ui.label(RichText::new("Başlat").color(TXT));
                if small_btn(ui, &if cap == 1 { "tuşa bas…".to_string() } else { sk }) {
                    self.sh.capturing.store(1, Ordering::Relaxed);
                }
                ui.add_space(10.0);
                ui.label(RichText::new("Durdur").color(TXT));
                if small_btn(ui, &if cap == 2 { "tuşa bas…".to_string() } else { tk }) {
                    self.sh.capturing.store(2, Ordering::Relaxed);
                }
            });
            ui.label(RichText::new("Başlat ve durdur aynı tuşsa aç/kapa olarak çalışır.").size(12.5).color(DIM));
        });
        card(ui, BLUE, "HAKKINDA", None, |ui| {
            ui.label(RichText::new(format!("AutoClicker v{VERSION}")).color(TXT));
            let u = lk(&self.upd).available.clone();
            match u {
                Some(u) => ui.label(RichText::new(format!("Yeni sürüm var: {}", u.version)).color(GREEN)),
                None => ui.label(RichText::new("Güncel sürümü kullanıyorsun.").color(DIM)),
            };
            ui.label(RichText::new("github.com/enesbilen/autoclicker").size(12.5).color(DIM));
        });
        card(ui, AMBER, "ACİL DURDURMA", None, |ui| {
            ui.label(RichText::new("Fare modunda (sabit konum kapalıyken) imleci ekranın sol üst köşesine götür.").size(13.0).color(DIM));
        });
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.sh.running.store(false, Ordering::Relaxed);
        self.sh.recording.store(false, Ordering::Relaxed);
    }

    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(50));
        if !self.settings_open {
            self.sh.capturing.store(0, Ordering::Relaxed);
        }
        let running = self.sh.running.load(Ordering::Relaxed);
        let recording = self.sh.recording.load(Ordering::Relaxed);
        if self.was_rec && !recording {
            self.finish_record(false);
        }
        self.was_rec = recording;

        if let Some((t, new_step)) = self.pick_at {
            if Instant::now() >= t {
                let (x, y) = *lk(&self.sh.pos);
                if new_step {
                    self.cfg.steps.push(Step { x, y, btn: Btn::Left, delay_ms: 500 });
                } else {
                    self.cfg.x = x;
                    self.cfg.y = y;
                    self.cfg.fixed_pos = true;
                }
                self.pick_at = None;
            }
        }
        self.sync_cfg();

        // ---- Üst: başlık + sekmeler + güncelleme bandı
        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin { left: 16.0, right: 16.0, top: 12.0, bottom: 8.0 }))
            .show(ctx, |ui| {
                self.header(ui, ctx);
                ui.add_space(4.0);
                ui.add_enabled_ui(!running && !recording, |ui| {
                    seg(
                        ui,
                        &mut self.cfg.mode,
                        &[(Mode::Mouse, "Fare"), (Mode::Keyboard, "Klavye"), (Mode::Sequence, "Dizi"), (Mode::Record, "Kayıt")],
                        Tone::Gray,
                    );
                });
                let upd = lk(&self.upd).available.clone();
                if let Some(u) = upd {
                    let status = lk(&self.upd).status.clone();
                    egui::Frame::none().fill(Color32::from_rgb(8, 40, 33)).rounding(10.0).inner_margin(8.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("Yeni sürüm: {}", u.version)).color(GREEN));
                            if (status.is_empty() || status.starts_with("Hata")) && small_btn(ui, if status.is_empty() { "Güncelle" } else { "Tekrar dene" }) {
                                download_update(u.clone(), self.upd.clone());
                            }
                        });
                        if !status.is_empty() {
                            ui.label(RichText::new(status).size(12.0).color(DIM));
                        }
                    });
                }
            });

        // ---- Alt: Başlat düğmesi + istatistik
        egui::TopBottomPanel::bottom("bottom")
            .frame(
                egui::Frame::none()
                    .fill(BG)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .inner_margin(egui::Margin { left: 16.0, right: 16.0, top: 12.0, bottom: 10.0 }),
            )
            .show(ctx, |ui| {
                let err = lk(&self.sh.error).clone();
                if !err.is_empty() {
                    ui.label(RichText::new(err).size(12.5).color(RED));
                }
                let chip = format!("{:?}", *lk(&self.sh.start_key));
                let dark = Color32::from_rgb(4, 36, 26);
                if self.cfg.mode == Mode::Record {
                    if recording {
                        if big_button(ui, "■  Kaydı Bitir", &chip, RED, Color32::WHITE) {
                            self.finish_record(true);
                        }
                    } else if big_button(ui, "●  Kaydı Başlat", &chip, GREEN, dark) {
                        lk(&self.sh.rec).clear();
                        *lk(&self.sh.rec_last) = None;
                        self.sh.recording.store(true, Ordering::Relaxed);
                        self.was_rec = true;
                    }
                } else if running {
                    if big_button(ui, "■  Durdur", &chip, RED, Color32::WHITE) {
                        self.sh.running.store(false, Ordering::Relaxed);
                    }
                } else if big_button(ui, "▶  Başlat", &chip, GREEN, dark) {
                    self.sync_cfg();
                    self.sh.running.store(true, Ordering::Relaxed);
                }

                let clicks = self.sh.clicks.load(Ordering::Relaxed);
                let el = match *lk(&self.sh.started) {
                    Some(t) => t.elapsed().as_millis() as u64,
                    None => self.sh.elapsed_ms.load(Ordering::Relaxed),
                };
                let cps = if el > 0 { clicks as f64 * 1000.0 / el as f64 } else { 0.0 };
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{clicks} tık  •  {:.1} sn  •  {cps:.1}/sn", el as f64 / 1000.0))
                            .monospace()
                            .size(13.0)
                            .color(if running { GREEN } else { TXT }),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("AUTOCLICKER ENGINE").size(11.0).color(Color32::from_rgb(95, 102, 118)));
                    });
                });
            });

        // ---- Orta: içerik
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin { left: 16.0, right: 16.0, top: 4.0, bottom: 4.0 }))
            .show(ctx, |ui| {
                self.frames = self.frames.saturating_add(1);
                let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]);
                if self.frames < 6 {
                    sa = sa.vertical_scroll_offset(0.0);
                }
                sa.show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    if self.settings_open {
                        self.settings_view(ui);
                        return;
                    }
                    ui.add_enabled_ui(!running && !recording, |ui| {
                        self.content(ui);
                    });
                    ui.add_space(4.0);
                });
            });
        self.sync_cfg();
    }
}

impl App {
    fn content(&mut self, ui: &mut egui::Ui) {
        let mode = self.cfg.mode;

        // Aralık (Fare / Klavye)
        if mode == Mode::Mouse || mode == Mode::Keyboard {
            let rate = 1000.0 / self.cfg.interval_ms() as f64;
            card(ui, GREEN, "ARALIK (ZAMAN AŞIMI)", Some(format!("{rate:.0} tık/sn")), |ui| {
                ui.columns(4, |c| {
                    field(&mut c[0], &mut self.cfg.h, 99, "saat", false);
                    field(&mut c[1], &mut self.cfg.m, 59, "dakika", false);
                    field(&mut c[2], &mut self.cfg.s, 59, "saniye", false);
                    field(&mut c[3], &mut self.cfg.ms, 999, "salise (ms)", true);
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new("Hızlı Seçim:").color(DIM));
                    let cur = self.cfg.interval_ms();
                    for (ms, label) in [(1u64, "1\nms"), (20, "20\nms"), (100, "100\nms"), (500, "500\nms"), (1000, "1\nsn")] {
                        let sel = cur == ms;
                        let b = egui::Button::new(RichText::new(label).size(12.0).color(if sel { GREEN } else { DIM }))
                            .fill(if sel { Color32::from_rgb(12, 44, 38) } else { Color32::from_rgb(26, 30, 40) })
                            .stroke(Stroke::new(1.0_f32, if sel { Color32::from_rgb(10, 110, 80) } else { BORDER }))
                            .rounding(9.0);
                        if ui.add_sized([42.0, 38.0], b).clicked() {
                            self.cfg.h = 0;
                            self.cfg.m = 0;
                            self.cfg.s = (ms / 1000) as u32;
                            self.cfg.ms = (ms % 1000) as u32;
                        }
                    }
                });
            });
        }

        match mode {
            Mode::Mouse => card(ui, BLUE, "TIKLAMA PARAMETRELERİ", None, |ui| {
                ui.columns(2, |c| {
                    c[0].label(RichText::new("Fare Düğmesi").color(DIM));
                    seg(&mut c[0], &mut self.cfg.btn, &[(Btn::Left, "Sol"), (Btn::Right, "Sağ"), (Btn::Middle, "Orta")], Tone::Blue);
                    c[1].label(RichText::new("Tıklama Tipi").color(DIM));
                    seg(&mut c[1], &mut self.cfg.double, &[(false, "Tek"), (true, "Çift Tık")], Tone::Gray);
                });
                hold_row(ui, &mut self.cfg.hold_ms);
                ui.separator();
                ui.label(RichText::new("Tıklama Konumu").color(DIM));
                seg(ui, &mut self.cfg.fixed_pos, &[(false, "İmleç Yerinde"), (true, "Sabit Konum")], Tone::Blue);
                if self.cfg.fixed_pos {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.cfg.x).prefix("X "));
                        ui.add(egui::DragValue::new(&mut self.cfg.y).prefix("Y "));
                        let lbl = if self.pick_at.is_some() { "Hedefe götür…" } else { "Seç (3 sn)" };
                        if small_btn(ui, lbl) {
                            self.pick_at = Some((Instant::now() + Duration::from_secs(3), false));
                        }
                    });
                }
            }),
            Mode::Keyboard => card(ui, BLUE, "KLAVYE PARAMETRELERİ", None, |ui| {
                ui.label(RichText::new("Değiştirici tuşlar").color(DIM));
                ui.horizontal(|ui| {
                    let names = if cfg!(target_os = "macos") {
                        ["Ctrl", "Shift", "Opt", "Cmd"]
                    } else {
                        ["Ctrl", "Shift", "Alt", "Win"]
                    };
                    for (i, n) in names.iter().enumerate() {
                        let on = self.cfg.mods[i];
                        let b = egui::Button::new(RichText::new(*n).color(if on { Color32::from_rgb(140, 185, 255) } else { DIM }))
                            .fill(if on { Color32::from_rgb(18, 38, 78) } else { TRACK })
                            .stroke(Stroke::new(1.0_f32, if on { BLUE } else { BORDER }))
                            .rounding(9.0);
                        if ui.add_sized([62.0, 32.0], b).clicked() {
                            self.cfg.mods[i] = !on;
                        }
                    }
                });
                let cur = self.keys.get(self.cfg.key_idx).map(|k| k.0).unwrap_or("?");
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Basılacak tuş").color(DIM));
                    egui::ComboBox::from_id_salt("key").selected_text(cur).show_ui(ui, |ui| {
                        for (i, (n, _)) in self.keys.iter().enumerate() {
                            ui.selectable_value(&mut self.cfg.key_idx, i, *n);
                        }
                    });
                });
                hold_row(ui, &mut self.cfg.key_hold_ms);
            }),
            Mode::Sequence => card(ui, BLUE, "TIKLAMA DİZİSİ", Some(format!("{} adım", self.cfg.steps.len())), |ui| {
                let mut del: Option<usize> = None;
                egui::ScrollArea::vertical().max_height(260.0).auto_shrink([true, true]).show(ui, |ui| {
                    for (i, st) in self.cfg.steps.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{}", i + 1)).monospace().color(DIM));
                            ui.add(egui::DragValue::new(&mut st.x).prefix("x "));
                            ui.add(egui::DragValue::new(&mut st.y).prefix("y "));
                            let name = match st.btn {
                                Btn::Left => "Sol",
                                Btn::Right => "Sağ",
                                Btn::Middle => "Orta",
                            };
                            egui::ComboBox::from_id_salt(("btn", i)).width(44.0).selected_text(name).show_ui(ui, |ui| {
                                ui.selectable_value(&mut st.btn, Btn::Left, "Sol");
                                ui.selectable_value(&mut st.btn, Btn::Right, "Sağ");
                                ui.selectable_value(&mut st.btn, Btn::Middle, "Orta");
                            });
                            ui.add(egui::DragValue::new(&mut st.delay_ms).range(0..=3_600_000).suffix(" ms"));
                            if ui.small_button("✕").clicked() {
                                del = Some(i);
                            }
                        });
                    }
                });
                if let Some(i) = del {
                    self.cfg.steps.remove(i);
                }
                ui.horizontal(|ui| {
                    let lbl = if self.pick_at.is_some() { "İmleci hedefe götür…" } else { "+ Adım ekle (3 sn)" };
                    if small_btn(ui, lbl) {
                        self.pick_at = Some((Instant::now() + Duration::from_secs(3), true));
                    }
                    if !self.cfg.steps.is_empty() && small_btn(ui, "Temizle") {
                        self.cfg.steps.clear();
                    }
                });
                ui.label(RichText::new("Her adım: konuma git, tıkla, ms kadar bekle. Kayıt sekmesinden de oluşturabilirsin.").size(12.5).color(DIM));
            }),
            Mode::Record => card(ui, RED, "TIKLAMA KAYDI", None, |ui| {
                ui.label(RichText::new("Kaydı başlat, sonra tıklamak istediğin yerlere sırayla tıkla. Bitirince adımlar Dizi sekmesine aktarılır; aralarındaki süreler de kaydedilir.").color(DIM));
                ui.label(RichText::new(format!("Kaydedilen: {} tıklama", lk(&self.sh.rec).len())).monospace().color(TXT));
                ui.label(RichText::new("Kaydı kısayol tuşuyla bitirirsen son tıklama silinmez.").size(12.5).color(DIM));
            }),
        }

        // Bitiş kuralı
        if mode != Mode::Record {
            card(ui, AMBER, "BİTİŞ KURALI", None, |ui| {
                seg(
                    ui,
                    &mut self.cfg.rep,
                    &[(Rep::Infinite, "Sonsuz"), (Rep::Count, "Sayı Sınırı"), (Rep::Time, "Zaman")],
                    Tone::Amber,
                );
                match self.cfg.rep {
                    Rep::Count => {
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut self.cfg.repeat).range(1..=u64::MAX));
                            ui.label(RichText::new(if mode == Mode::Sequence { "döngü" } else { "tıklama" }).color(DIM));
                        });
                    }
                    Rep::Time => {
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut self.cfg.limit_min).range(0..=999).suffix(" dk"));
                            ui.add(egui::DragValue::new(&mut self.cfg.limit_sec).range(0..=59).suffix(" sn"));
                        });
                    }
                    Rep::Infinite => {}
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("🔔").color(TXT));
                    ui.label(RichText::new("Bitince ses ve sistem bildirimi ver").color(TXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.cfg.notify, "");
                    });
                });
            });
        }
    }
}

fn setup_tray(sh: Arc<Shared>, ctx: egui::Context) -> Option<tray_icon::TrayIcon> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    let img = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?;
    let img = img.resize(44, 44, image::imageops::FilterType::Lanczos3).to_rgba8();
    let (w, h) = img.dimensions();
    let icon = tray_icon::Icon::from_rgba(img.into_raw(), w, h).ok()?;

    let toggle = MenuItem::new("Başlat / Durdur", true, None);
    let show = MenuItem::new("Pencereyi göster", true, None);
    let quit = MenuItem::new("Çık", true, None);
    let menu = Menu::new();
    menu.append_items(&[&toggle, &show, &PredefinedMenuItem::separator(), &quit]).ok()?;
    let (tid, sid, qid) = (toggle.id().clone(), show.id().clone(), quit.id().clone());

    MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
        if ev.id == tid {
            request_run(&sh, None);
            ctx.request_repaint();
        } else if ev.id == sid {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint();
        } else if ev.id == qid {
            sh.running.store(false, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(100));
            std::process::exit(0);
        }
    }));

    tray_icon::TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .with_tooltip("AutoClicker")
        .build()
        .ok()
}

fn main() -> eframe::Result {
    let sh = Arc::new(Shared {
        cfg: Mutex::new(Config::default()),
        running: AtomicBool::new(false),
        clicks: AtomicU64::new(0),
        started: Mutex::new(None),
        elapsed_ms: AtomicU64::new(0),
        start_key: Mutex::new(Keycode::F6),
        stop_key: Mutex::new(Keycode::F6),
        capturing: AtomicU8::new(0),
        pos: Mutex::new((-1, -1)),
        error: Mutex::new(String::new()),
        recording: AtomicBool::new(false),
        rec: Mutex::new(vec![]),
        rec_last: Mutex::new(None),
    });
    {
        let s = sh.clone();
        supervised(s, "Motor", engine_run);
        let s = sh.clone();
        supervised(s, "Kısayol dinleyici", hotkey_thread);
    }
    let upd = Arc::new(Mutex::new(UpdateState::default()));
    check_update(upd.clone());
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AutoClicker")
            .with_inner_size([400.0, 830.0])
            .with_min_inner_size([400.0, 640.0])
            .with_always_on_top()
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).expect("icon")),
        ..Default::default()
    };
    eframe::run_native(
        "AutoClicker",
        opts,
        Box::new(move |cc| {
            apply_style(&cc.egui_ctx);
            let icon = image::load_from_memory(include_bytes!("../assets/icon.png")).ok().map(|im| {
                let im = im.resize(64, 64, image::imageops::FilterType::Lanczos3).to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([64, 64], im.as_raw());
                cc.egui_ctx.load_texture("icon", ci, egui::TextureOptions::LINEAR)
            });
            let _tray = setup_tray(sh.clone(), cc.egui_ctx.clone());
            Ok(Box::new(App {
                _tray,
                sh,
                cfg: Config::default(),
                keys: key_list(),
                pick_at: None,
                upd,
                icon,
                settings_open: false,
                pinned: true,
                was_rec: false,
                frames: 0,
            }))
        }),
    )
}
