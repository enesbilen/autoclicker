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
}

#[derive(Clone, Copy, PartialEq)]
enum Btn {
    Left,
    Right,
    Middle,
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
    infinite: bool,
    repeat: u64,
    key_idx: usize,
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
            infinite: true,
            repeat: 100,
            key_idx: 0,
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

fn engine_thread(sh: Arc<Shared>) {
    let mut enigo = match Enigo::new(&Settings::default()) {
        Ok(e) => e,
        Err(_) => return,
    };
    let keys = key_list();
    let sleeper = spin_sleep::SpinSleeper::default();
    loop {
        if !sh.running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let cfg = sh.cfg.lock().unwrap().clone();
        let interval = Duration::from_millis(cfg.interval_ms());
        sh.clicks.store(0, Ordering::Relaxed);
        let t0 = Instant::now();
        *sh.started.lock().unwrap() = Some(t0);
        let mut next = t0;
        let mut count: u64 = 0;
        while sh.running.load(Ordering::Relaxed) {
            // hedef zamana kadar bekle (uzun beklemelerde dilimlere böl)
            loop {
                let now = Instant::now();
                if now >= next || !sh.running.load(Ordering::Relaxed) {
                    break;
                }
                sleeper.sleep((next - now).min(Duration::from_millis(20)));
            }
            if !sh.running.load(Ordering::Relaxed) {
                break;
            }
            match cfg.mode {
                Mode::Mouse => {
                    if cfg.fixed_pos {
                        let _ = enigo.move_mouse(cfg.x, cfg.y, Coordinate::Abs);
                    }
                    let b = match cfg.btn {
                        Btn::Left => Button::Left,
                        Btn::Right => Button::Right,
                        Btn::Middle => Button::Middle,
                    };
                    let _ = enigo.button(b, Direction::Click);
                    if cfg.double {
                        let _ = enigo.button(b, Direction::Click);
                    }
                }
                Mode::Keyboard => {
                    if let Some((_, k)) = keys.get(cfg.key_idx) {
                        let _ = enigo.key(*k, Direction::Click);
                    }
                }
            }
            count += 1;
            sh.clicks.store(count, Ordering::Relaxed);
            if !cfg.infinite && count >= cfg.repeat {
                sh.running.store(false, Ordering::Relaxed);
                break;
            }
            next += interval;
            let now = Instant::now();
            if next < now {
                next = now; // geride kaldıysak yığılma yapma
            }
        }
        sh.elapsed_ms
            .store(t0.elapsed().as_millis() as u64, Ordering::Relaxed);
        *sh.started.lock().unwrap() = None;
    }
}

fn hotkey_thread(sh: Arc<Shared>) {
    let ds = DeviceState::new();
    let mut prev: Vec<Keycode> = vec![];
    loop {
        std::thread::sleep(Duration::from_millis(8));
        let keys = ds.get_keys();
        let pressed: Vec<Keycode> = keys.iter().filter(|k| !prev.contains(k)).cloned().collect();
        let (mx, my) = ds.get_mouse().coords;
        *sh.pos.lock().unwrap() = (mx, my);
        if !pressed.is_empty() {
            let cap = sh.capturing.load(Ordering::Relaxed);
            if cap != 0 {
                let k = pressed[0];
                if cap == 1 {
                    *sh.start_key.lock().unwrap() = k;
                } else {
                    *sh.stop_key.lock().unwrap() = k;
                }
                sh.capturing.store(0, Ordering::Relaxed);
            } else {
                let sk = *sh.start_key.lock().unwrap();
                let tk = *sh.stop_key.lock().unwrap();
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

struct App {
    sh: Arc<Shared>,
    cfg: Config,
    keys: Vec<(&'static str, Key)>,
    pick_at: Option<Instant>,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(50));
        let running = self.sh.running.load(Ordering::Relaxed);
        let cap = self.sh.capturing.load(Ordering::Relaxed);

        if let Some(t) = self.pick_at {
            if Instant::now() >= t {
                let (x, y) = *self.sh.pos.lock().unwrap();
                self.cfg.x = x;
                self.cfg.y = y;
                self.cfg.fixed_pos = true;
                self.pick_at = None;
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(!running, |ui| {
                ui.heading("Tıklama Aralığı");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.cfg.h).range(0..=99).suffix(" saat"));
                    ui.add(egui::DragValue::new(&mut self.cfg.m).range(0..=59).suffix(" dk"));
                    ui.add(egui::DragValue::new(&mut self.cfg.s).range(0..=59).suffix(" sn"));
                    ui.add(egui::DragValue::new(&mut self.cfg.ms).range(0..=999).suffix(" salise(ms)"));
                });
                if self.cfg.interval_ms() == 1 {
                    ui.weak("En düşük aralık 1 ms");
                }
                ui.separator();

                ui.heading("Tıklama Seçenekleri");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.cfg.mode, Mode::Mouse, "Fare");
                    ui.radio_value(&mut self.cfg.mode, Mode::Keyboard, "Klavye");
                });
                match self.cfg.mode {
                    Mode::Mouse => {
                        ui.horizontal(|ui| {
                            ui.label("Düğme:");
                            ui.radio_value(&mut self.cfg.btn, Btn::Left, "Sol");
                            ui.radio_value(&mut self.cfg.btn, Btn::Right, "Sağ");
                            ui.radio_value(&mut self.cfg.btn, Btn::Middle, "Orta");
                            ui.checkbox(&mut self.cfg.double, "Çift tık");
                        });
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.cfg.fixed_pos, false, "İmleç neredeyse");
                            ui.radio_value(&mut self.cfg.fixed_pos, true, "Sabit konum");
                        });
                        if self.cfg.fixed_pos {
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(&mut self.cfg.x).prefix("X: "));
                                ui.add(egui::DragValue::new(&mut self.cfg.y).prefix("Y: "));
                                let lbl = if self.pick_at.is_some() {
                                    "İmleci hedefe götür..."
                                } else {
                                    "Konum seç (3 sn)"
                                };
                                if ui.button(lbl).clicked() {
                                    self.pick_at = Some(Instant::now() + Duration::from_secs(3));
                                }
                            });
                        }
                    }
                    Mode::Keyboard => {
                        let cur = self.keys.get(self.cfg.key_idx).map(|k| k.0).unwrap_or("?");
                        egui::ComboBox::from_label("Basılacak tuş")
                            .selected_text(cur)
                            .show_ui(ui, |ui| {
                                for (i, (n, _)) in self.keys.iter().enumerate() {
                                    ui.selectable_value(&mut self.cfg.key_idx, i, *n);
                                }
                            });
                    }
                }
                ui.separator();

                ui.heading("Tekrar");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.cfg.infinite, true, "Durdurulana kadar");
                    ui.radio_value(&mut self.cfg.infinite, false, "Sayı:");
                    ui.add_enabled(
                        !self.cfg.infinite,
                        egui::DragValue::new(&mut self.cfg.repeat).range(1..=u64::MAX),
                    );
                });
            });
            ui.separator();

            ui.heading("Kısayol Tuşları");
            let sk = format!("{:?}", *self.sh.start_key.lock().unwrap());
            let tk = format!("{:?}", *self.sh.stop_key.lock().unwrap());
            ui.horizontal(|ui| {
                ui.label("Başlat:");
                let t = if cap == 1 { "Bir tuşa bas...".to_string() } else { sk };
                if ui.button(t).clicked() {
                    self.sh.capturing.store(1, Ordering::Relaxed);
                }
                ui.label("Durdur:");
                let t = if cap == 2 { "Bir tuşa bas...".to_string() } else { tk };
                if ui.button(t).clicked() {
                    self.sh.capturing.store(2, Ordering::Relaxed);
                }
            });
            ui.weak("Başlat ve durdur aynı tuşsa aç/kapa olarak çalışır.");
            ui.separator();

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!running, egui::Button::new("▶ Başlat").min_size([110.0, 30.0].into()))
                    .clicked()
                {
                    *self.sh.cfg.lock().unwrap() = self.cfg.clone();
                    self.sh.running.store(true, Ordering::Relaxed);
                }
                if ui
                    .add_enabled(running, egui::Button::new("■ Durdur").min_size([110.0, 30.0].into()))
                    .clicked()
                {
                    self.sh.running.store(false, Ordering::Relaxed);
                }
            });
            // kısayolla başlatıldığında da güncel ayarlar kullanılsın
            if !running {
                *self.sh.cfg.lock().unwrap() = self.cfg.clone();
            }
            ui.separator();

            ui.heading("Tıklama Raporu");
            let clicks = self.sh.clicks.load(Ordering::Relaxed);
            let el = match *self.sh.started.lock().unwrap() {
                Some(t) => t.elapsed().as_millis() as u64,
                None => self.sh.elapsed_ms.load(Ordering::Relaxed),
            };
            let cps = if el > 0 { clicks as f64 * 1000.0 / el as f64 } else { 0.0 };
            ui.label(format!("Durum: {}", if running { "Çalışıyor" } else { "Durdu" }));
            ui.label(format!("Toplam tıklama: {clicks}"));
            ui.label(format!("Süre: {:.2} sn", el as f64 / 1000.0));
            ui.label(format!("Ortalama: {cps:.2} tık/sn"));
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
        pos: Mutex::new((0, 0)),
    });
    {
        let s = sh.clone();
        std::thread::spawn(move || engine_thread(s));
        let s = sh.clone();
        std::thread::spawn(move || hotkey_thread(s));
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 640.0])
            .with_always_on_top(),
        ..Default::default()
    };
    eframe::run_native(
        "Auto Clicker",
        opts,
        Box::new(move |_| {
            Ok(Box::new(App {
                sh,
                cfg: Config::default(),
                keys: key_list(),
                pick_at: None,
            }))
        }),
    )
}
