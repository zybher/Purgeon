//! Window styling through DWM, and the app icon (drawn in code, no asset files).

use crate::sys::win;
use std::sync::atomic::{AtomicIsize, Ordering};

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(f: unsafe extern "system" fn(isize, isize) -> i32, l: isize) -> i32;
    fn GetWindowThreadProcessId(h: isize, pid: *mut u32) -> u32;
    fn GetWindowTextW(h: isize, buf: *mut u16, n: i32) -> i32;
    fn IsWindow(h: isize) -> i32;
}

static MAIN: AtomicIsize = AtomicIsize::new(0);

/// Purgeon's own main window: titled "Purgeon" and owned by this process (a File Explorer
/// window open on a folder called Purgeon has the same title). 0 until it exists.
pub fn main_window() -> isize {
    let h = MAIN.load(Ordering::Relaxed);
    if h != 0 && unsafe { IsWindow(h) } != 0 {
        return h;
    }
    let h = find_window(crate::TITLE);
    MAIN.store(h, Ordering::Relaxed);
    h
}

/// A top-level window of this process with exactly this title, 0 if there is none
pub fn find_window(title: &str) -> isize {
    struct Find<'a> {
        title: &'a str,
        found: isize,
    }
    unsafe extern "system" fn each(h: isize, l: isize) -> i32 {
        let f = &mut *(l as *mut Find);
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, &mut pid);
        if pid != std::process::id() {
            return 1;
        }
        let mut buf = [0u16; 64];
        let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        if String::from_utf16_lossy(&buf[..n]) == f.title {
            f.found = h;
            return 0;
        }
        1
    }
    let mut f = Find { title, found: 0 };
    unsafe { EnumWindows(each, &mut f as *mut Find as isize) };
    f.found
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Placement {
    length: u32,
    flags: u32,
    show: u32,
    min: [i32; 2],
    max: [i32; 2],
    normal: [i32; 4],
}

#[link(name = "user32")]
extern "system" {
    fn ShowWindow(h: isize, cmd: i32) -> i32;
    fn SetForegroundWindow(h: isize) -> i32;
    fn GetWindowPlacement(h: isize, p: *mut Placement) -> i32;
    fn SetWindowPlacement(h: isize, p: *const Placement) -> i32;
    fn GetWindowLongPtrW(h: isize, i: i32) -> isize;
    fn SetWindowLongPtrW(h: isize, i: i32, v: isize) -> isize;
}

/// Where the window was and its extended style, while it sits in the tray
static PARKED: std::sync::Mutex<Option<(Placement, isize)>> = std::sync::Mutex::new(None);
const GWL_EXSTYLE: i32 = -20;
const WS_EX_TOOLWINDOW: isize = 0x80;
const WS_EX_APPWINDOW: isize = 0x40000;

/// Puts the main window away (to the tray): gone from the screen, the taskbar and Alt+Tab.
///
/// It is parked far off-screen as a tool window rather than hidden: eframe asks Windows to
/// redraw a window it believes is showing, a hidden window never gets that redraw, and eframe
/// then busy-waits for it on a full CPU core. Parked, any redraw just happens, unseen.
pub fn hide_window() {
    let h = main_window();
    let mut park = PARKED.lock().unwrap_or_else(|e| e.into_inner());
    if h == 0 || park.is_some() {
        return;
    }
    unsafe {
        let mut p = Placement { length: std::mem::size_of::<Placement>() as u32, ..Default::default() };
        if GetWindowPlacement(h, &mut p) == 0 {
            return;
        }
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        *park = Some((p, ex));
        // hidden while the style changes, so the taskbar drops the button
        ShowWindow(h, 0);
        SetWindowLongPtrW(h, GWL_EXSTYLE, (ex | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW);
        let [l, t, r, b] = p.normal;
        let off = Placement { show: 4, normal: [-32000, -32000, -32000 + (r - l), -32000 + (b - t)], flags: 0, ..p };
        // SW_SHOWNOACTIVATE: shown (to Windows) at the off-screen spot, without taking focus
        SetWindowPlacement(h, &off);
    }
}

/// Brings the main window back from the tray to where it was, in front
pub fn show_window() {
    let h = main_window();
    if h == 0 {
        return;
    }
    let parked = PARKED.lock().unwrap_or_else(|e| e.into_inner()).take();
    unsafe {
        if let Some((mut p, ex)) = parked {
            ShowWindow(h, 0);
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex);
            // back maximized if it was, otherwise normal (also when it was minimized)
            p.show = if p.show == 3 { 3 } else { 1 };
            SetWindowPlacement(h, &p);
        } else {
            // SW_RESTORE: e.g. minimized and Purgeon was started again
            ShowWindow(h, 9);
        }
        // allowed here: the user just clicked our icon (or started Purgeon again)
        SetForegroundWindow(h);
    }
}

/// Rounds the borderless window's corners (Windows 11), gives it a dark border and dark
/// system chrome. Returns false until the window exists.
pub fn style_window() -> bool {
    let h = main_window();
    if h == 0 {
        return false;
    }
    unsafe {
        let on: u32 = 1;
        // DWMWA_USE_IMMERSIVE_DARK_MODE
        win::DwmSetWindowAttribute(h, 20, &on as *const u32 as _, 4);
        // DWMWA_WINDOW_CORNER_PREFERENCE: round
        let round: u32 = 2;
        win::DwmSetWindowAttribute(h, 33, &round as *const u32 as _, 4);
        // DWMWA_BORDER_COLOR (COLORREF 0x00BBGGRR)
        let border: u32 = 0x002b2b2b;
        win::DwmSetWindowAttribute(h, 34, &border as *const u32 as _, 4);
    }
    true
}

#[link(name = "version")]
extern "system" {
    fn GetFileVersionInfoSizeW(file: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoW(file: *const u16, handle: u32, len: u32, data: *mut u8) -> i32;
    fn VerQueryValueW(block: *const u8, sub: *const u16, out: *mut *const u16, len: *mut u32) -> i32;
}

/// The name an app gives itself inside its .exe ("Roblox", "Google Chrome", "Blender"), read
/// from the file on disk (never from the running process). None when it has no usable one.
pub fn app_name(path: &str) -> Option<String> {
    use std::collections::HashMap;
    static CACHE: std::sync::Mutex<Option<HashMap<String, Option<String>>>> = std::sync::Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let c = c.get_or_insert_with(HashMap::new);
    if let Some(n) = c.get(path) {
        return n.clone();
    }
    let n = read_app_name(path);
    c.insert(path.to_string(), n.clone());
    n
}

fn read_app_name(path: &str) -> Option<String> {
    let file = win::wide(path);
    unsafe {
        let mut h = 0;
        let size = GetFileVersionInfoSizeW(file.as_ptr(), &mut h);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(file.as_ptr(), 0, size, data.as_mut_ptr()) == 0 {
            return None;
        }
        let query = |sub: &str| -> Option<String> {
            let (mut p, mut n) = (std::ptr::null(), 0u32);
            if VerQueryValueW(data.as_ptr(), win::wide(sub).as_ptr(), &mut p, &mut n) == 0 || p.is_null() || n == 0 {
                return None;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n as usize));
            let s = s.trim_end_matches('\0').trim().to_string();
            (!s.is_empty()).then_some(s)
        };
        // the file's own language first, then the usual English ones
        let mut langs = vec![];
        let (mut p, mut n) = (std::ptr::null::<u16>(), 0u32);
        if VerQueryValueW(data.as_ptr(), win::wide("\\VarFileInfo\\Translation").as_ptr(), &mut p, &mut n) != 0 && n >= 4 {
            langs.push(format!("{:04x}{:04x}", *p, *p.add(1)));
        }
        langs.extend(["040904b0".to_string(), "040904e4".to_string()]);
        let get = |key: &str| langs.iter().find_map(|l| query(&format!("\\StringFileInfo\\{l}\\{key}")));
        // The product name is the friendliest ("Roblox" rather than "Roblox Game Client"), but
        // Windows' own programs all share one long product name; their description is better
        let product = get("ProductName").filter(|s| s.chars().count() <= 24 && !s.contains('®') && !s.contains("(R)"));
        product.or_else(|| get("FileDescription").filter(|s| s.chars().count() <= 32))
    }
}

/// Dark rounded corners and border for a small borderless window such as the overlay
pub fn style_popup(h: isize) {
    unsafe {
        let on: u32 = 1;
        win::DwmSetWindowAttribute(h, 20, &on as *const u32 as _, 4);
        let round: u32 = 2;
        win::DwmSetWindowAttribute(h, 33, &round as *const u32 as _, 4);
        let border: u32 = 0x00333333;
        win::DwmSetWindowAttribute(h, 34, &border as *const u32 as _, 4);
    }
}

/// The app icon (chip on a dark tile) for the window and taskbar. Pre-rendered from
/// assets/purgeon-icon.svg by assets/make_icons.py as raw 128 px RGBA, so no image decoder ships.
pub fn app_icon() -> eframe::egui::IconData {
    const S: u32 = 128;
    let rgba = include_bytes!("../assets/icon-128.rgba").to_vec();
    debug_assert_eq!(rgba.len(), (S * S * 4) as usize);
    eframe::egui::IconData { rgba, width: S, height: S }
}

/// Hands every page we are not actively using back to Windows (startup-only pages such as the
/// shader compile and font build). The working set then regrows only to what drawing needs.
pub fn trim_memory() {
    unsafe { win::SetProcessWorkingSetSize(win::GetCurrentProcess(), usize::MAX, usize::MAX) };
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(hwnd: isize, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
}

/// Starts a new copy of Purgeon through the UAC prompt, passing `state` (such as
/// `--page=disk`) so it opens where this one was. Arguments with the same `--name=` are
/// replaced. True if it launched (the caller then exits); false if the user declined or it failed.
pub fn relaunch_as_admin(state: &[String]) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let key = |a: &str| a.split('=').next().unwrap_or("").to_string();
    let keys: Vec<String> = state.iter().map(|a| key(a)).collect();
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !keys.contains(&key(a)))
        .chain(state.iter().cloned())
        .filter(|a| a != "--relaunched")
        .chain(Some("--relaunched".to_string()))
        .map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a })
        .collect();
    let (op, file, params) = (win::wide("runas"), win::wide(&exe.to_string_lossy()), win::wide(&args.join(" ")));
    let r = unsafe { ShellExecuteW(0, op.as_ptr(), file.as_ptr(), params.as_ptr(), std::ptr::null(), 1) };
    r > 32
}

/// True when Windows' "Animation effects" setting is off (Settings > Accessibility > Visual effects)
pub fn reduce_motion() -> bool {
    let mut on: i32 = 1;
    // SPI_GETCLIENTAREAANIMATION
    let ok = unsafe { win::SystemParametersInfoW(0x1042, 0, &mut on as *mut i32 as _, 0) } != 0;
    ok && on == 0
}

/// Processes Purgeon refuses to end: ending them crashes or logs off Windows
const PROTECTED: [&str; 12] = [
    "system", "registry", "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe", "services.exe",
    "lsass.exe", "lsaiso.exe", "memcompression", "secure system", "fontdrvhost.exe",
];

/// Why a process may not be ended from Purgeon, if it may not
pub fn end_blocked(pid: u32, name: &str) -> Option<&'static str> {
    if pid <= 4 || PROTECTED.contains(&name.to_lowercase().as_str()) {
        return Some("Windows needs this process. Ending it would crash or sign you out.");
    }
    if pid == std::process::id() {
        return Some("This is Purgeon itself. Close the window instead.");
    }
    None
}

/// Ends one process after the user confirmed. Opens it with PROCESS_TERMINATE only now,
/// refuses critical processes (IsProcessCritical), and closes the handle straight away.
pub fn end_process(pid: u32, name: &str) -> Result<(), String> {
    if let Some(why) = end_blocked(pid, name) {
        return Err(why.into());
    }
    unsafe {
        let h = win::OpenProcess(win::PROCESS_TERMINATE | win::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h == 0 {
            return Err(match win::GetLastError() {
                5 => "Access denied. Click Elevate to end processes that run as administrator.".into(),
                87 => "The process has already exited.".into(),
                e => format!("Windows refused (error {e})."),
            });
        }
        let mut critical = 0i32;
        if win::IsProcessCritical(h, &mut critical) != 0 && critical != 0 {
            win::CloseHandle(h);
            return Err("Windows marks this process as critical. Ending it would crash Windows.".into());
        }
        let ok = win::TerminateProcess(h, 1) != 0;
        let err = win::GetLastError();
        win::CloseHandle(h);
        if ok {
            Ok(())
        } else {
            Err(if err == 5 { "Access denied. The process may be protected.".into() } else { format!("Windows refused (error {err}).") })
        }
    }
}
