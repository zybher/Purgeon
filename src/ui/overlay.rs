//! The focus overlay: a slim bar that slides down from the top of the screen when the shortcut
//! (Alt+Shift+P by default) is pressed, and back up when it's pressed again.
//!
//! Left, large: the app in front (whatever window has focus: a game, Blender, a browser) with
//! the RAM, VRAM, CPU and GPU it uses, its helper processes included. Right, small and dimmed:
//! the whole PC, which only lights up when something is wrong.
//!
//! It is an ordinary always-on-top window that never takes focus: nothing is injected into or
//! hooked onto the app below, so it is safe next to anti-cheat. It shows over borderless and
//! windowed apps, not over exclusive fullscreen. While it is closed its window doesn't exist.
//!
//! FPS comes from the frame events DirectX already announces to Windows' event tracing (what
//! PresentMon and the Xbox Game Bar read), so it doesn't touch the game either. Windows only
//! lets administrators listen, so without that the cell says "Elevate for FPS".

use super::morph::{self, Piece};
use super::widgets;
use crate::anim;
use crate::fmt;
use crate::sys::Snapshot;
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, ViewportBuilder, ViewportId};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU8, Ordering};

const TITLE: &str = "Purgeon overlay";
pub(super) const W: f32 = 920.0;
pub(super) const H: f32 = 64.0;
/// Gap between the top of the screen and the bar, in points
const TOP: f32 = 10.0;

static OPEN: AtomicBool = AtomicBool::new(false);
/// The window that was in front when the overlay opened (followed while it stays open)
static FRONT: AtomicIsize = AtomicIsize::new(0);
static HWND: AtomicIsize = AtomicIsize::new(0);
/// Closed by growing into the main window: the bar leaves at once instead of sliding up
static SHUT: AtomicBool = AtomicBool::new(false);
/// The size picked in Settings: 0 Auto, then 100%, 125%, 150% (255: not read yet)
static SIZE: AtomicU8 = AtomicU8::new(255);
/// The scale the bar's window was made at
static MADE_AT: AtomicU32 = AtomicU32::new(0);

/// The choices Settings offers for the bar's size
pub const SIZES: [&str; 4] = ["Auto", "100%", "125%", "150%"];
const SIZE_VALUES: [Option<&str>; 4] = [None, Some("100"), Some("125"), Some("150")];

/// The size picked in Settings (an index into SIZES)
pub fn size_pick() -> usize {
    let v = SIZE.load(Ordering::Relaxed);
    if v != 255 {
        return v as usize;
    }
    let saved = crate::settings::text("overlay_size");
    let i = SIZE_VALUES.iter().position(|v| *v == saved.as_deref()).unwrap_or(0);
    SIZE.store(i as u8, Ordering::Relaxed);
    i
}

pub fn set_size_pick(i: usize) {
    let i = i.min(3);
    SIZE.store(i as u8, Ordering::Relaxed);
    crate::settings::set_text("overlay_size", SIZE_VALUES[i]);
}

/// What Auto picks on a screen: bigger on very wide screens, where the bar would otherwise
/// look small (1.25 at 3440 wide, 1.5 at 4K, both at 100% Windows scaling)
fn auto_for(mon: isize) -> f32 {
    let mut mi = MonitorInfo { size: std::mem::size_of::<MonitorInfo>() as u32, monitor: [0; 4], work: [0; 4], flags: 0 };
    if unsafe { GetMonitorInfoW(mon, &mut mi) } == 0 {
        return 1.0;
    }
    let (mut dx, mut dy) = (96u32, 96u32);
    // MDT_EFFECTIVE_DPI
    if unsafe { GetDpiForMonitor(mon, 0, &mut dx, &mut dy) } != 0 {
        dx = 96;
    }
    let wide = (mi.monitor[2] - mi.monitor[0]) as f32 * 96.0 / dx.max(1) as f32;
    if wide >= 3800.0 {
        1.5
    } else if wide >= 3000.0 {
        1.25
    } else {
        1.0
    }
}

/// The scale Auto uses on the screen Purgeon's window is on (for Settings)
pub fn auto_here() -> f32 {
    // MONITOR_DEFAULTTONEAREST
    auto_for(unsafe { MonitorFromWindow(crate::platform::main_window(), 2) })
}

/// The bar's scale right now
fn zoom() -> f32 {
    match size_pick() {
        1 => 1.0,
        2 => 1.25,
        3 => 1.5,
        _ => auto_for(unsafe { MonitorFromWindow(FRONT.load(Ordering::Relaxed), 1) }),
    }
}

#[link(name = "user32")]
extern "system" {
    fn GetForegroundWindow() -> isize;
    fn GetWindowThreadProcessId(h: isize, pid: *mut u32) -> u32;
    fn MonitorFromWindow(h: isize, flags: u32) -> isize;
    fn GetMonitorInfoW(m: isize, info: *mut MonitorInfo) -> i32;
    fn GetWindowRect(h: isize, r: *mut [i32; 4]) -> i32;
    fn SetWindowPos(h: isize, after: isize, x: i32, y: i32, w: i32, h2: i32, flags: u32) -> i32;
    fn GetWindowLongPtrW(h: isize, i: i32) -> isize;
    fn SetWindowLongPtrW(h: isize, i: i32, v: isize) -> isize;
    fn IsWindow(h: isize) -> i32;
    fn MonitorFromRect(r: *const [i32; 4], flags: u32) -> isize;
}

#[link(name = "shcore")]
extern "system" {
    fn GetDpiForMonitor(m: isize, kind: u32, x: *mut u32, y: *mut u32) -> i32;
}

#[repr(C)]
struct MonitorInfo {
    size: u32,
    monitor: [i32; 4],
    work: [i32; 4],
    flags: u32,
}

pub fn is_open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

fn pid_of(h: isize) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(h, &mut pid) };
    pid
}

/// The focused window, unless it is Purgeon's own (then the last one seen stays)
fn track_front() {
    let h = unsafe { GetForegroundWindow() };
    if h != 0 && pid_of(h) != std::process::id() {
        FRONT.store(h, Ordering::Relaxed);
    }
}

/// Opens or closes the overlay (the shortcut, the tray menu)
pub fn toggle(ctx: &egui::Context) {
    let open = !OPEN.load(Ordering::Relaxed);
    if open {
        track_front();
        SHUT.store(false, Ordering::Relaxed);
    }
    OPEN.store(open, Ordering::Relaxed);
    ctx.request_repaint();
}

/// What the app in front is using, its helper processes included
struct Usage {
    /// Its processes, to find the one drawing frames
    pids: Vec<u32>,
    name: String,
    path: Option<std::sync::Arc<str>>,
    ram: u64,
    vram: u64,
    cpu: f32,
    gpu: f32,
}

fn usage(s: &Snapshot, pid: u32) -> Option<Usage> {
    let me = s.procs.iter().find(|p| p.pid == pid)?;
    // Every process with the same name (browsers, Roblox), plus everything they started
    let mut set: HashSet<u32> = s.procs.iter().filter(|p| p.name.eq_ignore_ascii_case(&me.name)).map(|p| p.pid).collect();
    for _ in 0..6 {
        let before = set.len();
        for p in &s.procs {
            if p.pid > 4 && set.contains(&p.ppid) && p.pid != std::process::id() {
                set.insert(p.pid);
            }
        }
        if set.len() == before {
            break;
        }
    }
    let name = crate::platform::friendly_name(me.name.trim_end_matches(".exe").trim_end_matches(".EXE"), me.path.as_deref());
    let mut u = Usage { pids: set.iter().copied().collect(), name, path: me.path.clone(), ram: 0, vram: 0, cpu: 0.0, gpu: 0.0 };
    let mut shared = 0;
    for p in s.procs.iter().filter(|p| set.contains(&p.pid)) {
        // the same rule as the RAM column: private memory of each, plus the largest shared part
        u.ram += p.private_ws;
        shared = shared.max(p.shared());
        u.vram += p.vram;
        u.cpu += p.cpu;
        u.gpu += p.gpu;
    }
    u.ram += shared;
    u.gpu = u.gpu.min(100.0);
    Some(u)
}

/// Called every frame of the main window (also while it sits in the tray)
pub fn show(ctx: &egui::Context, snap: Option<&Snapshot>) {
    let open = OPEN.load(Ordering::Relaxed);
    let t = anim::ease(ctx, egui::Id::new("overlay_t"), open);
    if !open && t <= 0.0 {
        // fully slid away: drop the window, so nothing is drawn or kept for it, and give back
        // the memory its drawing used
        if HWND.swap(0, Ordering::Relaxed) != 0 {
            crate::platform::trim_memory();
        }
        SHUT.store(false, Ordering::Relaxed);
        return;
    }
    if open {
        track_front();
    }
    let builder = ViewportBuilder::default()
        .with_title(TITLE)
        .with_inner_size([W, H])
        .with_position([-30000.0, -30000.0])
        .with_decorations(false)
        .with_resizable(false)
        .with_taskbar(false)
        .with_active(false)
        .with_always_on_top();
    // A bigger size draws the bar at a higher zoom, so the text stays sharp. The window is
    // made at that size, so a new size gets a new window.
    let z = zoom();
    if MADE_AT.swap(z.to_bits(), Ordering::Relaxed) != z.to_bits() {
        HWND.store(0, Ordering::Relaxed);
    }
    let main_zoom = ctx.zoom_factor();
    ctx.options_mut(|o| o.zoom_factor = z);
    ctx.show_viewport_immediate(ViewportId::from_hash_of(("purgeon_overlay", z.to_bits())), builder, |ctx, _| {
        place(t);
        egui::CentralPanel::default().frame(egui::Frame::none().fill(card())).show(ctx, |ui| bar(ui, snap));
    });
    ctx.options_mut(|o| o.zoom_factor = main_zoom);
}

/// Finds the overlay's window once, makes it never take focus, and puts it at the top center
/// of the screen the app in front is on, `t` of the way down
fn place(t: f32) {
    let mut h = HWND.load(Ordering::Relaxed);
    if h == 0 || unsafe { IsWindow(h) } == 0 {
        h = crate::platform::find_window(TITLE);
        if h == 0 {
            return;
        }
        HWND.store(h, Ordering::Relaxed);
        unsafe {
            // GWL_EXSTYLE: WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, so clicks never pull focus
            // away from the game and it stays out of Alt+Tab
            let ex = GetWindowLongPtrW(h, -20);
            SetWindowLongPtrW(h, -20, ex | 0x0800_0000 | 0x80);
        }
        crate::platform::style_popup(h);
    }
    unsafe {
        let front = FRONT.load(Ordering::Relaxed);
        let mut mi = MonitorInfo { size: std::mem::size_of::<MonitorInfo>() as u32, monitor: [0; 4], work: [0; 4], flags: 0 };
        // MONITOR_DEFAULTTOPRIMARY
        if GetMonitorInfoW(MonitorFromWindow(front, 1), &mut mi) == 0 {
            return;
        }
        let mut r = [0i32; 4];
        GetWindowRect(h, &mut r);
        let (w, hh) = (r[2] - r[0], r[3] - r[1]);
        let scale = hh as f32 / H;
        let x = mi.monitor[0] + (mi.monitor[2] - mi.monitor[0] - w) / 2;
        let shown = mi.monitor[1] + (TOP * scale) as i32;
        let mut y = shown - ((hh as f32 + TOP * scale) * (1.0 - t)) as i32;
        let x = if SHUT.load(Ordering::Relaxed) {
            y = -30000;
            -30000
        } else {
            x
        };
        if x != r[0] || y != r[1] {
            // HWND_TOPMOST, SWP_NOSIZE | SWP_NOACTIVATE
            SetWindowPos(h, -1, x, y, 0, 0, 0x1 | 0x10);
        }
    }
}

/// Dev check for the morph: draws the bar into any Ui
pub(super) fn test_bar(ui: &mut egui::Ui, snap: Option<&Snapshot>) {
    track_front();
    bar(ui, snap);
}

/// The overlay's window on screen (physical pixels), while it shows
pub(super) fn bar_rect() -> Option<[i32; 4]> {
    let h = HWND.load(Ordering::Relaxed);
    let mut r = [0i32; 4];
    (h != 0 && !SHUT.load(Ordering::Relaxed) && unsafe { GetWindowRect(h, &mut r) } != 0 && r[1] > -20000).then_some(r)
}

/// Where the main window is about to appear on screen (physical pixels)
pub(super) fn main_rect() -> Option<[i32; 4]> {
    let h = crate::platform::main_window();
    if h == 0 {
        return None;
    }
    unsafe {
        let Some((maxed, n)) = crate::platform::coming_back() else {
            let mut r = [0i32; 4];
            return (GetWindowRect(h, &mut r) != 0).then_some(r);
        };
        // the normal rectangle is relative to the main screen's work area (the taskbar can be
        // on the left or top)
        let mut mi = MonitorInfo { size: std::mem::size_of::<MonitorInfo>() as u32, monitor: [0; 4], work: [0; 4], flags: 0 };
        let primary = MonitorFromRect(&[0, 0, 1, 1], 1);
        let (ox, oy) = if GetMonitorInfoW(primary, &mut mi) != 0 { (mi.work[0] - mi.monitor[0], mi.work[1] - mi.monitor[1]) } else { (0, 0) };
        let r = [n[0] + ox, n[1] + oy, n[2] + ox, n[3] + oy];
        if !maxed {
            return Some(r);
        }
        // maximized: fills the work area of the screen it is on (MONITOR_DEFAULTTONEAREST)
        (GetMonitorInfoW(MonitorFromRect(&r, 2), &mut mi) != 0).then_some(mi.work)
    }
}

fn warn(level: u8) -> Color32 {
    match level {
        2 => Color32::from_rgb(0xef, 0x44, 0x44),
        1 => Color32::from_rgb(0xf5, 0x9e, 0x0b),
        _ => txt_dim(),
    }
}

fn bar(ui: &mut egui::Ui, snap: Option<&Snapshot>) {
    let full = ui.max_rect();
    let p = ui.painter().clone();
    // what is drawn is also noted down, for the morph into the main window
    morph::record_clear();
    let text = |key: Option<&'static str>, pos: Pos2, align: Align2, t: &str, font: FontId, col: Color32| {
        morph::record(Piece::Text { key, text: t.to_string(), pos, align, font, col });
    };
    let Some(s) = snap else {
        p.text(full.center(), Align2::CENTER_CENTER, "Reading the system…", reg(13.0), txt_faint());
        return;
    };

    // ---- the app in front -------------------------------------------------------------
    let front = FRONT.load(Ordering::Relaxed);
    let u = usage(s, pid_of(front));
    let left = Rect::from_min_max(full.min + vec2(14.0, 0.0), pos2(full.min.x + 560.0, full.max.y));
    let cy = full.center().y;
    match &u {
        None => {
            p.text(pos2(left.min.x, cy), Align2::LEFT_CENTER, "No app in front", sb(14.0), txt_dim());
        }
        Some(u) => {
            let ir = Rect::from_center_size(pos2(left.min.x + 14.0, cy), vec2(28.0, 28.0));
            super::icons::paint(ui.ctx(), &p, ir, &u.name, u.path.as_ref());
            morph::record(Piece::Icon { name: u.name.clone(), path: u.path.clone(), rect: ir });
            let nx = ir.max.x + 10.0;
            p.text(pos2(nx, cy - 8.0), Align2::LEFT_CENTER, "IN FRONT", sb(9.5), txt_faint());
            text(None, pos2(nx, cy - 8.0), Align2::LEFT_CENTER, "IN FRONT", sb(9.5), txt_faint());
            widgets::fit(&p, pos2(nx, cy + 7.0), Align2::LEFT_CENTER, &u.name, sb(14.0), 120.0, txt());
            text(None, pos2(nx, cy + 7.0), Align2::LEFT_CENTER, &u.name, sb(14.0), txt());
            let gpu = s.gpus.iter().find(|g| g.dedicated_total > 0);
            let vram_total = gpu.map_or(0, |g| g.dedicated_total);
            let stats: [(&str, String, f32, Color32); 4] = [
                ("RAM", fmt::bytes(u.ram), u.ram as f32 / s.mem.total.max(1) as f32, MEM_COL),
                ("VRAM", if s.gpu_proc { fmt::bytes(u.vram) } else { "—".into() }, u.vram as f32 / vram_total.max(1) as f32, VRAM_COL),
                ("CPU", format!("{:.0}%", u.cpu), u.cpu / 100.0, ACCENT),
                ("GPU", if s.gpu_proc { format!("{:.0}%", u.gpu) } else { "—".into() }, u.gpu / 100.0, GPU_COL),
            ];
            let x0 = nx + 132.0;
            let cw = (left.max.x - x0) / 5.0;
            // FPS first: for a game it is the number that matters most
            {
                let (value, label, col) = match crate::sys::etw::frames() {
                    crate::sys::etw::Frames::Counting => {
                        (crate::sys::etw::fps(&u.pids).map_or("—".into(), |f| format!("{f:.0}")), "FPS", txt())
                    }
                    crate::sys::etw::Frames::NotAllowed => ("Elevate".to_string(), "for FPS", txt_dim()),
                };
                widgets::fit(&p, pos2(x0, cy - 9.0), Align2::LEFT_CENTER, &value, sb(15.0), cw - 12.0, col);
                p.text(pos2(x0, cy + 9.0), Align2::LEFT_CENTER, label, reg(10.5), txt_faint());
                text(None, pos2(x0, cy - 9.0), Align2::LEFT_CENTER, &value, sb(15.0), col);
                text(None, pos2(x0, cy + 9.0), Align2::LEFT_CENTER, label, reg(10.5), txt_faint());
            }
            let x0 = x0 + cw;
            for (i, (label, value, frac, col)) in stats.iter().enumerate() {
                let x = x0 + i as f32 * cw;
                widgets::fit(&p, pos2(x, cy - 9.0), Align2::LEFT_CENTER, value, sb(15.0), cw - 12.0, txt());
                p.text(pos2(x, cy + 9.0), Align2::LEFT_CENTER, *label, reg(10.5), txt_faint());
                widgets::meter(&p, Rect::from_min_size(pos2(x, cy + 18.0), vec2(cw - 16.0, 2.5)), frac.clamp(0.0, 1.0), *col);
                text(None, pos2(x, cy - 9.0), Align2::LEFT_CENTER, value, sb(15.0), txt());
                text(None, pos2(x, cy + 9.0), Align2::LEFT_CENTER, label, reg(10.5), txt_faint());
                morph::record(Piece::Meter { rect: Rect::from_min_size(pos2(x, cy + 18.0), vec2(cw - 16.0, 2.5)), frac: frac.clamp(0.0, 1.0), col: *col });
            }
        }
    }

    // ---- divider ----------------------------------------------------------------------
    let dx = left.max.x + 8.0;
    p.line_segment([pos2(dx, full.min.y + 14.0), pos2(dx, full.max.y - 14.0)], Stroke::new(1.0_f32, border()));

    // ---- the whole PC, quiet unless something is wrong -----------------------------------
    let used = s.mem.used_frac();
    let temp = s.gpus.iter().filter_map(|g| g.sensors.temp).fold(None, |a: Option<f32>, t| Some(a.map_or(t, |a| a.max(t))));
    let down: f64 = s.nets.iter().map(|n| n.rx).sum();
    let gpu_load = s.gpus.iter().map(|g| g.util).fold(None, |a: Option<f32>, u| Some(a.map_or(u, |a| a.max(u))));
    // Two rows with the same columns: how busy each part is, and how hot it is. The CPU's
    // temperature needs a driver Purgeon doesn't load, so it stays a dash.
    let load: [(&str, String, u8); 4] = [
        ("CPU", format!("{:.0}%", s.cpu), if s.cpu >= 95.0 { 2 } else if s.cpu >= 85.0 { 1 } else { 0 }),
        ("GPU", gpu_load.map_or("—".into(), |u| format!("{u:.0}%")), 0),
        ("RAM", format!("{:.0}%", used * 100.0), if used >= 0.92 { 2 } else if used >= 0.85 { 1 } else { 0 }),
        ("NET", fmt::rate(down), 0),
    ];
    let heat: [(&str, String, u8); 2] = [
        ("CPU", "—".into(), 0),
        ("GPU", temp.map_or("—".into(), |t| format!("{t:.0}°C")), temp.map_or(0, |t| if t >= 85.0 { 2 } else if t >= 80.0 { 1 } else { 0 })),
    ];
    let sx = dx + 14.0;
    let btn = Rect::from_center_size(pos2(full.max.x - 24.0, cy), vec2(30.0, 30.0));
    let cols = sx + 38.0;
    let cw = (btn.min.x - 10.0 - cols) / 4.4;
    let col_x = |i: usize| cols + i as f32 * cw;
    for (row, y, cells) in [("LOAD", cy - 10.0, &load[..]), ("TEMP", cy + 11.0, &heat[..])] {
        p.text(pos2(sx, y), Align2::LEFT_CENTER, row, sb(9.0), txt_faint());
        text(None, pos2(sx, y), Align2::LEFT_CENTER, row, sb(9.0), txt_faint());
        for (i, (label, value, level)) in cells.iter().enumerate() {
            let x = col_x(i);
            let w = if i == 3 { btn.min.x - 10.0 - x } else { cw };
            let col = warn(*level);
            let lr = p.text(pos2(x, y), Align2::LEFT_CENTER, *label, reg(9.5), txt_faint());
            let vx = lr.max.x + 5.0;
            let font = if *level > 0 { sb(12.5) } else { reg(12.5) };
            widgets::fit(&p, pos2(vx, y), Align2::LEFT_CENTER, value, font.clone(), w - (vx - x) - 4.0, col);
            text(None, pos2(x, y), Align2::LEFT_CENTER, label, reg(9.5), txt_faint());
            // the numbers the Dashboard also shows fly to their places there
            let key = match (row, *label) {
                ("LOAD", "CPU") => Some("cpu"),
                ("LOAD", "GPU") => Some("gpu"),
                ("LOAD", "RAM") => Some("ram"),
                ("TEMP", "GPU") => Some("gpu_temp"),
                _ => None,
            };
            text(key.filter(|_| value != "—"), pos2(vx, y), Align2::LEFT_CENTER, value, font, col);
        }
    }

    // ---- open the full app ------------------------------------------------------------
    // The pointer reaches a zoomed bar unscaled (eframe converts it before the zoom is set),
    // so the button listens on its rect times the zoom
    let z = ui.ctx().zoom_factor();
    let hit = Rect::from_min_max((btn.min.to_vec2() * z).to_pos2(), (btn.max.to_vec2() * z).to_pos2());
    let mut hu = ui.new_child(egui::UiBuilder::new().max_rect(hit));
    hu.set_clip_rect(hit);
    let mut resp = hu.interact(hit, egui::Id::new("overlay_open"), Sense::click());
    if z == 1.0 {
        resp = resp.on_hover_text("Open Purgeon");
    }
    let (hov, _) = anim::hover_fx(ui.ctx(), &resp);
    p.rect_filled(btn, 8.0, mix(card(), card_hi(), hov));
    // two arrows pointing out of the corners: "expand"
    let c = btn.center();
    let col = mix(txt_dim(), txt(), hov);
    let st = Stroke::new(1.5_f32, col);
    for sgn in [-1.0f32, 1.0] {
        let tip = c + vec2(5.5, -5.5) * sgn;
        p.line_segment([c + vec2(1.5, -1.5) * sgn, tip], st);
        p.line_segment([tip, tip + vec2(-4.0, 0.0) * sgn], st);
        p.line_segment([tip, tip + vec2(0.0, 4.0) * sgn], st);
    }
    if resp.clicked() {
        OPEN.store(false, Ordering::Relaxed);
        // grow into the main window, rather than slide away
        if morph::begin() {
            SHUT.store(true, Ordering::Relaxed);
            place(0.0);
        }
        crate::tray::show_window();
    }
}
