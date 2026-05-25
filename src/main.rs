use eframe::egui;
use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, Vec2};
use std::fs::{File, OpenOptions};
#[cfg(target_os = "linux")]
use std::io::Read;
use std::io::{self, ErrorKind, Write};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const ROWS: usize = 10;
const COLS: usize = 7;
const MAX_LAYERS: usize = 10;
#[cfg(target_os = "windows")]
const USB_VENDOR_ID: u16 = 0xfc32;
#[cfg(target_os = "windows")]
const USB_PRODUCT_ID: u16 = 0x0287;
#[cfg(target_os = "linux")]
const VID_PID_MARKER: &str = "v0000FC32p00000287";

const ID_GET_PROTOCOL_VERSION: u8 = 0x01;
const ID_GET_KEYBOARD_VALUE: u8 = 0x02;
const ID_DYNAMIC_KEYMAP_GET_KEYCODE: u8 = 0x04;
const ID_DYNAMIC_KEYMAP_GET_LAYER_COUNT: u8 = 0x11;
const ID_LIGHTING_GET_VALUE: u8 = 0x08;
const ID_SWITCH_MATRIX_STATE: u8 = 0x03;
const VIALRGB_GET_OLED_CONFIG: u8 = 0x52;
const MATRIX_POLL_INTERVAL: Duration = Duration::from_millis(100);
const TELEMETRY_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
const KEY_FLASH_DURATION: Duration = Duration::from_millis(75);
const MINIMAL_VIEW_WIDTH: f32 = 900.0;
const MINIMAL_AUTOFIT_DELAY: Duration = Duration::from_millis(250);
const TOUCHPAD_VIEW_WIDTH: f32 = 1050.0;
const TOUCHPAD_VIEW_HEIGHT: f32 = 360.0;
const KEYBOARD_LOGICAL_WIDTH: f32 = 16.9;
const KEYBOARD_LOGICAL_HEIGHT: f32 = 6.15;
const SOFLE_TELEMETRY_COMMAND: u8 = 0x7a;
const SOFLE_TELEMETRY_ENABLE: u8 = 0x01;
const SOFLE_TELEMETRY_ACK: u8 = 0x02;
const SOFLE_TELEMETRY_HEARTBEAT: u8 = 0x03;
const SOFLE_TELEMETRY_EVENT: u8 = 0x10;
const SOFLE_TELEMETRY_ENABLED: u8 = 0x81;
const SOFLE_TELEMETRY_ACKED: u8 = 0x82;
const SOFLE_TELEMETRY_HEARTBEAT_RESPONSE: u8 = 0x83;

const KC_NO: u16 = 0x0000;
const KC_TRANSPARENT: u16 = 0x0001;
const QK_MODS: u16 = 0x0100;
const QK_MODS_MAX: u16 = 0x1fff;
const QK_MOD_TAP: u16 = 0x2000;
const QK_MOD_TAP_MAX: u16 = 0x3fff;
const QK_LAYER_TAP: u16 = 0x4000;
const QK_LAYER_TAP_MAX: u16 = 0x4fff;
const QK_LAYER_MOD: u16 = 0x5000;
const QK_LAYER_MOD_MAX: u16 = 0x51ff;
const QK_TO: u16 = 0x5200;
const QK_TO_MAX: u16 = 0x521f;
const QK_MOMENTARY: u16 = 0x5220;
const QK_MOMENTARY_MAX: u16 = 0x523f;
const QK_DEF_LAYER: u16 = 0x5240;
const QK_DEF_LAYER_MAX: u16 = 0x525f;
const QK_TOGGLE_LAYER: u16 = 0x5260;
const QK_TOGGLE_LAYER_MAX: u16 = 0x527f;
const QK_ONE_SHOT_LAYER: u16 = 0x5280;
const QK_ONE_SHOT_LAYER_MAX: u16 = 0x529f;
const QK_LAYER_TAP_TOGGLE: u16 = 0x52c0;
const QK_LAYER_TAP_TOGGLE_MAX: u16 = 0x52df;
const QK_PERSISTENT_DEF_LAYER: u16 = 0x52e0;
const QK_PERSISTENT_DEF_LAYER_MAX: u16 = 0x52ff;
const QK_KB_0: u16 = 0x7e00;

type Matrix = [[bool; COLS]; ROWS];
type Keycodes = [[[u16; COLS]; ROWS]; MAX_LAYERS];

static TRACE_LOG: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static TRACE_START: OnceLock<Instant> = OnceLock::new();

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct TouchContact {
    active: bool,
    x: u8,
    y: u8,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct TouchpadTelemetry {
    enabled: bool,
    ptp_mode: bool,
    contacts: [TouchContact; 5],
}

#[derive(Clone, Copy, Default)]
struct TouchpadState {
    telemetry: TouchpadTelemetry,
    last_update: Option<Instant>,
}

#[derive(Clone)]
struct LiveState {
    connected: bool,
    device_path: String,
    status: String,
    layer_count: usize,
    layer_names: [String; MAX_LAYERS],
    active_layer: Option<usize>,
    keycodes: Keycodes,
    matrix: Matrix,
    visible_until: [[Option<Instant>; COLS]; ROWS],
    pressed_since: [[Option<Instant>; COLS]; ROWS],
    last_update: Option<Instant>,
    touchpad: TouchpadState,
    error: Option<String>,
    keymap_reload_requested: bool,
}

impl Default for LiveState {
    fn default() -> Self {
        Self {
            connected: false,
            device_path: String::new(),
            status: "Scanning for Sofle Plus 2".to_owned(),
            layer_count: MAX_LAYERS,
            layer_names: std::array::from_fn(|i| format!("L{i}")),
            active_layer: None,
            keycodes: [[[KC_NO; COLS]; ROWS]; MAX_LAYERS],
            matrix: [[false; COLS]; ROWS],
            visible_until: [[None; COLS]; ROWS],
            pressed_since: [[None; COLS]; ROWS],
            last_update: None,
            touchpad: TouchpadState::default(),
            error: None,
            keymap_reload_requested: false,
        }
    }
}

#[derive(Clone, Copy)]
struct KeyDef {
    row: usize,
    col: usize,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl KeyDef {
    const fn new(row: usize, col: usize, x: f32, y: f32) -> Self {
        Self {
            row,
            col,
            x,
            y,
            w: 1.0,
            h: 1.0,
        }
    }

    const fn tall(row: usize, col: usize, x: f32, y: f32) -> Self {
        Self {
            row,
            col,
            x,
            y,
            w: 1.0,
            h: 1.5,
        }
    }

    const fn sized(row: usize, col: usize, x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            row,
            col,
            x,
            y,
            w,
            h,
        }
    }
}

const KEYS: &[KeyDef] = &[
    KeyDef::new(0, 0, 0.0, 0.5),
    KeyDef::new(0, 1, 1.0, 0.375),
    KeyDef::new(0, 2, 2.0, 0.125),
    KeyDef::new(0, 3, 3.0, 0.0),
    KeyDef::new(0, 4, 4.0, 0.125),
    KeyDef::new(0, 5, 5.0, 0.25),
    KeyDef::new(5, 5, 10.5, 0.25),
    KeyDef::new(5, 4, 11.5, 0.125),
    KeyDef::new(5, 3, 12.5, 0.0),
    KeyDef::new(5, 2, 13.5, 0.125),
    KeyDef::new(5, 1, 14.5, 0.375),
    KeyDef::new(5, 0, 15.5, 0.5),
    KeyDef::new(1, 0, 0.0, 1.5),
    KeyDef::new(1, 1, 1.0, 1.375),
    KeyDef::new(1, 2, 2.0, 1.125),
    KeyDef::new(1, 3, 3.0, 1.0),
    KeyDef::new(1, 4, 4.0, 1.125),
    KeyDef::new(1, 5, 5.0, 1.25),
    KeyDef::new(6, 5, 10.5, 1.25),
    KeyDef::new(6, 4, 11.5, 1.125),
    KeyDef::new(6, 3, 12.5, 1.0),
    KeyDef::new(6, 2, 13.5, 1.125),
    KeyDef::new(6, 1, 14.5, 1.375),
    KeyDef::new(6, 0, 15.5, 1.5),
    KeyDef::new(2, 0, 0.0, 2.5),
    KeyDef::new(2, 1, 1.0, 2.375),
    KeyDef::new(2, 2, 2.0, 2.125),
    KeyDef::new(2, 3, 3.0, 2.0),
    KeyDef::new(2, 4, 4.0, 2.125),
    KeyDef::new(2, 5, 5.0, 2.25),
    KeyDef::new(7, 5, 10.5, 2.25),
    KeyDef::new(7, 4, 11.5, 2.125),
    KeyDef::new(7, 3, 12.5, 2.0),
    KeyDef::new(7, 2, 13.5, 2.125),
    KeyDef::new(7, 1, 14.5, 2.375),
    KeyDef::new(7, 0, 15.5, 2.5),
    KeyDef::new(3, 0, 0.0, 3.5),
    KeyDef::new(3, 1, 1.0, 3.375),
    KeyDef::new(3, 2, 2.0, 3.125),
    KeyDef::new(3, 3, 3.0, 3.0),
    KeyDef::new(3, 4, 4.0, 3.125),
    KeyDef::new(3, 5, 5.0, 3.25),
    KeyDef::new(4, 5, 6.0, 2.75),
    KeyDef::new(9, 5, 9.5, 2.75),
    KeyDef::new(8, 5, 10.5, 3.25),
    KeyDef::new(8, 4, 11.5, 3.125),
    KeyDef::new(8, 3, 12.5, 3.0),
    KeyDef::new(8, 2, 13.5, 3.125),
    KeyDef::new(8, 1, 14.5, 3.375),
    KeyDef::new(8, 0, 15.5, 3.5),
    KeyDef::new(4, 0, 1.5, 4.375),
    KeyDef::new(4, 1, 2.5, 4.125),
    KeyDef::new(4, 2, 3.5, 4.15),
    KeyDef::new(4, 3, 4.5, 4.25),
    KeyDef::tall(4, 4, 6.0, 4.25),
    KeyDef::tall(9, 4, 9.5, 4.25),
    KeyDef::new(9, 3, 11.0, 4.25),
    KeyDef::new(9, 2, 12.0, 4.15),
    KeyDef::new(9, 1, 13.0, 4.125),
    KeyDef::new(9, 0, 14.0, 4.375),
    KeyDef::sized(0, 6, 7.58, 3.62, 0.52, 0.58),
    KeyDef::sized(1, 6, 8.16, 3.04, 0.52, 0.58),
    KeyDef::sized(2, 6, 8.74, 3.62, 0.52, 0.58),
    KeyDef::sized(3, 6, 8.16, 3.62, 0.52, 0.58),
    KeyDef::sized(4, 6, 8.16, 4.20, 0.52, 0.58),
];

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    init_trace_log(
        args.iter().any(|arg| arg == "--log-events")
            || std::env::var("SOFLE_VIEWER_TRACE").as_deref() == Ok("1"),
    );
    if args.iter().any(|arg| arg == "--probe") {
        if let Err(error) = run_probe() {
            eprintln!("probe failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--dump-keymap") {
        if let Err(error) = run_keymap_dump() {
            eprintln!("keymap dump failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--trace-telemetry") {
        if let Err(error) = run_telemetry_trace(Duration::from_secs(30)) {
            eprintln!("telemetry trace failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 560.0])
            .with_min_inner_size([420.0, 170.0])
            .with_always_on_top(),
        event_loop_builder: linux_x11_event_loop_builder(),
        ..Default::default()
    };

    eframe::run_native(
        "Sofle Plus 2 Viewer",
        options,
        Box::new(|cc| Ok(Box::new(SofleApp::new(cc)))),
    )
}

fn init_trace_log(enabled: bool) {
    TRACE_START.get_or_init(Instant::now);
    let file = if enabled {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(trace_log_path())
            .ok()
    } else {
        None
    };
    let _ = TRACE_LOG.set(Mutex::new(file));
    trace_log("trace_start");
}

fn trace_log(message: impl AsRef<str>) {
    let Some(log) = TRACE_LOG.get() else {
        return;
    };
    let Ok(mut guard) = log.lock() else {
        return;
    };
    let Some(file) = guard.as_mut() else {
        return;
    };
    let elapsed_us = TRACE_START.get_or_init(Instant::now).elapsed().as_micros();
    let _ = writeln!(file, "{elapsed_us:>10}us {}", message.as_ref());
    let _ = file.flush();
}

fn trace_log_path() -> PathBuf {
    std::env::temp_dir().join("sofle-plus2-viewer-events.log")
}

#[cfg(target_os = "linux")]
fn linux_x11_event_loop_builder() -> Option<eframe::EventLoopBuilderHook> {
    std::env::var_os("DISPLAY")?;

    Some(Box::new(|builder| {
        use winit::platform::x11::EventLoopBuilderExtX11 as _;
        builder.with_x11();
    }))
}

#[cfg(not(target_os = "linux"))]
fn linux_x11_event_loop_builder() -> Option<eframe::EventLoopBuilderHook> {
    None
}

fn run_probe() -> io::Result<()> {
    let mut device = SofleDevice::connect()?;
    println!("device: {}", device.path);
    println!("via_protocol: 0x{:04x}", device.protocol_version()?);
    let layer_count = device.layer_count()?.clamp(1, MAX_LAYERS);
    println!("layers: {layer_count}");
    if let Ok(name) = device.oled_name(0xff) {
        println!("oled_name: {name}");
    }
    for layer in 0..layer_count {
        if let Ok(name) = device.oled_name(layer as u8) {
            println!("layer_{layer}: {name}");
        }
    }
    match device.enable_telemetry()? {
        Some(event) => println!("telemetry: yes, active_layer={}", event.active_layer),
        None => println!("telemetry: no, using Vial polling fallback"),
    }
    let matrix = device.switch_matrix()?;
    let pressed = KEYS.iter().filter(|key| matrix[key.row][key.col]).count();
    println!("pressed_keys: {pressed}");
    Ok(())
}

fn run_keymap_dump() -> io::Result<()> {
    let mut device = SofleDevice::connect()?;
    let layer_count = device.layer_count()?.clamp(1, MAX_LAYERS);
    let keycodes = device.fetch_keycodes(layer_count)?;
    println!("device: {}", device.path);
    println!("layers: {layer_count}");
    for (layer, layer_keycodes) in keycodes.iter().enumerate().take(layer_count) {
        let name = device.oled_name(layer as u8).unwrap_or_default();
        println!("layer {layer}: {name}");
        for key in KEYS {
            let keycode = layer_keycodes[key.row][key.col];
            println!(
                "  r{}c{} {:04x} {}",
                key.row,
                key.col,
                keycode,
                keycode_label(keycode)
            );
        }
    }
    Ok(())
}

fn run_telemetry_trace(duration: Duration) -> io::Result<()> {
    let mut device = SofleDevice::connect()?;
    println!("device: {}", device.path);
    match device.enable_telemetry()? {
        Some(event) => {
            println!(
                "telemetry enabled: kind={} seq={} flags=0x{:02x} layer={} pressed={} rows={}",
                event.kind.as_str(),
                event.seq,
                event.flags,
                event.active_layer,
                pressed_count(&event.matrix),
                matrix_rows_hex(&event.matrix)
            );
        }
        None => {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "telemetry command is not supported by this firmware",
            ));
        }
    }

    println!("trace_start_ms=0 duration_ms={}", duration.as_millis());
    println!("press keys quickly now; tracing telemetry events...");

    let start = Instant::now();
    let mut last_event_at = start;
    let mut last_matrix = [[false; COLS]; ROWS];
    let mut event_count = 0usize;
    let mut new_telemetry_protocol = false;

    while start.elapsed() < duration {
        let remaining = duration.saturating_sub(start.elapsed());
        let timeout = TELEMETRY_HEARTBEAT_INTERVAL.min(remaining);
        match device.wait_telemetry(timeout)? {
            Some(event) => {
                let now = Instant::now();
                event_count += 1;
                if event.kind == TelemetryKind::Event && (event.seq != 0 || event.flags != 0) {
                    new_telemetry_protocol = true;
                    let _ = device.ack_telemetry(event.seq);
                }
                println!(
                    "event={event_count:04} t_ms={:>6} dt_ms={:>5} kind={} seq={} flags=0x{:02x} layer={} pressed={} changes={} rows={}",
                    now.duration_since(start).as_millis(),
                    now.duration_since(last_event_at).as_millis(),
                    event.kind.as_str(),
                    event.seq,
                    event.flags,
                    event.active_layer,
                    pressed_count(&event.matrix),
                    matrix_change_text(&matrix_changes(&last_matrix, &event.matrix)),
                    matrix_rows_hex(&event.matrix)
                );
                last_event_at = now;
                last_matrix = event.matrix;
            }
            None => {
                device.send_telemetry_heartbeat(new_telemetry_protocol)?;
                println!("heartbeat t_ms={:>6}", start.elapsed().as_millis());
            }
        }
    }

    println!("trace_done events={event_count}");
    Ok(())
}

struct SofleApp {
    shared: Arc<Mutex<LiveState>>,
    always_on_top: bool,
    window_level_applied: bool,
    last_stale_log: Instant,
    last_window_size: Vec2,
    last_resize_seen: Instant,
    last_auto_fit_width: Option<f32>,
}

impl SofleApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let shared = Arc::new(Mutex::new(LiveState::default()));
        spawn_device_worker(shared.clone(), cc.egui_ctx.clone());
        Self {
            shared,
            always_on_top: true,
            window_level_applied: false,
            last_stale_log: Instant::now(),
            last_window_size: Vec2::ZERO,
            last_resize_seen: Instant::now(),
            last_auto_fit_width: None,
        }
    }

    fn apply_window_level(&self, ctx: &egui::Context) {
        let level = if self.always_on_top {
            egui::viewport::WindowLevel::AlwaysOnTop
        } else {
            egui::viewport::WindowLevel::Normal
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
    }
}

impl eframe::App for SofleApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.window_level_applied {
            self.apply_window_level(ctx);
            self.window_level_applied = true;
        }

        let state = self
            .shared
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| LiveState::default());
        let active_layer = state
            .active_layer
            .unwrap_or_else(|| {
                infer_active_layer(&state.matrix, &state.keycodes, state.layer_count)
            })
            .min(state.layer_count.saturating_sub(1));
        trace_log(format!(
            "frame active_layer={active_layer} pressed={} visible={} visual_only={} rows={} last_update_age_ms={}",
            pressed_count(&state.matrix),
            visible_count(&state),
            visual_only_count(&state),
            matrix_rows_hex(&state.matrix),
            state
                .last_update
                .map(|updated| updated.elapsed().as_millis())
                .unwrap_or(0)
        ));
        self.log_stale_pressed(&state);
        if let Some(after) = next_visible_expiry(&state) {
            ctx.request_repaint_after(after);
        }

        let screen_rect = ctx.screen_rect();
        let current_size = screen_rect.size();
        if (current_size.x - self.last_window_size.x).abs() > 1.0
            || (current_size.y - self.last_window_size.y).abs() > 1.0
        {
            self.last_window_size = current_size;
            self.last_resize_seen = Instant::now();
            ctx.request_repaint_after(MINIMAL_AUTOFIT_DELAY);
        }

        let minimal_view = screen_rect.width() < MINIMAL_VIEW_WIDTH;
        if minimal_view {
            let desired_height = minimal_inner_height(screen_rect.width());
            let width_needs_fit = self
                .last_auto_fit_width
                .is_none_or(|width| (width - screen_rect.width()).abs() > 4.0);
            if self.last_resize_seen.elapsed() >= MINIMAL_AUTOFIT_DELAY
                && width_needs_fit
                && (screen_rect.height() - desired_height).abs() > 12.0
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                    screen_rect.width(),
                    desired_height,
                )));
                self.last_auto_fit_width = Some(screen_rect.width());
                self.last_resize_seen = Instant::now();
            }
        } else {
            self.last_auto_fit_width = None;
        }

        if !minimal_view {
            egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
                ui.add_space(6.0);
                let compact_top = ui.available_width() < 760.0;
                ui.horizontal_wrapped(|ui| {
                    if compact_top {
                        ui.label(egui::RichText::new("Sofle+2").size(20.0));
                    } else {
                        ui.heading("Sofle Plus 2");
                    }
                    ui.separator();
                    let status_color = if state.connected {
                        Color32::from_rgb(87, 190, 128)
                    } else {
                        Color32::from_rgb(235, 176, 82)
                    };
                    ui.colored_label(
                        status_color,
                        if compact_top && state.connected {
                            "Live"
                        } else {
                            &state.status
                        },
                    );
                    if !state.device_path.is_empty() {
                        ui.separator();
                        let device = if compact_top {
                            state
                                .device_path
                                .rsplit('/')
                                .next()
                                .unwrap_or(&state.device_path)
                        } else {
                            &state.device_path
                        };
                        ui.monospace(device);
                    }
                    ui.separator();
                    if ui
                        .button(if compact_top {
                            "Reload"
                        } else {
                            "Reload keymap"
                        })
                        .clicked()
                    {
                        request_keymap_reload(&self.shared, ctx);
                    }
                    ui.separator();
                    let mode_label = if self.always_on_top {
                        if compact_top {
                            "Top"
                        } else {
                            "On top"
                        }
                    } else if compact_top {
                        "Normal"
                    } else {
                        "Respect windows"
                    };
                    if ui
                        .toggle_value(&mut self.always_on_top, mode_label)
                        .changed()
                    {
                        self.apply_window_level(ctx);
                    }
                    if let Some(error) = &state.error {
                        ui.separator();
                        ui.colored_label(Color32::from_rgb(232, 110, 92), error);
                    }
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Layer");
                    for layer in 0..state.layer_count.min(MAX_LAYERS) {
                        let label = state
                            .layer_names
                            .get(layer)
                            .map(String::as_str)
                            .unwrap_or("L?");
                        let text = if compact_top {
                            layer.to_string()
                        } else {
                            format!("{layer} {label}")
                        };
                        let fill = if layer == active_layer {
                            Color32::from_rgb(45, 128, 116)
                        } else {
                            Color32::from_rgb(38, 43, 49)
                        };
                        egui::Frame::none()
                            .fill(fill)
                            .rounding(egui::Rounding::same(6.0))
                            .inner_margin(egui::Margin::symmetric(
                                if compact_top { 7.0 } else { 8.0 },
                                3.0,
                            ))
                            .show(ui, |ui| {
                                ui.label(text);
                            });
                    }
                });
                ui.add_space(6.0);
            });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(18, 20, 23)))
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                let show_touchpad = !minimal_view
                    && rect.width() >= TOUCHPAD_VIEW_WIDTH
                    && rect.height() >= TOUCHPAD_VIEW_HEIGHT;
                draw_keyboard(ui, rect, &state, active_layer, show_touchpad);
                ui.allocate_space(rect.size());
            });

        if !minimal_view {
            egui::TopBottomPanel::bottom("bottom_bar").show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let pressed = pressed_labels(&state, active_layer);
                    if pressed.is_empty() {
                        ui.label("Pressed: none");
                    } else {
                        ui.label(format!("Pressed: {}", pressed.join(", ")));
                    }
                    if state.last_update.is_some() {
                        ui.separator();
                        ui.label("Live HID matrix");
                    }
                    let stale =
                        stale_pressed_labels(&state, Instant::now(), Duration::from_millis(250));
                    if !stale.is_empty() {
                        ui.separator();
                        ui.colored_label(Color32::from_rgb(235, 176, 82), stale.join(", "));
                    }
                });
                ui.add_space(4.0);
            });
        }
    }
}

impl SofleApp {
    fn log_stale_pressed(&mut self, state: &LiveState) {
        if self.last_stale_log.elapsed() < Duration::from_millis(250) {
            return;
        }
        let stale = stale_pressed_labels(state, Instant::now(), Duration::from_millis(250));
        if stale.is_empty() {
            return;
        }
        self.last_stale_log = Instant::now();
        trace_log(format!(
            "stale_pressed keys={} rows={} last_update_age_ms={}",
            stale.join(","),
            matrix_rows_hex(&state.matrix),
            state
                .last_update
                .map(|updated| updated.elapsed().as_millis())
                .unwrap_or(0)
        ));
    }
}

fn spawn_device_worker(shared: Arc<Mutex<LiveState>>, ctx: egui::Context) {
    thread::spawn(move || loop {
        match SofleDevice::connect() {
            Ok(mut device) => {
                trace_log(format!("device_connected path={}", device.path));
                update_state(&shared, &ctx, |state| {
                    state.connected = true;
                    state.device_path = device.path.clone();
                    state.status = "Connected over Vial raw HID".to_owned();
                    state.error = None;
                    state.last_update = Some(Instant::now());
                });

                load_keymap_from_device(&mut device, &shared, &ctx, "Live");

                match device.enable_telemetry() {
                    Ok(Some(event)) => {
                        trace_log(format!(
                            "telemetry_enabled kind={} proto={} seq={} flags=0x{:02x} layer={} pressed={} touch={} rows={}",
                            event.kind.as_str(),
                            event.protocol,
                            event.seq,
                            event.flags,
                            event.active_layer,
                            pressed_count(&event.matrix),
                            touchpad_count(&event.touchpad),
                            matrix_rows_hex(&event.matrix)
                        ));
                        update_state(&shared, &ctx, |state| {
                            state.status = "Live telemetry".to_owned();
                            apply_matrix_update(state, event.matrix, Some(event.active_layer));
                            apply_touchpad_update(state, event.touchpad);
                            state.error = None;
                        });

                        let mut last_telemetry_matrix = event.matrix;
                        let mut last_telemetry_layer = event.active_layer;
                        let mut last_telemetry_touchpad = event.touchpad;
                        let mut new_telemetry_protocol = event.protocol >= 2;
                        loop {
                            if take_keymap_reload(&shared) {
                                load_keymap_from_device(
                                    &mut device,
                                    &shared,
                                    &ctx,
                                    "Live telemetry",
                                );
                            }
                            match device.wait_telemetry(TELEMETRY_HEARTBEAT_INTERVAL) {
                                Ok(Some(event)) => {
                                    if event.protocol >= 2
                                        || (event.kind == TelemetryKind::Event
                                            && (event.seq != 0 || event.flags != 0))
                                    {
                                        new_telemetry_protocol = true;
                                    }
                                    if event.kind == TelemetryKind::Event
                                        && (event.seq != 0 || event.flags != 0)
                                    {
                                        match device.ack_telemetry(event.seq) {
                                            Ok(()) => {
                                                trace_log(format!("ack_sent seq={}", event.seq))
                                            }
                                            Err(error) => trace_log(format!(
                                                "ack_error seq={} error={error}",
                                                event.seq
                                            )),
                                        }
                                    }
                                    let changes =
                                        matrix_changes(&last_telemetry_matrix, &event.matrix);
                                    let touchpad_changed =
                                        event.touchpad != last_telemetry_touchpad;
                                    if changes.is_empty()
                                        && event.active_layer == last_telemetry_layer
                                        && !touchpad_changed
                                    {
                                        trace_log(format!(
                                            "hid_packet_same kind={} proto={} seq={} flags=0x{:02x} layer={} pressed={} touch={} rows={}",
                                            event.kind.as_str(),
                                            event.protocol,
                                            event.seq,
                                            event.flags,
                                            event.active_layer,
                                            pressed_count(&event.matrix),
                                            touchpad_count(&event.touchpad),
                                            matrix_rows_hex(&event.matrix)
                                        ));
                                        continue;
                                    }
                                    let change_text = if touchpad_changed && changes.is_empty() {
                                        "touchpad".to_owned()
                                    } else if changes.is_empty() {
                                        "layer".to_owned()
                                    } else {
                                        matrix_change_text(&changes)
                                    };
                                    trace_log(format!(
                                        "hid_event kind={} proto={} seq={} flags=0x{:02x} layer={} pressed={} touch={} changes={} rows={}",
                                        event.kind.as_str(),
                                        event.protocol,
                                        event.seq,
                                        event.flags,
                                        event.active_layer,
                                        pressed_count(&event.matrix),
                                        touchpad_count(&event.touchpad),
                                        change_text,
                                        matrix_rows_hex(&event.matrix)
                                    ));
                                    last_telemetry_matrix = event.matrix;
                                    last_telemetry_layer = event.active_layer;
                                    last_telemetry_touchpad = event.touchpad;
                                    update_state(&shared, &ctx, |state| {
                                        state.connected = true;
                                        apply_matrix_update(
                                            state,
                                            event.matrix,
                                            Some(event.active_layer),
                                        );
                                        apply_touchpad_update(state, event.touchpad);
                                        state.error = None;
                                    });
                                }
                                Ok(None) => {
                                    trace_log("heartbeat_send");
                                    if let Err(error) =
                                        device.send_telemetry_heartbeat(new_telemetry_protocol)
                                    {
                                        update_state(&shared, &ctx, |state| {
                                            state.connected = false;
                                            state.status = "Disconnected, rescanning".to_owned();
                                            state.error = Some(error.to_string());
                                            clear_matrix_state(state);
                                            state.active_layer = None;
                                        });
                                        break;
                                    }
                                }
                                Err(error) => {
                                    update_state(&shared, &ctx, |state| {
                                        state.connected = false;
                                        state.status = "Disconnected, rescanning".to_owned();
                                        state.error = Some(error.to_string());
                                        clear_matrix_state(state);
                                        state.active_layer = None;
                                    });
                                    break;
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        trace_log("telemetry_unavailable fallback");
                        update_state(&shared, &ctx, |state| {
                            state.status = "Live via Vial polling fallback".to_owned();
                            state.active_layer = None;
                        });
                    }
                    Err(error) => {
                        trace_log(format!("telemetry_enable_error error={error}"));
                        update_state(&shared, &ctx, |state| {
                            state.status = "Live via Vial polling fallback".to_owned();
                            state.error = Some(format!("telemetry enable failed: {error}"));
                            state.active_layer = None;
                        });
                    }
                }

                let mut last_matrix: Option<Matrix> = None;
                let mut layer_tracker = FallbackLayerTracker::default();
                loop {
                    if take_keymap_reload(&shared) {
                        load_keymap_from_device(
                            &mut device,
                            &shared,
                            &ctx,
                            "Live via Vial polling fallback",
                        );
                        layer_tracker = FallbackLayerTracker::default();
                        last_matrix = None;
                    }
                    match device.switch_matrix() {
                        Ok(matrix) => {
                            let (keycodes, layer_count) = keymap_snapshot(&shared);
                            let active_layer =
                                layer_tracker.update(&matrix, &keycodes, layer_count);
                            if last_matrix.as_ref() != Some(&matrix) {
                                last_matrix = Some(matrix);
                                update_state(&shared, &ctx, |state| {
                                    state.connected = true;
                                    apply_matrix_update(state, matrix, Some(active_layer));
                                    state.error = None;
                                });
                            }
                            thread::sleep(MATRIX_POLL_INTERVAL);
                        }
                        Err(error) => {
                            update_state(&shared, &ctx, |state| {
                                state.connected = false;
                                state.status = "Disconnected, rescanning".to_owned();
                                state.error = Some(error.to_string());
                                clear_matrix_state(state);
                                state.active_layer = None;
                            });
                            break;
                        }
                    }
                }
            }
            Err(error) => {
                update_state(&shared, &ctx, |state| {
                    state.connected = false;
                    state.status = "Scanning for Sofle Plus 2".to_owned();
                    state.device_path.clear();
                    state.error = Some(error.to_string());
                    clear_matrix_state(state);
                    state.active_layer = None;
                });
                thread::sleep(Duration::from_secs(1));
            }
        }
    });
}

fn load_keymap_from_device(
    device: &mut SofleDevice,
    shared: &Arc<Mutex<LiveState>>,
    ctx: &egui::Context,
    live_status: &str,
) {
    let layer_count = device
        .layer_count()
        .unwrap_or(MAX_LAYERS)
        .clamp(1, MAX_LAYERS);
    let mut layer_names = std::array::from_fn(|i| format!("L{i}"));
    if let Ok(name) = device.oled_name(0xff) {
        update_state(shared, ctx, |state| {
            state.status = format!("Connected to {name}");
        });
    }
    for (layer, layer_name) in layer_names.iter_mut().enumerate().take(layer_count) {
        if let Ok(name) = device.oled_name(layer as u8) {
            if !name.is_empty() {
                *layer_name = name;
            }
        }
    }

    update_state(shared, ctx, |state| {
        state.status = "Loading keymap from Vial".to_owned();
        state.layer_count = layer_count;
        state.layer_names = layer_names.clone();
        state.keymap_reload_requested = false;
    });

    match device.fetch_keycodes(layer_count) {
        Ok(keycodes) => {
            update_state(shared, ctx, |state| {
                state.keycodes = keycodes;
                state.status = live_status.to_owned();
                state.error = None;
                state.keymap_reload_requested = false;
            });
        }
        Err(error) => {
            update_state(shared, ctx, |state| {
                state.status = "Live matrix, labels unavailable".to_owned();
                state.error = Some(error.to_string());
                state.keymap_reload_requested = false;
            });
        }
    }
}

fn request_keymap_reload(shared: &Arc<Mutex<LiveState>>, ctx: &egui::Context) {
    if let Ok(mut state) = shared.lock() {
        state.keymap_reload_requested = true;
        state.status = "Reloading keymap".to_owned();
    }
    ctx.request_repaint();
}

fn take_keymap_reload(shared: &Arc<Mutex<LiveState>>) -> bool {
    shared
        .lock()
        .map(|mut state| {
            let requested = state.keymap_reload_requested;
            state.keymap_reload_requested = false;
            requested
        })
        .unwrap_or(false)
}

fn keymap_snapshot(shared: &Arc<Mutex<LiveState>>) -> (Keycodes, usize) {
    shared
        .lock()
        .map(|state| (state.keycodes, state.layer_count))
        .unwrap_or(([[[KC_NO; COLS]; ROWS]; MAX_LAYERS], MAX_LAYERS))
}

fn apply_matrix_update(state: &mut LiveState, matrix: Matrix, active_layer: Option<usize>) {
    let now = Instant::now();
    let changes = matrix_changes(&state.matrix, &matrix);
    let previous = state.matrix;
    let mut releases = Vec::new();
    trace_log(format!(
        "state_update layer={active_layer:?} pressed={} changes={}",
        pressed_count(&matrix),
        matrix_change_text(&changes)
    ));
    for row in 0..ROWS {
        for col in 0..COLS {
            match (previous[row][col], matrix[row][col]) {
                (false, true) => {
                    state.pressed_since[row][col] = Some(now);
                }
                (true, false) => {
                    if let Some(since) = state.pressed_since[row][col] {
                        releases.push(format!("r{row}c{col}:{}ms", since.elapsed().as_millis()));
                    }
                    state.pressed_since[row][col] = None;
                }
                (false, false) => {
                    state.pressed_since[row][col] = None;
                }
                (true, true) => {}
            }
            if matrix[row][col] {
                state.visible_until[row][col] = Some(now + KEY_FLASH_DURATION);
            }
        }
    }
    if !releases.is_empty() {
        trace_log(format!("release_ages {}", releases.join(",")));
    }
    state.matrix = matrix;
    state.active_layer = active_layer;
    state.last_update = Some(now);
}

fn apply_touchpad_update(state: &mut LiveState, touchpad: TouchpadTelemetry) {
    state.touchpad.telemetry = touchpad;
    state.touchpad.last_update = Some(Instant::now());
}

fn clear_matrix_state(state: &mut LiveState) {
    state.matrix = [[false; COLS]; ROWS];
    state.visible_until = [[None; COLS]; ROWS];
    state.pressed_since = [[None; COLS]; ROWS];
    state.touchpad = TouchpadState::default();
}

fn update_state(
    shared: &Arc<Mutex<LiveState>>,
    ctx: &egui::Context,
    update: impl FnOnce(&mut LiveState),
) {
    if let Ok(mut state) = shared.lock() {
        update(&mut state);
    }
    trace_log("repaint_requested");
    ctx.request_repaint();
}

struct SofleDevice {
    inner: PlatformDevice,
    path: String,
}

#[cfg(target_os = "linux")]
struct PlatformDevice {
    file: File,
}

#[cfg(target_os = "windows")]
struct PlatformDevice {
    device: hidapi::HidDevice,
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
struct PlatformDevice;

#[derive(Clone, Copy)]
struct TelemetryEvent {
    matrix: Matrix,
    active_layer: usize,
    kind: TelemetryKind,
    protocol: u8,
    seq: u8,
    flags: u8,
    touchpad: TouchpadTelemetry,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TelemetryKind {
    Event,
    Enabled,
    Heartbeat,
}

impl TelemetryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::Enabled => "enabled",
            Self::Heartbeat => "heartbeat",
        }
    }
}

struct FallbackLayerTracker {
    persistent_mask: u16,
    previous_matrix: Matrix,
}

impl Default for FallbackLayerTracker {
    fn default() -> Self {
        Self {
            persistent_mask: 1,
            previous_matrix: [[false; COLS]; ROWS],
        }
    }
}

impl FallbackLayerTracker {
    fn update(&mut self, matrix: &Matrix, keycodes: &Keycodes, layer_count: usize) -> usize {
        let layer_count = layer_count.clamp(1, MAX_LAYERS);
        self.persistent_mask = normalize_layer_mask(self.persistent_mask, layer_count);

        let mut active_mask =
            active_mask_from_pressed(matrix, keycodes, layer_count, self.persistent_mask);
        let previous_matrix = self.previous_matrix;
        for key in KEYS
            .iter()
            .filter(|key| matrix[key.row][key.col] && !previous_matrix[key.row][key.col])
        {
            let keycode = resolve_keycode(keycodes, highest_layer(active_mask), key.row, key.col);
            if self.apply_layer_press(keycode, layer_count) {
                active_mask =
                    active_mask_from_pressed(matrix, keycodes, layer_count, self.persistent_mask);
            }
        }

        self.previous_matrix = *matrix;
        highest_layer(active_mask).min(layer_count - 1)
    }

    fn apply_layer_press(&mut self, keycode: u16, layer_count: usize) -> bool {
        if is_direct_layer_switch(keycode) {
            self.move_to_layer((keycode & 0x1f) as usize, layer_count);
            true
        } else if (QK_TOGGLE_LAYER..=QK_TOGGLE_LAYER_MAX).contains(&keycode) {
            let layer = (keycode & 0x1f) as usize;
            if layer < layer_count {
                self.persistent_mask ^= 1 << layer;
                self.persistent_mask = normalize_layer_mask(self.persistent_mask, layer_count);
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    fn move_to_layer(&mut self, layer: usize, layer_count: usize) {
        self.persistent_mask = if layer < layer_count {
            1 | (1 << layer)
        } else {
            1
        };
        self.persistent_mask = normalize_layer_mask(self.persistent_mask, layer_count);
    }
}

impl SofleDevice {
    fn connect() -> io::Result<Self> {
        platform_connect()
    }

    fn protocol_version(&mut self) -> io::Result<u16> {
        let mut request = [0u8; 32];
        request[0] = ID_GET_PROTOCOL_VERSION;
        let response = self.transact(request, Duration::from_millis(250))?;
        if response[0] != ID_GET_PROTOCOL_VERSION {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "not a VIA raw HID endpoint",
            ));
        }
        Ok(u16::from_be_bytes([response[1], response[2]]))
    }

    fn layer_count(&mut self) -> io::Result<usize> {
        let mut request = [0u8; 32];
        request[0] = ID_DYNAMIC_KEYMAP_GET_LAYER_COUNT;
        let response = self.transact(request, Duration::from_millis(250))?;
        Ok(response[1] as usize)
    }

    fn oled_name(&mut self, item: u8) -> io::Result<String> {
        let mut request = [0u8; 32];
        request[0] = ID_LIGHTING_GET_VALUE;
        request[1] = VIALRGB_GET_OLED_CONFIG;
        request[2] = item;
        let response = self.transact(request, Duration::from_millis(250))?;
        let bytes = &response[3..8];
        Ok(bytes_to_name(bytes))
    }

    fn fetch_keycodes(&mut self, layer_count: usize) -> io::Result<Keycodes> {
        let mut keycodes = [[[KC_NO; COLS]; ROWS]; MAX_LAYERS];
        for (layer, layer_rows) in keycodes.iter_mut().enumerate().take(layer_count) {
            for (row, row_values) in layer_rows.iter_mut().enumerate() {
                for (col, keycode) in row_values.iter_mut().enumerate() {
                    *keycode = self.keycode(layer as u8, row as u8, col as u8)?;
                }
            }
        }
        Ok(keycodes)
    }

    fn keycode(&mut self, layer: u8, row: u8, col: u8) -> io::Result<u16> {
        let mut request = [0u8; 32];
        request[0] = ID_DYNAMIC_KEYMAP_GET_KEYCODE;
        request[1] = layer;
        request[2] = row;
        request[3] = col;
        let response = self.transact(request, Duration::from_millis(250))?;
        Ok(u16::from_be_bytes([response[4], response[5]]))
    }

    fn switch_matrix(&mut self) -> io::Result<Matrix> {
        let mut request = [0u8; 32];
        request[0] = ID_GET_KEYBOARD_VALUE;
        request[1] = ID_SWITCH_MATRIX_STATE;
        let response = self.transact(request, Duration::from_millis(250))?;
        let mut matrix = [[false; COLS]; ROWS];
        for (row, row_values) in matrix.iter_mut().enumerate() {
            let bits = response[2 + row];
            for (col, pressed) in row_values.iter_mut().enumerate() {
                *pressed = (bits & (1 << col)) != 0;
            }
        }
        Ok(matrix)
    }

    fn enable_telemetry(&mut self) -> io::Result<Option<TelemetryEvent>> {
        let mut request = [0u8; 32];
        request[0] = SOFLE_TELEMETRY_COMMAND;
        request[1] = SOFLE_TELEMETRY_ENABLE;
        request[2] = 1;
        let response = self.transact(request, Duration::from_millis(250))?;
        Ok(parse_telemetry_packet(&response))
    }

    fn send_telemetry_heartbeat(&mut self, new_telemetry_protocol: bool) -> io::Result<()> {
        let mut request = [0u8; 32];
        request[0] = SOFLE_TELEMETRY_COMMAND;
        request[1] = if new_telemetry_protocol {
            SOFLE_TELEMETRY_HEARTBEAT
        } else {
            SOFLE_TELEMETRY_ENABLE
        };
        request[2] = 1;
        self.inner.write_packet(&request)
    }

    fn ack_telemetry(&mut self, seq: u8) -> io::Result<()> {
        let mut request = [0u8; 32];
        request[0] = SOFLE_TELEMETRY_COMMAND;
        request[1] = SOFLE_TELEMETRY_ACK;
        request[2] = 1;
        request[3] = seq;
        self.inner.write_packet(&request)
    }

    fn wait_telemetry(&mut self, timeout: Duration) -> io::Result<Option<TelemetryEvent>> {
        let start = Instant::now();
        loop {
            let Some(remaining) = timeout.checked_sub(start.elapsed()) else {
                return Ok(None);
            };
            match self.inner.read_packet_timeout(remaining)? {
                Some(packet) => {
                    if let Some(event) = parse_telemetry_live_packet(&packet) {
                        return Ok(Some(event));
                    } else if packet[0] == SOFLE_TELEMETRY_COMMAND {
                        trace_log(format!(
                            "hid_ignored type=0x{:02x} version={} seq={} flags=0x{:02x}",
                            packet[1], packet[2], packet[14], packet[15]
                        ));
                    }
                }
                None => return Ok(None),
            }
        }
    }

    fn transact(&mut self, request: [u8; 32], timeout: Duration) -> io::Result<[u8; 32]> {
        self.drain();
        self.inner.write_packet(&request)?;

        match self.inner.read_packet_timeout(timeout)? {
            Some(response) => Ok(response),
            None => Err(io::Error::new(
                ErrorKind::TimedOut,
                "Vial command timed out",
            )),
        }
    }

    fn drain(&mut self) {
        while matches!(self.inner.read_packet_timeout(Duration::ZERO), Ok(Some(_))) {}
    }
}

#[cfg(target_os = "linux")]
fn platform_connect() -> io::Result<SofleDevice> {
    let candidates = hidraw_candidates()?;
    let mut errors = Vec::new();

    for path in candidates {
        match PlatformDevice::open_path(&path) {
            Ok(inner) => {
                let path_text = path.display().to_string();
                let mut device = SofleDevice {
                    inner,
                    path: path_text.clone(),
                };
                match device.protocol_version() {
                    Ok(0x0009) => return Ok(device),
                    Ok(version) => {
                        errors.push(format!("{path_text} reported VIA protocol 0x{version:04x}"))
                    }
                    Err(error) => errors.push(format!("{path_text}: {error}")),
                }
            }
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }

    if errors.is_empty() {
        Err(io::Error::new(
            ErrorKind::NotFound,
            "no FC32:0287 hidraw interface found",
        ))
    } else {
        Err(io::Error::other(errors.join("; ")))
    }
}

#[cfg(target_os = "linux")]
impl PlatformDevice {
    fn open_path(path: &PathBuf) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)?;
        Ok(Self { file })
    }

    fn write_packet(&mut self, packet: &[u8; 32]) -> io::Result<()> {
        self.file.write_all(packet)?;
        self.file.flush()
    }

    fn read_packet_timeout(&mut self, timeout: Duration) -> io::Result<Option<[u8; 32]>> {
        let fd = self.file.as_raw_fd();
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout_ms = duration_to_i32_millis(timeout);

        loop {
            let ready = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
            if ready == 0 {
                return Ok(None);
            }
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if pollfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                return Err(io::Error::new(
                    ErrorKind::BrokenPipe,
                    "hidraw endpoint closed",
                ));
            }
            if pollfd.revents & libc::POLLIN != 0 {
                return self.read_packet().map(Some);
            }
        }
    }

    fn read_packet(&mut self) -> io::Result<[u8; 32]> {
        let mut buffer = [0u8; 64];
        match self.file.read(&mut buffer) {
            Ok(0) => Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "empty hidraw read",
            )),
            Ok(read) => Ok(packet_from_hid_buffer(&buffer, read)),
            Err(error) => Err(error),
        }
    }
}

#[cfg(target_os = "windows")]
fn platform_connect() -> io::Result<SofleDevice> {
    let api = hidapi::HidApi::new().map_err(hid_error)?;
    let mut errors = Vec::new();
    let mut found = false;

    for info in api
        .device_list()
        .filter(|info| info.vendor_id() == USB_VENDOR_ID && info.product_id() == USB_PRODUCT_ID)
    {
        found = true;
        let path_text = info.path().to_string_lossy().into_owned();
        match api.open_path(info.path()).map_err(hid_error) {
            Ok(device) => {
                let mut device = SofleDevice {
                    inner: PlatformDevice { device },
                    path: path_text.clone(),
                };
                match device.protocol_version() {
                    Ok(0x0009) => return Ok(device),
                    Ok(version) => {
                        errors.push(format!("{path_text} reported VIA protocol 0x{version:04x}"))
                    }
                    Err(error) => errors.push(format!("{path_text}: {error}")),
                }
            }
            Err(error) => errors.push(format!("{path_text}: {error}")),
        }
    }

    if !found {
        Err(io::Error::new(
            ErrorKind::NotFound,
            "no FC32:0287 HID interface found",
        ))
    } else {
        Err(io::Error::other(errors.join("; ")))
    }
}

#[cfg(target_os = "windows")]
impl PlatformDevice {
    fn write_packet(&mut self, packet: &[u8; 32]) -> io::Result<()> {
        let mut report = [0u8; 33];
        report[1..].copy_from_slice(packet);
        self.device.write(&report).map_err(hid_error)?;
        Ok(())
    }

    fn read_packet_timeout(&mut self, timeout: Duration) -> io::Result<Option<[u8; 32]>> {
        let mut buffer = [0u8; 64];
        let read = self
            .device
            .read_timeout(&mut buffer, duration_to_i32_millis(timeout))
            .map_err(hid_error)?;
        if read == 0 {
            return Ok(None);
        }
        Ok(Some(packet_from_hid_buffer(&buffer, read)))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn platform_connect() -> io::Result<SofleDevice> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "Sofle Plus 2 Viewer currently supports Linux and Windows",
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
impl PlatformDevice {
    fn write_packet(&mut self, _packet: &[u8; 32]) -> io::Result<()> {
        Err(io::Error::new(ErrorKind::Unsupported, "unsupported OS"))
    }

    fn read_packet_timeout(&mut self, _timeout: Duration) -> io::Result<Option<[u8; 32]>> {
        Err(io::Error::new(ErrorKind::Unsupported, "unsupported OS"))
    }
}

fn packet_from_hid_buffer(buffer: &[u8; 64], read: usize) -> [u8; 32] {
    let mut packet = [0u8; 32];
    if read >= 33 && buffer[0] == 0 {
        packet.copy_from_slice(&buffer[1..33]);
    } else if read >= 32 {
        packet.copy_from_slice(&buffer[..32]);
    } else {
        packet[..read].copy_from_slice(&buffer[..read]);
    }
    packet
}

fn duration_to_i32_millis(timeout: Duration) -> i32 {
    timeout.as_millis().min(i32::MAX as u128) as i32
}

#[cfg(target_os = "windows")]
fn hid_error(error: hidapi::HidError) -> io::Error {
    io::Error::new(ErrorKind::Other, error.to_string())
}

fn parse_telemetry_packet(packet: &[u8; 32]) -> Option<TelemetryEvent> {
    parse_telemetry_packet_kind(packet, true)
}

fn parse_telemetry_live_packet(packet: &[u8; 32]) -> Option<TelemetryEvent> {
    parse_telemetry_packet_kind(packet, true)
}

fn parse_telemetry_packet_kind(
    packet: &[u8; 32],
    allow_enabled_response: bool,
) -> Option<TelemetryEvent> {
    if packet[0] != SOFLE_TELEMETRY_COMMAND {
        return None;
    }
    let kind = match packet[1] {
        SOFLE_TELEMETRY_EVENT => TelemetryKind::Event,
        SOFLE_TELEMETRY_ENABLED if allow_enabled_response => TelemetryKind::Enabled,
        SOFLE_TELEMETRY_HEARTBEAT_RESPONSE if allow_enabled_response => TelemetryKind::Heartbeat,
        SOFLE_TELEMETRY_ACKED => return None,
        _ => return None,
    };
    if packet[1] != SOFLE_TELEMETRY_EVENT && !allow_enabled_response {
        return None;
    }
    let protocol = packet[2];
    if !(1..=2).contains(&protocol) {
        return None;
    }

    let mut matrix = [[false; COLS]; ROWS];
    for (row, row_values) in matrix.iter_mut().enumerate() {
        let bits = packet[4 + row];
        for (col, pressed) in row_values.iter_mut().enumerate() {
            *pressed = (bits & (1 << col)) != 0;
        }
    }

    Some(TelemetryEvent {
        matrix,
        active_layer: (packet[3] as usize).min(MAX_LAYERS - 1),
        kind,
        protocol,
        seq: packet[14],
        flags: packet[15],
        touchpad: parse_touchpad_telemetry(protocol, packet),
    })
}

fn parse_touchpad_telemetry(protocol: u8, packet: &[u8; 32]) -> TouchpadTelemetry {
    if protocol < 2 {
        return TouchpadTelemetry::default();
    }

    let mut contacts = [TouchContact::default(); 5];
    let mask = packet[16];
    for index in 0..contacts.len() {
        contacts[index] = TouchContact {
            active: (mask & (1 << index)) != 0,
            x: packet[18 + (index * 2)],
            y: packet[19 + (index * 2)],
        };
    }

    TouchpadTelemetry {
        enabled: packet[28] != 0,
        ptp_mode: packet[29] != 0,
        contacts,
    }
}

#[cfg(target_os = "linux")]
fn hidraw_candidates() -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir("/sys/class/hidraw")? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let uevent = entry.path().join("device/uevent");
        let Ok(text) = std::fs::read_to_string(uevent) else {
            continue;
        };
        if text.contains(VID_PID_MARKER) {
            paths.push(PathBuf::from("/dev").join(name.as_ref()));
        }
    }
    paths.sort_by_key(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("hidraw"))
            .and_then(|suffix| suffix.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    });
    Ok(paths)
}

fn bytes_to_name(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}

fn draw_keyboard(
    ui: &mut egui::Ui,
    rect: Rect,
    state: &LiveState,
    active_layer: usize,
    show_touchpad: bool,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_rgb(18, 20, 23));
    let now = Instant::now();

    let scale = (rect.width() / KEYBOARD_LOGICAL_WIDTH)
        .min(rect.height() / KEYBOARD_LOGICAL_HEIGHT)
        .max(1.0);
    let board_size = Vec2::new(
        KEYBOARD_LOGICAL_WIDTH * scale,
        KEYBOARD_LOGICAL_HEIGHT * scale,
    );
    let origin = Pos2::new(
        rect.center().x - board_size.x / 2.0,
        rect.center().y - board_size.y / 2.0,
    );

    draw_half_plate(
        &painter,
        origin,
        scale,
        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(7.35, 5.95)),
    );
    draw_half_plate(
        &painter,
        origin,
        scale,
        Rect::from_min_max(Pos2::new(9.1, 0.0), Pos2::new(16.65, 5.95)),
    );

    if show_touchpad {
        draw_large_touchpad(&painter, origin, scale, &state.touchpad);
    }

    for key in KEYS {
        let pressed = state.matrix[key.row][key.col]
            || state.visible_until[key.row][key.col].is_some_and(|until| until > now);
        let keycode = resolve_keycode(&state.keycodes, active_layer, key.row, key.col);
        let is_layer_key = target_layer(keycode).is_some();
        let compact_key = key.w < 0.85 || key.h < 0.85;
        let label = draw_label(&compact_label(&keycode_label(keycode)), compact_key);

        let key_rect = Rect::from_min_size(
            Pos2::new(origin.x + key.x * scale, origin.y + key.y * scale),
            Vec2::new(key.w * scale * 0.9, key.h * scale * 0.84),
        );
        let fill = if pressed {
            Color32::from_rgb(230, 143, 70)
        } else if is_layer_key {
            Color32::from_rgb(42, 70, 91)
        } else {
            Color32::from_rgb(35, 40, 46)
        };
        let stroke = if pressed {
            Stroke::new(2.0, Color32::from_rgb(255, 222, 148))
        } else if is_layer_key {
            Stroke::new(1.3, Color32::from_rgb(90, 160, 190))
        } else {
            Stroke::new(1.0, Color32::from_rgb(74, 83, 93))
        };
        painter.rect_filled(key_rect, 6.0, fill);
        painter.rect_stroke(key_rect, 6.0, stroke);
        let label_size = fitted_label_size(
            &label,
            key_rect.width(),
            key_rect.height(),
            if compact_key { 10.5 } else { 13.0 },
        );
        painter.text(
            key_rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(label_size),
            Color32::from_rgb(235, 240, 245),
        );

        if !compact_key && key_rect.width() >= 44.0 && key_rect.height() >= 38.0 {
            let matrix_label = format!("{},{}", key.row, key.col);
            painter.text(
                Pos2::new(key_rect.left() + 5.0, key_rect.bottom() - 5.0),
                Align2::LEFT_BOTTOM,
                matrix_label,
                FontId::monospace(8.5),
                Color32::from_rgb(121, 132, 145),
            );
        }
    }
}

fn draw_large_touchpad(
    painter: &egui::Painter,
    origin: Pos2,
    scale: f32,
    touchpad: &TouchpadState,
) {
    let rect = logical_rect(
        origin,
        scale,
        Rect::from_min_size(Pos2::new(9.2, 0.45), Vec2::new(1.25, 1.9)),
    );
    let rounding = (scale * 0.11).clamp(6.0, 11.0);

    painter.rect_filled(rect, rounding, Color32::from_rgb(12, 14, 16));
    painter.rect_stroke(
        rect,
        rounding,
        Stroke::new(1.2, Color32::from_rgb(61, 70, 80)),
    );
    painter.rect_stroke(
        rect.shrink((scale * 0.06).max(3.0)),
        (rounding - 2.0).max(4.0),
        Stroke::new(0.7, Color32::from_rgb(30, 36, 43)),
    );

    for contact in touchpad
        .telemetry
        .contacts
        .iter()
        .filter(|contact| contact.active)
    {
        let x = rect.left() + rect.width() * (contact.x as f32 / 255.0);
        let y = rect.top() + rect.height() * (contact.y as f32 / 255.0);
        let center = Pos2::new(x, y);
        let radius = (scale * 0.09).clamp(5.0, 11.0);
        painter.circle_filled(center, radius, Color32::from_rgb(230, 143, 70));
        painter.circle_stroke(
            center,
            radius,
            Stroke::new(1.5, Color32::from_rgb(255, 222, 148)),
        );
    }
}

fn fitted_label_size(label: &str, width: f32, height: f32, max_size: f32) -> f32 {
    let chars = label.chars().count().max(1) as f32;
    (width / (chars * 0.62))
        .min(height * 0.45)
        .clamp(5.5, max_size)
}

fn minimal_inner_height(width: f32) -> f32 {
    ((width / KEYBOARD_LOGICAL_WIDTH) * KEYBOARD_LOGICAL_HEIGHT)
        .ceil()
        .clamp(170.0, 380.0)
}

fn draw_label(label: &str, compact_key: bool) -> String {
    if !compact_key {
        return label.to_owned();
    }

    match label {
        "UP" => "^".to_owned(),
        "DOWN" => "v".to_owned(),
        "LEFT" => "<".to_owned(),
        "RGHT" | "RIGHT" => ">".to_owned(),
        "MB1" => "M1".to_owned(),
        _ if label.chars().count() > 3 => label.chars().take(3).collect(),
        _ => label.to_owned(),
    }
}

fn logical_rect(origin: Pos2, scale: f32, logical: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(
            origin.x + logical.min.x * scale,
            origin.y + logical.min.y * scale,
        ),
        Pos2::new(
            origin.x + logical.max.x * scale,
            origin.y + logical.max.y * scale,
        ),
    )
}

fn draw_half_plate(painter: &egui::Painter, origin: Pos2, scale: f32, logical: Rect) {
    let rect = logical_rect(origin, scale, logical);
    painter.rect_filled(rect.expand(8.0), 8.0, Color32::from_rgb(24, 27, 31));
    painter.rect_stroke(
        rect.expand(8.0),
        8.0,
        Stroke::new(1.0, Color32::from_rgb(45, 52, 60)),
    );
}

fn next_visible_expiry(state: &LiveState) -> Option<Duration> {
    let now = Instant::now();
    state
        .visible_until
        .iter()
        .flatten()
        .flatten()
        .filter_map(|until| until.checked_duration_since(now))
        .min()
}

fn pressed_labels(state: &LiveState, active_layer: usize) -> Vec<String> {
    KEYS.iter()
        .filter(|key| state.matrix[key.row][key.col])
        .map(|key| {
            let keycode = resolve_keycode(&state.keycodes, active_layer, key.row, key.col);
            format!(
                "{} r{}c{}",
                compact_label(&keycode_label(keycode)),
                key.row,
                key.col
            )
        })
        .collect()
}

fn pressed_count(matrix: &Matrix) -> usize {
    KEYS.iter().filter(|key| matrix[key.row][key.col]).count()
}

fn visible_count(state: &LiveState) -> usize {
    let now = Instant::now();
    KEYS.iter()
        .filter(|key| {
            state.matrix[key.row][key.col]
                || state.visible_until[key.row][key.col].is_some_and(|until| until > now)
        })
        .count()
}

fn touchpad_count(touchpad: &TouchpadTelemetry) -> usize {
    touchpad
        .contacts
        .iter()
        .filter(|contact| contact.active)
        .count()
}

fn visual_only_count(state: &LiveState) -> usize {
    let now = Instant::now();
    KEYS.iter()
        .filter(|key| {
            !state.matrix[key.row][key.col]
                && state.visible_until[key.row][key.col].is_some_and(|until| until > now)
        })
        .count()
}

fn stale_pressed_labels(state: &LiveState, now: Instant, threshold: Duration) -> Vec<String> {
    KEYS.iter()
        .filter_map(|key| {
            if !state.matrix[key.row][key.col] {
                return None;
            }
            let since = state.pressed_since[key.row][key.col]?;
            let age = now.checked_duration_since(since)?;
            if age < threshold {
                return None;
            }
            let active_layer = state
                .active_layer
                .unwrap_or_else(|| {
                    infer_active_layer(&state.matrix, &state.keycodes, state.layer_count)
                })
                .min(state.layer_count.saturating_sub(1));
            let keycode = resolve_keycode(&state.keycodes, active_layer, key.row, key.col);
            Some(format!(
                "{} r{}c{} {}ms",
                compact_label(&keycode_label(keycode)),
                key.row,
                key.col,
                age.as_millis()
            ))
        })
        .collect()
}

fn matrix_rows_hex(matrix: &Matrix) -> String {
    (0..ROWS)
        .map(|row| {
            let bits = (0..COLS).fold(0u8, |acc, col| {
                if matrix[row][col] {
                    acc | (1u8 << col)
                } else {
                    acc
                }
            });
            format!("{bits:02x}")
        })
        .collect::<Vec<_>>()
        .join(":")
}

fn matrix_changes(previous: &Matrix, current: &Matrix) -> Vec<String> {
    let mut changes = Vec::new();
    for key in KEYS {
        match (previous[key.row][key.col], current[key.row][key.col]) {
            (false, true) => changes.push(format!("+r{}c{}", key.row, key.col)),
            (true, false) => changes.push(format!("-r{}c{}", key.row, key.col)),
            _ => {}
        }
    }
    changes
}

fn matrix_change_text(changes: &[String]) -> String {
    if changes.is_empty() {
        "none".to_owned()
    } else {
        changes.join(",")
    }
}

fn infer_active_layer(matrix: &Matrix, keycodes: &Keycodes, layer_count: usize) -> usize {
    highest_layer(active_mask_from_pressed(matrix, keycodes, layer_count, 1))
}

fn active_mask_from_pressed(
    matrix: &Matrix,
    keycodes: &Keycodes,
    layer_count: usize,
    initial_mask: u16,
) -> u16 {
    let layer_count = layer_count.clamp(1, MAX_LAYERS);
    let mut active_mask = normalize_layer_mask(initial_mask, layer_count);

    for _ in 0..MAX_LAYERS {
        let before = active_mask;
        let highest = highest_layer(active_mask).min(layer_count - 1);
        for key in KEYS.iter().filter(|key| matrix[key.row][key.col]) {
            let keycode = resolve_keycode(keycodes, highest, key.row, key.col);
            if let Some(layer) = momentary_layer(keycode) {
                if layer < layer_count {
                    active_mask |= 1 << layer;
                }
            }
        }
        if active_mask == before {
            break;
        }
    }

    normalize_layer_mask(active_mask, layer_count)
}

fn normalize_layer_mask(mask: u16, layer_count: usize) -> u16 {
    let valid = valid_layer_mask(layer_count);
    let normalized = mask & valid;
    if normalized == 0 {
        1
    } else {
        normalized
    }
}

fn valid_layer_mask(layer_count: usize) -> u16 {
    let layer_count = layer_count.clamp(1, MAX_LAYERS);
    ((1u32 << layer_count) - 1) as u16
}

fn highest_layer(mask: u16) -> usize {
    (0..MAX_LAYERS)
        .rev()
        .find(|layer| (mask & (1 << layer)) != 0)
        .unwrap_or(0)
}

fn resolve_keycode(keycodes: &Keycodes, layer: usize, row: usize, col: usize) -> u16 {
    for current_layer in (0..=layer.min(MAX_LAYERS - 1)).rev() {
        let keycode = keycodes[current_layer][row][col];
        if keycode != KC_TRANSPARENT {
            return keycode;
        }
    }
    KC_NO
}

fn target_layer(keycode: u16) -> Option<usize> {
    if let Some(layer) = momentary_layer(keycode) {
        Some(layer)
    } else if is_layer_action_keycode(keycode) {
        Some((keycode & 0x1f) as usize)
    } else {
        None
    }
}

fn momentary_layer(keycode: u16) -> Option<usize> {
    if (QK_MOMENTARY..=QK_MOMENTARY_MAX).contains(&keycode) {
        Some((keycode & 0x1f) as usize)
    } else if (QK_LAYER_TAP..=QK_LAYER_TAP_MAX).contains(&keycode) {
        Some(((keycode >> 8) & 0x0f) as usize)
    } else if (QK_LAYER_MOD..=QK_LAYER_MOD_MAX).contains(&keycode) {
        Some(((keycode >> 5) & 0x0f) as usize)
    } else if (QK_ONE_SHOT_LAYER..=QK_ONE_SHOT_LAYER_MAX).contains(&keycode)
        || (QK_LAYER_TAP_TOGGLE..=QK_LAYER_TAP_TOGGLE_MAX).contains(&keycode)
    {
        Some((keycode & 0x1f) as usize)
    } else {
        None
    }
}

fn is_direct_layer_switch(keycode: u16) -> bool {
    (QK_TO..=QK_TO_MAX).contains(&keycode)
        || (QK_DEF_LAYER..=QK_DEF_LAYER_MAX).contains(&keycode)
        || (QK_PERSISTENT_DEF_LAYER..=QK_PERSISTENT_DEF_LAYER_MAX).contains(&keycode)
}

fn is_layer_action_keycode(keycode: u16) -> bool {
    is_direct_layer_switch(keycode) || (QK_TOGGLE_LAYER..=QK_TOGGLE_LAYER_MAX).contains(&keycode)
}

fn compact_label(label: &str) -> String {
    const MAX: usize = 8;
    if label.chars().count() <= MAX {
        return label.to_owned();
    }
    let mut compact: String = label.chars().take(MAX - 2).collect();
    compact.push_str("..");
    compact
}

fn keycode_label(keycode: u16) -> String {
    if (QK_MODS..=QK_MODS_MAX).contains(&keycode) {
        let mods = ((keycode >> 8) & 0x1f) as u8;
        let base = keycode & 0xff;
        return format!("{}{}", mod_prefix(mods), keycode_label(base));
    }
    if (QK_MOD_TAP..=QK_MOD_TAP_MAX).contains(&keycode) {
        let mods = ((keycode >> 8) & 0x1f) as u8;
        let base = keycode & 0xff;
        return format!("MT {}{}", mod_prefix(mods), keycode_label(base));
    }
    if (QK_LAYER_TAP..=QK_LAYER_TAP_MAX).contains(&keycode) {
        let layer = (keycode >> 8) & 0x0f;
        let base = keycode & 0xff;
        return format!("LT{layer}/{}", keycode_label(base));
    }
    if (QK_LAYER_MOD..=QK_LAYER_MOD_MAX).contains(&keycode) {
        return format!("LM{}", (keycode >> 5) & 0x0f);
    }
    if (QK_TO..=QK_TO_MAX).contains(&keycode) {
        return format!("TO{}", keycode & 0x1f);
    }
    if (QK_MOMENTARY..=QK_MOMENTARY_MAX).contains(&keycode) {
        return format!("MO{}", keycode & 0x1f);
    }
    if (QK_DEF_LAYER..=QK_DEF_LAYER_MAX).contains(&keycode) {
        return format!("DF{}", keycode & 0x1f);
    }
    if (QK_TOGGLE_LAYER..=QK_TOGGLE_LAYER_MAX).contains(&keycode) {
        return format!("TG{}", keycode & 0x1f);
    }
    if (QK_ONE_SHOT_LAYER..=QK_ONE_SHOT_LAYER_MAX).contains(&keycode) {
        return format!("OSL{}", keycode & 0x1f);
    }
    if (QK_LAYER_TAP_TOGGLE..=QK_LAYER_TAP_TOGGLE_MAX).contains(&keycode) {
        return format!("TT{}", keycode & 0x1f);
    }
    if (QK_PERSISTENT_DEF_LAYER..=QK_PERSISTENT_DEF_LAYER_MAX).contains(&keycode) {
        return format!("PDF{}", keycode & 0x1f);
    }
    if (QK_KB_0..=QK_KB_0 + 31).contains(&keycode) {
        return custom_keycode_label((keycode - QK_KB_0) as usize).to_owned();
    }

    basic_keycode_label(keycode).to_owned()
}

fn mod_prefix(mods: u8) -> String {
    let mut out = String::new();
    if mods & 0x01 != 0 {
        out.push_str("C+");
    }
    if mods & 0x02 != 0 {
        out.push_str("S+");
    }
    if mods & 0x04 != 0 {
        out.push_str("A+");
    }
    if mods & 0x08 != 0 {
        out.push_str("G+");
    }
    if mods & 0x10 != 0 {
        out.push_str("R+");
    }
    out
}

fn custom_keycode_label(index: usize) -> &'static str {
    const CUSTOM: &[&str] = &[
        "ATABF", "ATABR", "ATMU", "ATMD", "POWER", "SCRUD", "SCRLR", "DPI+", "DPI-", "DPI0",
        "SCR+", "SCR-", "SCR0", "LASCR", "LASW2", "LASW3", "LA0", "TPTOG", "SNPMO", "SNPTG",
        "SNPLR", "SNP+", "SNP-", "SNPSH", "OSTOG", "ZMTOG", "GLOBE", "TPINF",
    ];
    CUSTOM.get(index).copied().unwrap_or("CUSTOM")
}

fn basic_keycode_label(keycode: u16) -> &'static str {
    match keycode {
        KC_NO => "",
        KC_TRANSPARENT => "TRNS",
        0x0004 => "A",
        0x0005 => "B",
        0x0006 => "C",
        0x0007 => "D",
        0x0008 => "E",
        0x0009 => "F",
        0x000a => "G",
        0x000b => "H",
        0x000c => "I",
        0x000d => "J",
        0x000e => "K",
        0x000f => "L",
        0x0010 => "M",
        0x0011 => "N",
        0x0012 => "O",
        0x0013 => "P",
        0x0014 => "Q",
        0x0015 => "R",
        0x0016 => "S",
        0x0017 => "T",
        0x0018 => "U",
        0x0019 => "V",
        0x001a => "W",
        0x001b => "X",
        0x001c => "Y",
        0x001d => "Z",
        0x001e => "1",
        0x001f => "2",
        0x0020 => "3",
        0x0021 => "4",
        0x0022 => "5",
        0x0023 => "6",
        0x0024 => "7",
        0x0025 => "8",
        0x0026 => "9",
        0x0027 => "0",
        0x0028 => "ENT",
        0x0029 => "ESC",
        0x002a => "BSPC",
        0x002b => "TAB",
        0x002c => "SPC",
        0x002d => "-",
        0x002e => "=",
        0x002f => "[",
        0x0030 => "]",
        0x0031 => "\\",
        0x0032 => "NUHS",
        0x0033 => ";",
        0x0034 => "'",
        0x0035 => "`",
        0x0036 => ",",
        0x0037 => ".",
        0x0038 => "/",
        0x0039 => "CAPS",
        0x003a => "F1",
        0x003b => "F2",
        0x003c => "F3",
        0x003d => "F4",
        0x003e => "F5",
        0x003f => "F6",
        0x0040 => "F7",
        0x0041 => "F8",
        0x0042 => "F9",
        0x0043 => "F10",
        0x0044 => "F11",
        0x0045 => "F12",
        0x0046 => "PSCR",
        0x0047 => "SCRL",
        0x0048 => "PAUS",
        0x0049 => "INS",
        0x004a => "HOME",
        0x004b => "PGUP",
        0x004c => "DEL",
        0x004d => "END",
        0x004e => "PGDN",
        0x004f => "RGHT",
        0x0050 => "LEFT",
        0x0051 => "DOWN",
        0x0052 => "UP",
        0x0053 => "NUM",
        0x0054 => "KP/",
        0x0055 => "KP*",
        0x0056 => "KP-",
        0x0057 => "KP+",
        0x0058 => "KPENT",
        0x0059 => "KP1",
        0x005a => "KP2",
        0x005b => "KP3",
        0x005c => "KP4",
        0x005d => "KP5",
        0x005e => "KP6",
        0x005f => "KP7",
        0x0060 => "KP8",
        0x0061 => "KP9",
        0x0062 => "KP0",
        0x0063 => "KPDOT",
        0x0065 => "APP",
        0x0066 => "POWER",
        0x0067 => "KPEQL",
        0x00a5 => "PWR",
        0x00a6 => "SLEEP",
        0x00a7 => "WAKE",
        0x00a8 => "MUTE",
        0x00a9 => "VOL+",
        0x00aa => "VOL-",
        0x00ab => "NEXT",
        0x00ac => "PREV",
        0x00ad => "STOP",
        0x00ae => "PLAY",
        0x00af => "MSEL",
        0x00b0 => "EJECT",
        0x00b1 => "MAIL",
        0x00b2 => "CALC",
        0x00b3 => "MYPC",
        0x00b4 => "SRCH",
        0x00b5 => "WWW",
        0x00b6 => "BACK",
        0x00b7 => "FWD",
        0x00b8 => "WSTOP",
        0x00b9 => "RFRS",
        0x00ba => "FAV",
        0x00bb => "FFWD",
        0x00bc => "RWD",
        0x00bd => "BRI+",
        0x00be => "BRI-",
        0x00c0 => "ASST",
        0x00c1 => "MC",
        0x00c2 => "LPAD",
        0x00cd => "MSUP",
        0x00ce => "MSDN",
        0x00cf => "MSLFT",
        0x00d0 => "MSRGT",
        0x00d1 => "MB1",
        0x00d2 => "MB2",
        0x00d3 => "MB3",
        0x00d9 => "WHUP",
        0x00da => "WHDN",
        0x00db => "WHL",
        0x00dc => "WHR",
        0x00dd => "ACL0",
        0x00de => "ACL1",
        0x00df => "ACL2",
        0x00e0 => "LCTL",
        0x00e1 => "LSFT",
        0x00e2 => "LALT",
        0x00e3 => "LGUI",
        0x00e4 => "RCTL",
        0x00e5 => "RSFT",
        0x00e6 => "RALT",
        0x00e7 => "RGUI",
        0x7800..=0x78ff => "RGB",
        0x7c00..=0x7dff => "QMK",
        _ => "KC?",
    }
}
