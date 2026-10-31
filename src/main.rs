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

#[derive(Clone)]
struct Step {
    x: i32,
    y: i32,
    btn: Btn,
    delay_ms: u32,
}

#[derive(Clone)]
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
    const LETTERS: &str = "abcdefghijklmnopqrstuvwxyz0123456789";
    for c in LETTERS.chars() {
        let name: &'static str = Box::leak(c.to_string().into_boxed_str());
        v.push((name, Key::Unicode(c)));
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
    for _ in 0..(if double { 2 } else { 1 }) {
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
        let b = to_button(cfg.btn);
        let failsafe = |sh: &Shared| -> bool {
            // sabit konumda değilken imleç sol üst köşedeyse acil durdur
            let (x, y) = *lk(&sh.pos);
            if x <= 0 && y <= 0 {
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
                for st in &cfg.steps {
                    if !sh.running.load(Ordering::Relaxed) {
                        break 'outer;
                    }
                    let _ = enigo.move_mouse(st.x, st.y, Coordinate::Abs);
                    click_btn(enigo, &sh, &sleeper, to_button(st.btn), false, 0);
                    count += 1;
                    sh.clicks.store(count, Ordering::Relaxed);
                    sleep_until(&sh, &sleeper, Instant::now() + Duration::from_millis(st.delay_ms as u64), deadline);
                }
                loops += 1;
                if cfg.rep == Rep::Count && loops >= cfg.repeat {
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
                            if cfg.hold_ms > 0 {
                                let _ = enigo.key(*k, Direction::Press);
                                sleep_until(&sh, &sleeper, Instant::now() + Duration::from_millis(cfg.hold_ms as u64), None);
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
                    Mode::Sequence => {}
                }
                count += 1;
                sh.clicks.store(count, Ordering::Relaxed);
                if cfg.rep == Rep::Count && count >= cfg.repeat {
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
        if cfg.notify && count > 0 {
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
        *lk(&sh.error) = format!("{name} beklenmedik şekilde durdu, yeniden başlatıldı.");
        std::thread::sleep(Duration::from_millis(500));
    });
}

fn hotkey_thread(sh: Arc<Shared>) {
    let ds = DeviceState::new();
    let mut prev: Vec<Keycode> = vec![];
    loop {
        std::thread::sleep(Duration::from_millis(8));
        let keys = ds.get_keys();
        let pressed: Vec<Keycode> = keys.iter().filter(|k| !prev.contains(k)).cloned().collect();
        let (mx, my) = ds.get_mouse().coords;
        *lk(&sh.pos) = (mx, my);
        if !pressed.is_empty() {
            let cap = sh.capturing.load(Ordering::Relaxed);
            if cap != 0 {
                let k = pressed[0];
                if cap == 1 {
                    *lk(&sh.start_key) = k;
                } else {
                    *lk(&sh.stop_key) = k;
                }
                sh.capturing.store(0, Ordering::Relaxed);
            } else {
                let sk = *lk(&sh.start_key);
                let tk = *lk(&sh.stop_key);
                for k in &pressed {
                    if sk == tk && *k == sk {
                        let r = sh.running.load(Ordering::Relaxed);
                        sh.running.store(!r, Ordering::Relaxed);
                    } else if *k == sk {
                        sh.running.store(true, Ordering::Relaxed);
                    } else if *k == tk {
                        sh.running.store(false, Ordering::Relaxed);
                    }
                }
            }
        }
        prev = keys;
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "enesbilen/autoclicker";
const ACCENT: egui::Color32 = egui::Color32::from_rgb(79, 140, 255);
const GREEN: egui::Color32 = egui::Color32::from_rgb(46, 160, 90);
const RED: egui::Color32 = egui::Color32::from_rgb(214, 69, 65);

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
        let res = (|| -> Result<std::path::PathBuf, String> {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .map_err(|e| e.to_string())?;
            let path = std::path::Path::new(&home).join("Downloads").join(&u.name);
            let resp = ureq::get(&u.url).set("User-Agent", "autoclicker").call().map_err(|e| e.to_string())?;
            let mut f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
            std::io::copy(&mut resp.into_reader(), &mut f).map_err(|e| e.to_string())?;
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
            Err(e) => format!("Hata: {e}"),
        };
        lk(&st).status = msg;
    });
}

fn card(ui: &mut egui::Ui, title: &str, f: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(egui::Color32::from_gray(34))
        .rounding(10.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(title.to_uppercase()).size(10.5).weak());
            ui.add_space(2.0);
            f(ui);
        });
}

fn seg<T: PartialEq + Copy>(ui: &mut egui::Ui, val: &mut T, items: &[(T, &str)]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (v, label) in items {
            let sel = *val == *v;
            let b = egui::Button::new(egui::RichText::new(*label).color(if sel {
                egui::Color32::WHITE
            } else {
                egui::Color32::from_gray(190)
            }))
            .fill(if sel { ACCENT } else { egui::Color32::from_gray(48) })
            .rounding(6.0)
            .min_size([0.0, 26.0].into());
            if ui.add(b).clicked() {
                *val = *v;
            }
        }
    });
}

struct App {
    sh: Arc<Shared>,
    cfg: Config,
    keys: Vec<(&'static str, Key)>,
    pick_at: Option<(Instant, bool)>, // (zaman, yeni adım mı)
    upd: Arc<Mutex<UpdateState>>,
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.sh.running.store(false, Ordering::Relaxed);
    }

    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(50));
        let running = self.sh.running.load(Ordering::Relaxed);
        let cap = self.sh.capturing.load(Ordering::Relaxed);

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

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(egui::Color32::from_gray(22)).inner_margin(14.0))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                let upd = lk(&self.upd).available.clone();
                if let Some(u) = upd {
                    let status = lk(&self.upd).status.clone();
                    egui::Frame::none()
                        .fill(ACCENT.gamma_multiply(0.25))
                        .rounding(8.0)
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(format!("Yeni sürüm: {}", u.version));
                                if status.is_empty() && ui.button("Güncelle").clicked() {
                                    download_update(u.clone(), self.upd.clone());
                                }
                            });
                            if !status.is_empty() {
                                ui.label(egui::RichText::new(status).small());
                            }
                        });
                }

                ui.add_enabled_ui(!running, |ui| {
                    if self.cfg.mode != Mode::Sequence {
                    card(ui, "Aralık", |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            let field = |ui: &mut egui::Ui, v: &mut u32, max: u32, lbl: &str| {
                                ui.vertical(|ui| {
                                    ui.add_sized([70.0, 30.0], egui::DragValue::new(v).range(0..=max));
                                    ui.label(egui::RichText::new(lbl).small().weak());
                                });
                            };
                            field(ui, &mut self.cfg.h, 99, "saat");
                            field(ui, &mut self.cfg.m, 59, "dakika");
                            field(ui, &mut self.cfg.s, 59, "saniye");
                            field(ui, &mut self.cfg.ms, 999, "salise (ms)");
                        });
                    });
                    }

                    card(ui, "Tıklama", |ui| {
                        seg(ui, &mut self.cfg.mode, &[(Mode::Mouse, "Fare"), (Mode::Keyboard, "Klavye"), (Mode::Sequence, "Dizi")]);
                        match self.cfg.mode {
                            Mode::Mouse => {
                                ui.horizontal(|ui| {
                                    seg(ui, &mut self.cfg.btn, &[(Btn::Left, "Sol"), (Btn::Right, "Sağ"), (Btn::Middle, "Orta")]);
                                    ui.checkbox(&mut self.cfg.double, "Çift");
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Basılı tut");
                                    ui.add(egui::DragValue::new(&mut self.cfg.hold_ms).range(0..=60000).suffix(" ms"));
                                    ui.label(egui::RichText::new("0 = normal tık").small().weak());
                                });
                                seg(ui, &mut self.cfg.fixed_pos, &[(false, "İmleç yerinde"), (true, "Sabit konum")]);
                                if self.cfg.fixed_pos {
                                    ui.horizontal(|ui| {
                                        ui.add(egui::DragValue::new(&mut self.cfg.x).prefix("X "));
                                        ui.add(egui::DragValue::new(&mut self.cfg.y).prefix("Y "));
                                        let lbl = if self.pick_at.is_some() { "Hedefe götür…" } else { "Seç (3 sn)" };
                                        if ui.button(lbl).clicked() {
                                            self.pick_at = Some((Instant::now() + Duration::from_secs(3), false));
                                        }
                                    });
                                }
                            }
                            Mode::Sequence => {
                                let mut del: Option<usize> = None;
                                egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                                    for (i, st) in self.cfg.steps.iter_mut().enumerate() {
                                        ui.horizontal(|ui| {
                                            ui.label(format!("{}", i + 1));
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
                                    if ui.button(lbl).clicked() {
                                        self.pick_at = Some((Instant::now() + Duration::from_secs(3), true));
                                    }
                                    if !self.cfg.steps.is_empty() && ui.button("Temizle").clicked() {
                                        self.cfg.steps.clear();
                                    }
                                });
                                ui.label(egui::RichText::new("Her adım: konuma git, tıkla, ms kadar bekle.").small().weak());
                            }
                            Mode::Keyboard => {
                                let cur = self.keys.get(self.cfg.key_idx).map(|k| k.0).unwrap_or("?");
                                ui.horizontal(|ui| {
                                    let names = if cfg!(target_os = "macos") {
                                        ["Ctrl", "Shift", "Opt", "Cmd"]
                                    } else {
                                        ["Ctrl", "Shift", "Alt", "Win"]
                                    };
                                    for (i, n) in names.iter().enumerate() {
                                        ui.checkbox(&mut self.cfg.mods[i], *n);
                                    }
                                });
                                ui.horizontal(|ui| {
                                    egui::ComboBox::from_id_salt("key")
                                        .selected_text(format!("Tuş: {cur}"))
                                        .show_ui(ui, |ui| {
                                            for (i, (n, _)) in self.keys.iter().enumerate() {
                                                ui.selectable_value(&mut self.cfg.key_idx, i, *n);
                                            }
                                        });
                                    ui.add(egui::DragValue::new(&mut self.cfg.hold_ms).range(0..=60000).prefix("basılı ").suffix(" ms"));
                                });
                            }
                        }
                    });

                    card(ui, "Bitiş", |ui| {
                        ui.horizontal(|ui| {
                            seg(ui, &mut self.cfg.rep, &[(Rep::Infinite, "Sonsuz"), (Rep::Count, "Sayı"), (Rep::Time, "Süre")]);
                            match self.cfg.rep {
                                Rep::Count => {
                                    ui.add(egui::DragValue::new(&mut self.cfg.repeat).range(1..=u64::MAX));
                                }
                                Rep::Time => {
                                    ui.add(egui::DragValue::new(&mut self.cfg.limit_min).range(0..=999).suffix(" dk"));
                                    ui.add(egui::DragValue::new(&mut self.cfg.limit_sec).range(0..=59).suffix(" sn"));
                                }
                                Rep::Infinite => {}
                            }
                        });
                        ui.checkbox(&mut self.cfg.notify, "Bitince ses / bildirim");
                    });
                });

                card(ui, "Kısayollar", |ui| {
                    let sk = format!("{:?}", *lk(&self.sh.start_key));
                    let tk = format!("{:?}", *lk(&self.sh.stop_key));
                    ui.horizontal(|ui| {
                        ui.label("Başlat");
                        let t = if cap == 1 { "tuşa bas…".to_string() } else { sk };
                        if ui.button(t).clicked() {
                            self.sh.capturing.store(1, Ordering::Relaxed);
                        }
                        ui.add_space(8.0);
                        ui.label("Durdur");
                        let t = if cap == 2 { "tuşa bas…".to_string() } else { tk };
                        if ui.button(t).clicked() {
                            self.sh.capturing.store(2, Ordering::Relaxed);
                        }
                    });
                    if self.cfg.mode == Mode::Mouse && !self.cfg.fixed_pos {
                        ui.label(egui::RichText::new("Acil durdurma: imleci ekranın sol üst köşesine götür.").small().weak());
                    }
                });

                if !running {
                    *lk(&self.sh.cfg) = self.cfg.clone();
                }
                let err = lk(&self.sh.error).clone();
                if !err.is_empty() {
                    ui.colored_label(RED, err);
                }
                let (txt, col) = if running { ("■  Durdur", RED) } else { ("▶  Başlat", GREEN) };
                let b = egui::Button::new(egui::RichText::new(txt).size(16.0).strong().color(egui::Color32::WHITE))
                    .fill(col)
                    .rounding(10.0);
                if ui.add_sized([ui.available_width(), 44.0], b).clicked() {
                    self.sh.running.store(!running, Ordering::Relaxed);
                }

                let clicks = self.sh.clicks.load(Ordering::Relaxed);
                let el = match *lk(&self.sh.started) {
                    Some(t) => t.elapsed().as_millis() as u64,
                    None => self.sh.elapsed_ms.load(Ordering::Relaxed),
                };
                let cps = if el > 0 { clicks as f64 * 1000.0 / el as f64 } else { 0.0 };
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{clicks} tık  ·  {:.1} sn  ·  {cps:.1}/sn", el as f64 / 1000.0))
                            .color(if running { ACCENT } else { egui::Color32::GRAY }),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(format!("v{VERSION}")).small().weak());
                    });
                });
            });
    }
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
        pos: Mutex::new((1, 1)),
        error: Mutex::new(String::new()),
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
            .with_inner_size([380.0, 600.0])
            .with_resizable(false)
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).expect("icon"))
            .with_always_on_top(),
        ..Default::default()
    };
    eframe::run_native(
        "Auto Clicker",
        opts,
        Box::new(move |cc| {
            let mut style = (*cc.egui_ctx.style()).clone();
            style.visuals.selection.bg_fill = ACCENT;
            style.visuals.widgets.inactive.rounding = 6.0.into();
            style.visuals.widgets.hovered.rounding = 6.0.into();
            style.visuals.widgets.active.rounding = 6.0.into();
            style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_gray(48);
            style.spacing.button_padding = [10.0, 4.0].into();
            cc.egui_ctx.set_style(style);
            Ok(Box::new(App {
                sh,
                cfg: Config::default(),
                keys: key_list(),
                pick_at: None,
                upd,
            }))
        }),
    )
}
