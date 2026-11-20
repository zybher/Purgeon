//! Hand-written Win32 / NT declarations (no windows-sys dependency, keeps builds small).
#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FILETIME {
    pub lo: u32,
    pub hi: u32,
}
impl FILETIME {
    pub fn v(&self) -> u64 {
        ((self.hi as u64) << 32) | self.lo as u64
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UNICODE_STRING {
    pub Length: u16,
    pub MaximumLength: u16,
    pub Buffer: *const u16,
}

/// The full (phnt) layout of SystemProcessInformation's records
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SYSTEM_PROCESS_INFORMATION {
    pub NextEntryOffset: u32,
    pub NumberOfThreads: u32,
    pub WorkingSetPrivateSize: i64,
    pub HardFaultCount: u32,
    pub NumberOfThreadsHighWatermark: u32,
    pub CycleTime: u64,
    pub CreateTime: i64,
    pub UserTime: i64,
    pub KernelTime: i64,
    pub ImageName: UNICODE_STRING,
    pub BasePriority: i32,
    pub UniqueProcessId: usize,
    pub InheritedFromUniqueProcessId: usize,
    pub HandleCount: u32,
    pub SessionId: u32,
    pub UniqueProcessKey: usize,
    pub PeakVirtualSize: usize,
    pub VirtualSize: usize,
    pub PageFaultCount: u32,
    pub PeakWorkingSetSize: usize,
    pub WorkingSetSize: usize,
    pub QuotaPeakPagedPoolUsage: usize,
    pub QuotaPagedPoolUsage: usize,
    pub QuotaPeakNonPagedPoolUsage: usize,
    pub QuotaNonPagedPoolUsage: usize,
    pub PagefileUsage: usize,
    pub PeakPagefileUsage: usize,
    pub PrivatePageCount: usize,
    pub ReadOperationCount: i64,
    pub WriteOperationCount: i64,
    pub OtherOperationCount: i64,
    pub ReadTransferCount: i64,
    pub WriteTransferCount: i64,
    pub OtherTransferCount: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION {
    pub IdleTime: i64,
    pub KernelTime: i64,
    pub UserTime: i64,
    pub DpcTime: i64,
    pub InterruptTime: i64,
    pub InterruptCount: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SYSTEM_MEMORY_LIST_INFORMATION {
    pub ZeroPageCount: usize,
    pub FreePageCount: usize,
    pub ModifiedPageCount: usize,
    pub ModifiedNoWritePageCount: usize,
    pub BadPageCount: usize,
    pub PageCountByPriority: [usize; 8],
    pub RepurposedPagesByPriority: [usize; 8],
    pub ModifiedPageCountPageFile: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MEMORYSTATUSEX {
    pub dwLength: u32,
    pub dwMemoryLoad: u32,
    pub ullTotalPhys: u64,
    pub ullAvailPhys: u64,
    pub ullTotalPageFile: u64,
    pub ullAvailPageFile: u64,
    pub ullTotalVirtual: u64,
    pub ullAvailVirtual: u64,
    pub ullAvailExtendedVirtual: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PERFORMANCE_INFORMATION {
    pub cb: u32,
    pub CommitTotal: usize,
    pub CommitLimit: usize,
    pub CommitPeak: usize,
    pub PhysicalTotal: usize,
    pub PhysicalAvailable: usize,
    pub SystemCache: usize,
    pub KernelTotal: usize,
    pub KernelPaged: usize,
    pub KernelNonpaged: usize,
    pub PageSize: usize,
    pub HandleCount: u32,
    pub ProcessCount: u32,
    pub ThreadCount: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LUID {
    pub LowPart: u32,
    pub HighPart: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TOKEN_PRIVILEGES {
    pub PrivilegeCount: u32,
    pub Luid: LUID,
    pub Attributes: u32,
}

pub const SYSTEM_PROCESS_INFORMATION_CLASS: u32 = 5;
pub const SYSTEM_PROCESSOR_PERFORMANCE_CLASS: u32 = 8;
pub const SYSTEM_MEMORY_LIST_CLASS: u32 = 80;
pub const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
pub const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;
pub const RRF_RT_REG_SZ: u32 = 0x2;
pub const RRF_RT_REG_DWORD: u32 = 0x10;

#[link(name = "ntdll")]
extern "system" {
    pub fn NtQuerySystemInformation(class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetSystemTimes(idle: *mut FILETIME, kernel: *mut FILETIME, user: *mut FILETIME) -> i32;
    pub fn GlobalMemoryStatusEx(m: *mut MEMORYSTATUSEX) -> i32;
    pub fn K32GetPerformanceInfo(p: *mut PERFORMANCE_INFORMATION, cb: u32) -> i32;
    pub fn GetTickCount64() -> u64;
    pub fn GetLogicalProcessorInformationEx(rel: u32, buf: *mut c_void, len: *mut u32) -> i32;
    pub fn GetCurrentProcess() -> isize;
    pub fn CloseHandle(h: isize) -> i32;
    pub fn SetProcessWorkingSetSize(p: isize, min: usize, max: usize) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    pub fn RegGetValueW(hkey: isize, sub: *const u16, val: *const u16, flags: u32, ty: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
    pub fn OpenProcessToken(p: isize, access: u32, tok: *mut isize) -> i32;
    pub fn LookupPrivilegeValueW(sys: *const u16, name: *const u16, luid: *mut LUID) -> i32;
    pub fn AdjustTokenPrivileges(tok: isize, disable: i32, new: *const TOKEN_PRIVILEGES, len: u32, prev: *mut c_void, ret: *mut u32) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    pub fn IsUserAnAdmin() -> i32;
}

#[link(name = "user32")]
extern "system" {
    pub fn FindWindowW(class: *const u16, title: *const u16) -> isize;
}

#[link(name = "dwmapi")]
extern "system" {
    pub fn DwmSetWindowAttribute(h: isize, attr: u32, v: *const c_void, size: u32) -> i32;
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Turns on a privilege for this process (works only when it is in the token, i.e. as admin)
pub fn enable_privilege(name: &str) -> bool {
    unsafe {
        let mut tok = 0isize;
        // TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY
        if OpenProcessToken(GetCurrentProcess(), 0x20 | 0x8, &mut tok) == 0 {
            return false;
        }
        let mut tp = TOKEN_PRIVILEGES { PrivilegeCount: 1, Attributes: 2, ..Default::default() };
        let n = wide(name);
        let ok = LookupPrivilegeValueW(std::ptr::null(), n.as_ptr(), &mut tp.Luid) != 0
            && AdjustTokenPrivileges(tok, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut()) != 0;
        CloseHandle(tok);
        ok
    }
}

pub fn reg_string(sub: &str, val: &str) -> Option<String> {
    let (s, v) = (wide(sub), wide(val));
    let mut buf = [0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, s.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as _, &mut len) };
    if r != 0 {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]).trim().to_string())
}

pub fn reg_dword(sub: &str, val: &str) -> Option<u32> {
    let (s, v) = (wide(sub), wide(val));
    let mut out = 0u32;
    let mut len = 4u32;
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, s.as_ptr(), v.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut out as *mut u32 as _, &mut len) };
    (r == 0).then_some(out)
}

/// Number of physical cores (RelationProcessorCore records)
pub fn physical_cores() -> usize {
    unsafe {
        let mut len = 0u32;
        GetLogicalProcessorInformationEx(0, std::ptr::null_mut(), &mut len);
        if len == 0 {
            return 0;
        }
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        if GetLogicalProcessorInformationEx(0, buf.as_mut_ptr() as _, &mut len) == 0 {
            return 0;
        }
        let base = buf.as_ptr() as *const u8;
        let (mut off, mut n) = (0usize, 0usize);
        while off + 8 <= len as usize {
            let size = std::ptr::read_unaligned(base.add(off + 4) as *const u32) as usize;
            if size == 0 {
                break;
            }
            n += 1;
            off += size;
        }
        n
    }
}
