//! What the details panel looks up about one process, only while it is open: the command line
//! it was started with, the Windows services it hosts, what its file says about itself, its
//! priority, and when it started in local time.
//!
//! Same safety policy as the rest of Purgeon (see `win::PROCESS_QUERY_LIMITED_INFORMATION`):
//! a query-only handle, never reading or writing the process's memory. The command line comes
//! from the kernel (ProcessCommandLineInformation), the way Task Manager's own column gets it.
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use super::win::{self, wide, FILETIME, UNICODE_STRING};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;
/// Needed only to change priority, asked for only when the user picks a new one
const PROCESS_SET_INFORMATION: u32 = 0x0200;

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationProcess(h: isize, class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SYSTEMTIME {
    year: u16,
    month: u16,
    dow: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    ms: u16,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetPriorityClass(h: isize) -> u32;
    fn SetPriorityClass(h: isize, class: u32) -> i32;
    fn GetSystemTimeAsFileTime(ft: *mut FILETIME);
    fn FileTimeToSystemTime(ft: *const FILETIME, st: *mut SYSTEMTIME) -> i32;
    fn SystemTimeToTzSpecificLocalTime(tz: *const c_void, utc: *const SYSTEMTIME, local: *mut SYSTEMTIME) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ENUM_SERVICE_STATUS_PROCESSW {
    service_name: *const u16,
    display_name: *const u16,
    service_type: u32,
    current_state: u32,
    controls_accepted: u32,
    exit_code: u32,
    specific_exit_code: u32,
    check_point: u32,
    wait_hint: u32,
    process_id: u32,
    service_flags: u32,
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenSCManagerW(machine: *const u16, db: *const u16, access: u32) -> isize;
    fn EnumServicesStatusExW(
        h: isize,
        level: u32,
        ty: u32,
        state: u32,
        buf: *mut u8,
        size: u32,
        needed: *mut u32,
        returned: *mut u32,
        resume: *mut u32,
        group: *const u16,
    ) -> i32;
    fn CloseServiceHandle(h: isize) -> i32;
}

unsafe fn wstr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 && n < 4096 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

/// The command line the process was started with. None when Windows won't say (protected
/// processes, and other users' processes without administrator).
pub fn command_line(pid: u32) -> Option<String> {
    if pid <= 4 {
        return None;
    }
    unsafe {
        let h = win::OpenProcess(win::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h == 0 {
            return None;
        }
        let mut buf = vec![0u64; 1024];
        let mut out = None;
        for _ in 0..3 {
            let mut ret = 0u32;
            let st = NtQueryInformationProcess(h, PROCESS_COMMAND_LINE_INFORMATION, buf.as_mut_ptr() as _, (buf.len() * 8) as u32, &mut ret);
            if st == win::STATUS_INFO_LENGTH_MISMATCH || st == 0x8000_0005_u32 as i32 || st == 0xC000_0023_u32 as i32 {
                buf.resize((ret as usize).div_ceil(8) + 16, 0);
                continue;
            }
            if st >= 0 {
                let us = std::ptr::read_unaligned(buf.as_ptr() as *const UNICODE_STRING);
                if !us.Buffer.is_null() && us.Length > 0 {
                    let s = String::from_utf16_lossy(std::slice::from_raw_parts(us.Buffer, us.Length as usize / 2));
                    out = Some(s.trim().to_string()).filter(|s| !s.is_empty());
                }
            }
            break;
        }
        win::CloseHandle(h);
        out
    }
}

/// Windows priority classes, lowest first, with the names Task Manager uses. Realtime is left
/// out on purpose: it can starve the mouse and keyboard.
pub const PRIORITIES: [(u32, &str); 5] = [(0x40, "Low"), (0x4000, "Below normal"), (0x20, "Normal"), (0x8000, "Above normal"), (0x80, "High")];

pub fn priority_name(class: u32) -> &'static str {
    match class {
        0x100 => "Realtime",
        c => PRIORITIES.iter().find(|p| p.0 == c).map_or("Unknown", |p| p.1),
    }
}

pub fn priority(pid: u32) -> Option<u32> {
    unsafe {
        let h = win::OpenProcess(win::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h == 0 {
            return None;
        }
        let c = GetPriorityClass(h);
        win::CloseHandle(h);
        (c != 0).then_some(c)
    }
}

/// Changes one process's priority after the user picked it. The handle is opened with just
/// the right to do that and closed straight away.
pub fn set_priority(pid: u32, class: u32) -> Result<(), String> {
    unsafe {
        let h = win::OpenProcess(PROCESS_SET_INFORMATION, 0, pid);
        if h == 0 {
            return Err(match win::GetLastError() {
                5 => "Access denied. Click Elevate to change programs that run as administrator.".into(),
                87 => "The process has already exited.".into(),
                e => format!("Windows refused (error {e})."),
            });
        }
        let ok = SetPriorityClass(h, class) != 0;
        let err = win::GetLastError();
        win::CloseHandle(h);
        if ok {
            Ok(())
        } else {
            Err(format!("Windows refused (error {err})."))
        }
    }
}

/// Running Windows services by the process that hosts them: pid -> display names. Read at
/// most every few seconds.
type Services = HashMap<u32, Vec<String>>;

pub fn services() -> Services {
    static CACHE: Mutex<Option<(Instant, Services)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, m)) = c.as_ref() {
        if at.elapsed() < Duration::from_secs(4) {
            return m.clone();
        }
    }
    let m = read_services();
    *c = Some((Instant::now(), m.clone()));
    m
}

fn read_services() -> HashMap<u32, Vec<String>> {
    let mut out: HashMap<u32, Vec<String>> = HashMap::new();
    unsafe {
        // SC_MANAGER_ENUMERATE_SERVICE
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), 0x0004);
        if scm == 0 {
            return out;
        }
        // every service type (includes per-user services on Windows 10+), else plain Win32 ones
        for ty in [0x3FF_u32, 0x30] {
            let mut buf = vec![0u64; 64 * 1024];
            let mut resume = 0u32;
            let mut ok = false;
            loop {
                let (mut needed, mut n) = (0u32, 0u32);
                // SC_ENUM_PROCESS_INFO, SERVICE_ACTIVE
                let r = EnumServicesStatusExW(scm, 0, ty, 1, buf.as_mut_ptr() as _, (buf.len() * 8) as u32, &mut needed, &mut n, &mut resume, std::ptr::null());
                let more = r == 0 && win::GetLastError() == 234; // ERROR_MORE_DATA
                if r == 0 && !more {
                    break;
                }
                ok = true;
                let rows = std::slice::from_raw_parts(buf.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW, n as usize);
                for row in rows {
                    if row.process_id == 0 {
                        continue;
                    }
                    let mut name = wstr(row.display_name);
                    if name.is_empty() {
                        name = wstr(row.service_name);
                    }
                    let v = out.entry(row.process_id).or_default();
                    if !v.contains(&name) {
                        v.push(name);
                    }
                }
                if !more {
                    break;
                }
                if needed as usize > buf.len() * 8 {
                    buf.resize((needed as usize).div_ceil(8) + 1024, 0);
                }
            }
            if ok {
                break;
            }
        }
        CloseServiceHandle(scm);
    }
    for v in out.values_mut() {
        v.sort();
    }
    out
}

/// What a program's file says about itself
#[derive(Clone, Default)]
pub struct FileInfo {
    pub company: Option<String>,
    pub description: Option<String>,
    pub product: Option<String>,
    pub version: Option<String>,
}

#[link(name = "version")]
extern "system" {
    fn GetFileVersionInfoSizeW(file: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoW(file: *const u16, handle: u32, len: u32, data: *mut u8) -> i32;
    fn VerQueryValueW(block: *const u8, sub: *const u16, out: *mut *const u16, len: *mut u32) -> i32;
}

/// Company, description, product and version from the .exe's version resource (cached)
pub fn file_info(path: &str) -> FileInfo {
    static CACHE: Mutex<Option<HashMap<String, FileInfo>>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let c = c.get_or_insert_with(HashMap::new);
    if let Some(f) = c.get(path) {
        return f.clone();
    }
    let f = read_file_info(path).unwrap_or_default();
    if c.len() > 512 {
        c.clear();
    }
    c.insert(path.to_string(), f.clone());
    f
}

fn read_file_info(path: &str) -> Option<FileInfo> {
    let file = wide(path);
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
            if VerQueryValueW(data.as_ptr(), wide(sub).as_ptr(), &mut p, &mut n) == 0 || p.is_null() || n == 0 {
                return None;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n as usize));
            let s = s.trim_end_matches('\0').trim().to_string();
            (!s.is_empty()).then_some(s)
        };
        let mut langs = vec![];
        let (mut p, mut n) = (std::ptr::null::<u16>(), 0u32);
        if VerQueryValueW(data.as_ptr(), wide("\\VarFileInfo\\Translation").as_ptr(), &mut p, &mut n) != 0 && n >= 4 {
            langs.push(format!("{:04x}{:04x}", *p, *p.add(1)));
        }
        langs.extend(["040904b0".to_string(), "040904e4".to_string(), "000004b0".to_string()]);
        let get = |key: &str| langs.iter().find_map(|l| query(&format!("\\StringFileInfo\\{l}\\{key}")));
        Some(FileInfo {
            company: get("CompanyName"),
            description: get("FileDescription"),
            product: get("ProductName"),
            version: get("ProductVersion").or_else(|| get("FileVersion")),
        })
    }
}

/// Now, in Windows' file time (100 ns ticks since 1601, UTC)
pub fn now_filetime() -> i64 {
    let mut ft = FILETIME::default();
    unsafe { GetSystemTimeAsFileTime(&mut ft) };
    ft.v() as i64
}

/// A start time in local time: "14:02" today, "Yesterday 09:41", or "21 Nov 09:41"
pub fn local_time(ft: i64) -> Option<String> {
    if ft <= 0 {
        return None;
    }
    let to_local = |t: i64| -> Option<SYSTEMTIME> {
        let f = FILETIME { lo: t as u32, hi: (t >> 32) as u32 };
        let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
        unsafe { (FileTimeToSystemTime(&f, &mut utc) != 0 && SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) != 0).then_some(local) }
    };
    let t = to_local(ft)?;
    let now = to_local(now_filetime())?;
    let hm = format!("{:02}:{:02}", t.hour, t.minute);
    let same = |a: &SYSTEMTIME, b: &SYSTEMTIME| a.year == b.year && a.month == b.month && a.day == b.day;
    if same(&t, &now) {
        return Some(format!("today at {hm}"));
    }
    let yesterday = to_local(now_filetime() - 864_000_000_000)?;
    if same(&t, &yesterday) {
        return Some(format!("yesterday at {hm}"));
    }
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mon = MONTHS.get(t.month.saturating_sub(1) as usize).copied().unwrap_or("");
    Some(format!("{} {mon} at {hm}", t.day))
}
