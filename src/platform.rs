//! Window styling through DWM, and the app icon (drawn in code, no asset files).

use crate::sys::win;

/// Rounds the borderless window's corners (Windows 11), gives it a dark border and dark
/// system chrome. Returns false until the window exists.
pub fn style_window() -> bool {
    let title: Vec<u16> = crate::TITLE.encode_utf16().chain(Some(0)).collect();
    let h = unsafe { win::FindWindowW(std::ptr::null(), title.as_ptr()) };
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
