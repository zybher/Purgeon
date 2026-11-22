//! "Start with Windows": a value under the user's own Run key (HKCU, no administrator rights),
//! which starts Purgeon in the tray at sign-in. Off unless the user turns it on in Settings.

use crate::sys::win::wide;
use std::ffi::c_void;

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(key: isize, sub: *const u16, opts: u32, access: u32, out: *mut isize) -> i32;
    fn RegQueryValueExW(key: isize, name: *const u16, res: *mut u32, ty: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
    fn RegSetValueExW(key: isize, name: *const u16, res: u32, ty: u32, data: *const c_void, len: u32) -> i32;
    fn RegDeleteValueW(key: isize, name: *const u16) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}

const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const NAME: &str = "Purgeon";
const KEY_QUERY_VALUE: u32 = 0x1;
const KEY_SET_VALUE: u32 = 0x2;
const REG_SZ: u32 = 1;

/// The command the Run key should hold: this exe, started in the tray
fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" --tray", exe.display()))
}

fn open(access: u32) -> Option<isize> {
    let sub = wide(RUN);
    let mut k = 0isize;
    (unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, access, &mut k) } == 0).then_some(k)
}

/// What the Run key holds for Purgeon, if anything
fn current() -> Option<String> {
    let k = open(KEY_QUERY_VALUE)?;
    let name = wide(NAME);
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let mut ty = 0u32;
    let r = unsafe { RegQueryValueExW(k, name.as_ptr(), std::ptr::null_mut(), &mut ty, buf.as_mut_ptr() as _, &mut len) };
    unsafe { RegCloseKey(k) };
    if r != 0 || ty != REG_SZ {
        return None;
    }
    let n = (len as usize / 2).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_string())
}

pub fn is_on() -> bool {
    current().is_some()
}

/// Turns it on or off; false if Windows refused
pub fn set(on: bool) -> bool {
    let Some(k) = open(KEY_SET_VALUE) else { return false };
    let name = wide(NAME);
    let ok = if on {
        match command() {
            Some(c) => {
                let v = wide(&c);
                unsafe { RegSetValueExW(k, name.as_ptr(), 0, REG_SZ, v.as_ptr() as _, (v.len() * 2) as u32) == 0 }
            }
            None => false,
        }
    } else {
        let r = unsafe { RegDeleteValueW(k, name.as_ptr()) };
        r == 0 || r == 2 // ERROR_FILE_NOT_FOUND: already off
    };
    unsafe { RegCloseKey(k) };
    ok
}

/// When it is on but points at another copy of Purgeon (the exe was moved or rebuilt elsewhere),
/// point it at this one
pub fn refresh() {
    if let (Some(cur), Some(want)) = (current(), command()) {
        if cur != want {
            set(true);
        }
    }
}

/// Started by the Run key (or with `--tray`): open in the tray, not on screen
pub fn started_in_tray() -> bool {
    std::env::args().any(|a| a == "--tray")
}
