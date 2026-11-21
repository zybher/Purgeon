//! The notification-area (tray) icon, Windows notifications, and "only one Purgeon".
//!
//! The icon lives on its own thread with a hidden window that receives its clicks, so it keeps
//! working while the main window is hidden and not drawing. Left click opens Purgeon; right
//! click shows Open / Alerts / Quit. Notifications use the icon's balloon, which Windows 10 and
//! 11 show as a normal notification (and hold back during Focus assist / Do not disturb).

use crate::sys::{win::wide, Shared};
use eframe::egui;
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, OnceLock};

#[repr(C)]
struct NotifyIconData {
    cb_size: u32,
    hwnd: isize,
    id: u32,
    flags: u32,
    callback_msg: u32,
    icon: isize,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    timeout: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: [u32; 4],
    balloon_icon: isize,
}

#[repr(C)]
struct WndClassEx {
    cb_size: u32,
    style: u32,
    wnd_proc: unsafe extern "system" fn(isize, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: isize,
    icon: isize,
    cursor: isize,
    background: isize,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: isize,
}

#[repr(C)]
struct Msg {
    hwnd: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: [i32; 2],
    private: u32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(c: *const WndClassEx) -> u16;
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: isize, menu: isize, inst: isize, param: *mut c_void) -> isize;
    fn DefWindowProcW(h: isize, m: u32, w: usize, l: isize) -> isize;
    fn GetMessageW(m: *mut Msg, h: isize, min: u32, max: u32) -> i32;
    fn TranslateMessage(m: *const Msg) -> i32;
    fn DispatchMessageW(m: *const Msg) -> isize;
    fn PostMessageW(h: isize, m: u32, w: usize, l: isize) -> i32;
    fn SetForegroundWindow(h: isize) -> i32;
    fn RegisterHotKey(h: isize, id: i32, mods: u32, vk: u32) -> i32;
    fn RegisterWindowMessageW(name: *const u16) -> u32;
    fn ChangeWindowMessageFilterEx(h: isize, m: u32, action: u32, info: *mut c_void) -> i32;
    fn CreatePopupMenu() -> isize;
    fn AppendMenuW(m: isize, flags: u32, id: usize, text: *const u16) -> i32;
    fn SetMenuDefaultItem(m: isize, item: u32, by_pos: u32) -> i32;
    fn TrackPopupMenu(m: isize, flags: u32, x: i32, y: i32, reserved: i32, h: isize, rect: *const c_void) -> i32;
    fn DestroyMenu(m: isize) -> i32;
    fn GetCursorPos(p: *mut [i32; 2]) -> i32;
    fn LoadImageW(inst: isize, name: *const u16, ty: u32, cx: i32, cy: i32, flags: u32) -> isize;
    fn LoadIconW(inst: isize, name: *const u16) -> isize;
    fn GetSystemMetrics(i: i32) -> i32;
    fn FindWindowW(class: *const u16, title: *const u16) -> isize;
}
#[link(name = "shell32")]
extern "system" {
    fn Shell_NotifyIconW(msg: u32, data: *const NotifyIconData) -> i32;
}
#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> isize;
    fn CreateMutexW(attr: *const c_void, owner: i32, name: *const u16) -> isize;
    fn GetLastError() -> u32;
}

const WM_APP: u32 = 0x8000;
/// The icon's clicks arrive as this message
const WM_TRAY: u32 = WM_APP + 1;
/// Sent by a second copy of Purgeon (wparam 0: show the window) or a test script (1: quit,
/// 2: open or close the overlay)
const WM_COMMAND_PURGEON: u32 = WM_APP + 2;
const WM_HOTKEY: u32 = 0x0312;
/// The overlay's shortcut when settings.ini doesn't name another one (`overlay_key=...`)
const DEFAULT_KEY: &str = "Alt+Shift+P";

/// The overlay shortcut as typed in settings.ini ("Ctrl+Alt+O", "Alt+Shift+F2"), as
/// RegisterHotKey's modifiers and virtual key
fn parse_key(s: &str) -> Option<(u32, u32)> {
    let mut mods = 0x4000; // MOD_NOREPEAT
    let mut vk = None;
    for part in s.split('+').map(|p| p.trim().to_ascii_uppercase()) {
        match part.as_str() {
            "ALT" => mods |= 1,
            "CTRL" | "CONTROL" => mods |= 2,
            "SHIFT" => mods |= 4,
            "WIN" => mods |= 8,
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => vk = Some(k.as_bytes()[0] as u32),
            k if k.starts_with('F') => {
                let n: u32 = k[1..].parse().ok()?;
                if !(1..=24).contains(&n) {
                    return None;
                }
                vk = Some(0x6F + n);
            }
            _ => return None,
        }
    }
    // a bare key would steal it from every app
    if mods & 0xF == 0 {
        return None;
    }
    Some((mods, vk?))
}

/// The overlay shortcut, as shown in the tray menu
pub fn overlay_key() -> String {
    crate::settings::text("overlay_key").filter(|k| parse_key(k).is_some()).unwrap_or_else(|| DEFAULT_KEY.to_string())
}

fn toggle_overlay() {
    if let Some((_, ctx)) = APP.get() {
        crate::ui::overlay::toggle(ctx);
    }
}
const WM_LBUTTONUP: isize = 0x0202;
const WM_RBUTTONUP: isize = 0x0205;
const NIN_BALLOONUSERCLICK: isize = 0x0405;
const CLASS: &str = "PurgeonTray";

static HWND: AtomicIsize = AtomicIsize::new(0);
static ICON: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
static APP: OnceLock<(Arc<Shared>, egui::Context)> = OnceLock::new();

/// True once the tray icon is showing; closing the window then hides to the tray
pub fn active() -> bool {
    HWND.load(Ordering::Relaxed) != 0
}

/// If another Purgeon is already running, asks it to show its window and returns false (this
/// copy should then exit). `--relaunched` (the elevated copy started by Elevate) skips the check,
/// since the copy that started it is about to exit.
pub fn first_instance() -> bool {
    unsafe {
        let m = CreateMutexW(std::ptr::null(), 0, wide("Local\\Purgeon.SingleInstance").as_ptr());
        // 183: already exists; 5: exists but belongs to an elevated copy
        let err = GetLastError();
        if std::env::args().any(|a| a == "--relaunched") {
            return true;
        }
        if m != 0 && err != 183 {
            return true;
        }
        if m == 0 && err != 5 {
            return true;
        }
        let h = FindWindowW(wide(CLASS).as_ptr(), std::ptr::null());
        if h == 0 {
            // the other copy has no tray icon (yet); run normally
            return true;
        }
        PostMessageW(h, WM_COMMAND_PURGEON, 0, 0);
        false
    }
}

/// Starts the tray icon on its own thread
pub fn start(shared: Arc<Shared>, ctx: egui::Context) {
    let _ = APP.set((shared, ctx));
    let _ = std::thread::Builder::new().name("purgeon-tray".into()).spawn(|| unsafe { run() });
}

unsafe fn run() {
    let inst = GetModuleHandleW(std::ptr::null());
    let class = wide(CLASS);
    let wc = WndClassEx {
        cb_size: std::mem::size_of::<WndClassEx>() as u32,
        style: 0,
        wnd_proc: wnd_proc,
        cls_extra: 0,
        wnd_extra: 0,
        instance: inst,
        icon: 0,
        cursor: 0,
        background: 0,
        menu_name: std::ptr::null(),
        class_name: class.as_ptr(),
        icon_sm: 0,
    };
    if RegisterClassExW(&wc) == 0 {
        return;
    }
    // A hidden top-level window (not message-only) so it also hears "TaskbarCreated" when
    // Explorer restarts, and can put the icon back
    let h = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, 0, 0, inst, std::ptr::null_mut());
    if h == 0 {
        return;
    }
    let tc = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
    let _ = TASKBAR_CREATED.set(tc);
    // When Purgeon runs as administrator, still accept these from Explorer and from a second,
    // non-elevated copy (MSGFLT_ALLOW)
    for m in [tc, WM_TRAY, WM_COMMAND_PURGEON] {
        ChangeWindowMessageFilterEx(h, m, 1, std::ptr::null_mut());
    }
    // The exe's own icon (resource 1), at the small-icon size for this screen's scaling
    let (cx, cy) = (GetSystemMetrics(49), GetSystemMetrics(50));
    let mut icon = LoadImageW(inst, 1 as *const u16, 1, cx, cy, 0);
    if icon == 0 {
        icon = LoadIconW(0, 32512 as *const u16);
    }
    ICON.store(icon, Ordering::Relaxed);
    if !add(h) {
        return;
    }
    HWND.store(h, Ordering::Relaxed);
    // The overlay shortcut. RegisterHotKey only gets told about that one key combination; it
    // doesn't watch the keyboard, so it is safe next to anti-cheat.
    let key = overlay_key();
    if let Some((mods, vk)) = parse_key(&key) {
        if RegisterHotKey(h, 1, mods, vk) == 0 {
            notify(
                "The overlay shortcut is taken",
                &format!("Another app already uses {key}. You can still open the overlay from Purgeon's tray icon."),
                false,
            );
        }
    }
    let mut msg: Msg = std::mem::zeroed();
    while GetMessageW(&mut msg, 0, 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

fn base(h: isize) -> NotifyIconData {
    // all-zero is a valid empty value for every field
    let mut d: NotifyIconData = unsafe { std::mem::zeroed() };
    d.cb_size = std::mem::size_of::<NotifyIconData>() as u32;
    d.hwnd = h;
    d.id = 1;
    d
}

fn copy(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

unsafe fn add(h: isize) -> bool {
    let mut d = base(h);
    // NIF_MESSAGE | NIF_ICON | NIF_TIP
    d.flags = 1 | 2 | 4;
    d.callback_msg = WM_TRAY;
    d.icon = ICON.load(Ordering::Relaxed);
    copy(&mut d.tip, crate::TITLE);
    Shell_NotifyIconW(0, &d) != 0
}

/// Takes the icon out of the tray (before Purgeon exits, or it lingers until hovered)
pub fn remove() {
    let h = HWND.swap(0, Ordering::Relaxed);
    if h != 0 {
        unsafe { Shell_NotifyIconW(2, &base(h)) };
    }
}

/// The icon's hover text, e.g. "Purgeon · CPU 12% · RAM 48%"
pub fn set_tip(text: &str) {
    let h = HWND.load(Ordering::Relaxed);
    if h == 0 {
        return;
    }
    let mut d = base(h);
    d.flags = 4;
    copy(&mut d.tip, text);
    unsafe { Shell_NotifyIconW(1, &d) };
}

/// Shows a Windows notification from the tray icon. `warn` picks the warning icon.
pub fn notify(title: &str, body: &str, warn: bool) {
    let h = HWND.load(Ordering::Relaxed);
    if h == 0 {
        return;
    }
    let mut d = base(h);
    // NIF_INFO
    d.flags = 0x10;
    copy(&mut d.info_title, title);
    copy(&mut d.info, body);
    // NIIF_WARNING or NIIF_INFO, plus NIIF_RESPECT_QUIET_TIME
    d.info_flags = (if warn { 2 } else { 1 }) | 0x80;
    unsafe { Shell_NotifyIconW(1, &d) };
}

/// Brings the main window back from the tray (or from minimized) and wakes the UI
pub fn show_window() {
    let Some((shared, ctx)) = APP.get() else { return };
    shared.hidden.store(false, Ordering::Relaxed);
    shared.visible.store(true, Ordering::Relaxed);
    crate::platform::show_window();
    ctx.request_repaint();
}

const ID_OPEN: usize = 1;
const ID_ALERTS: usize = 2;
const ID_QUIT: usize = 3;
const ID_OVERLAY: usize = 4;

unsafe fn menu(h: isize) {
    let m = CreatePopupMenu();
    if m == 0 {
        return;
    }
    // MF_STRING 0, MF_CHECKED 8, MF_SEPARATOR 0x800
    AppendMenuW(m, 0, ID_OPEN, wide("Open Purgeon").as_ptr());
    let ov = if crate::ui::overlay::is_open() { "Hide overlay" } else { "Show overlay" };
    AppendMenuW(m, 0, ID_OVERLAY, wide(&format!("{ov}\t{}", overlay_key())).as_ptr());
    AppendMenuW(m, if crate::alerts::enabled() { 8 } else { 0 }, ID_ALERTS, wide("Warn me when something's wrong").as_ptr());
    AppendMenuW(m, 0x800, 0, std::ptr::null());
    AppendMenuW(m, 0, ID_QUIT, wide("Quit Purgeon").as_ptr());
    SetMenuDefaultItem(m, ID_OPEN as u32, 0);
    let mut pt = [0i32; 2];
    GetCursorPos(&mut pt);
    // the menu only closes on an outside click if our window is in the foreground
    SetForegroundWindow(h);
    // TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY | TPM_BOTTOMALIGN
    let cmd = TrackPopupMenu(m, 0x2 | 0x100 | 0x80 | 0x20, pt[0], pt[1], 0, h, std::ptr::null()) as usize;
    PostMessageW(h, 0, 0, 0);
    DestroyMenu(m);
    match cmd {
        ID_OPEN => show_window(),
        ID_OVERLAY => toggle_overlay(),
        ID_ALERTS => crate::alerts::set_enabled(!crate::alerts::enabled()),
        ID_QUIT => crate::quit(),
        _ => {}
    }
}

unsafe extern "system" fn wnd_proc(h: isize, m: u32, w: usize, l: isize) -> isize {
    if m == WM_TRAY {
        match l & 0xffff {
            WM_LBUTTONUP | NIN_BALLOONUSERCLICK => show_window(),
            WM_RBUTTONUP => menu(h),
            _ => {}
        }
        return 0;
    }
    if m == WM_HOTKEY {
        toggle_overlay();
        return 0;
    }
    if m == WM_COMMAND_PURGEON {
        if w == 1 {
            crate::quit();
        }
        if w == 2 {
            toggle_overlay();
            return 0;
        }
        show_window();
        return 0;
    }
    if Some(&m) == TASKBAR_CREATED.get() {
        add(h);
        return 0;
    }
    DefWindowProcW(h, m, w, l)
}
