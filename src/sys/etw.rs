//! Per-process physical disk I/O from the kernel's DiskIo trace events (needs administrator).
//!
//! This is the source Resource Monitor uses: every read and write that actually reaches a disk,
//! with its size and how long the disk took to serve it, tagged with the thread that issued it.
//! Unlike the process I/O counters it leaves out network traffic and reads served from the cache.
//!
//! The same session also carries the kernel's TcpIp / UdpIp send and receive events, which
//! give per-process network speed (also Resource Monitor's source). One session, two uses.
//!
//! It runs a private "system logger" session (Windows 8+), so it never takes over the shared
//! "NT Kernel Logger" another tool may be using. The session is stopped when Purgeon exits.

use super::win::wide;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Guid {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

/// Kernel DiskIo event class
const DISK_IO: Guid = Guid { d1: 0x3d6f_a8d4, d2: 0xfe05, d3: 0x11d0, d4: [0x9d, 0xda, 0x00, 0xc0, 0x4f, 0xd7, 0xba, 0x7c] };
/// Kernel TcpIp and UdpIp event classes
const TCP_IP: Guid = Guid { d1: 0x9a28_0ac0, d2: 0xc8e0, d3: 0x11d1, d4: [0x84, 0xe2, 0x00, 0xc0, 0x4f, 0xb9, 0x98, 0xa2] };
const UDP_IP: Guid = Guid { d1: 0xbf3a_50c5, d2: 0xa9c9, d3: 0x4988, d4: [0xa0, 0x05, 0x2d, 0xf0, 0xb7, 0xc8, 0x0f, 0x80] };
/// Kernel trace control GUID (only for the shared NT Kernel Logger fallback)
const SYSTEM_TRACE_CONTROL: Guid = Guid { d1: 0x9e81_4aad, d2: 0x3204, d3: 0x11d2, d4: [0x9a, 0x82, 0x00, 0x60, 0x08, 0xa8, 0x69, 0x39] };
/// Our own session's GUID
const SESSION_GUID: Guid = Guid { d1: 0x5b1e_7c21, d2: 0x9a4f, d3: 0x4d0b, d4: [0x8e, 0x61, 0x2f, 0x3a, 0x70, 0xd4, 0x91, 0xc5] };

const SESSION_NAME: &str = "Purgeon Disk Trace";
const KERNEL_LOGGER: &str = "NT Kernel Logger";

const WNODE_FLAG_TRACED_GUID: u32 = 0x0002_0000;
const EVENT_TRACE_REAL_TIME_MODE: u32 = 0x100;
const EVENT_TRACE_SYSTEM_LOGGER_MODE: u32 = 0x0200_0000;
const EVENT_TRACE_FLAG_DISK_IO: u32 = 0x100;
const EVENT_TRACE_FLAG_NETWORK_TCPIP: u32 = 0x1_0000;
const EVENT_TRACE_CONTROL_STOP: u32 = 1;
const PROCESS_TRACE_MODE_REAL_TIME: u32 = 0x100;
const PROCESS_TRACE_MODE_EVENT_RECORD: u32 = 0x1000_0000;
const ERROR_ALREADY_EXISTS: u32 = 183;
const INVALID_TRACE: u64 = u64::MAX;

/// WNODE_HEADER + EVENT_TRACE_PROPERTIES, followed by room for the session name
#[repr(C)]
struct Props {
    buffer_size: u32,
    provider_id: u32,
    historical_context: u64,
    timestamp: i64,
    guid: Guid,
    client_context: u32,
    flags: u32,
    buffer_kb: u32,
    min_buffers: u32,
    max_buffers: u32,
    max_file_size: u32,
    log_file_mode: u32,
    flush_timer: u32,
    enable_flags: u32,
    age_limit: i32,
    number_of_buffers: u32,
    free_buffers: u32,
    events_lost: u32,
    buffers_written: u32,
    log_buffers_lost: u32,
    real_time_buffers_lost: u32,
    logger_thread_id: isize,
    log_file_name_offset: u32,
    logger_name_offset: u32,
    name: [u16; 64],
}
const _: () = assert!(std::mem::offset_of!(Props, logger_thread_id) == 104 && std::mem::offset_of!(Props, name) == 120);

/// EVENT_RECORD
#[repr(C)]
struct EventRecord {
    size: u16,
    header_type: u16,
    flags: u16,
    event_property: u16,
    thread_id: u32,
    process_id: u32,
    timestamp: i64,
    provider: Guid,
    id: u16,
    version: u8,
    channel: u8,
    level: u8,
    opcode: u8,
    task: u16,
    keyword: u64,
    kernel_time: u32,
    user_time: u32,
    activity: Guid,
    buffer_context: u32,
    extended_count: u16,
    user_data_len: u16,
    extended_data: *const c_void,
    user_data: *const u8,
    user_context: *mut c_void,
}
const _: () = assert!(std::mem::offset_of!(EventRecord, opcode) == 45 && std::mem::offset_of!(EventRecord, user_data) == 96);

/// EVENT_TRACE_LOGFILEW (the two big embedded structs are opaque here)
#[repr(C)]
struct LogFile {
    log_file_name: *const u16,
    logger_name: *const u16,
    current_time: i64,
    buffers_read: u32,
    process_trace_mode: u32,
    current_event: [u8; 88],
    logfile_header: [u8; 280],
    buffer_callback: usize,
    buffer_size: u32,
    filled: u32,
    events_lost: u32,
    event_record_callback: Option<unsafe extern "system" fn(*const EventRecord)>,
    is_kernel_trace: u32,
    context: usize,
}
const _: () = assert!(std::mem::offset_of!(LogFile, event_record_callback) == 424 && std::mem::size_of::<LogFile>() == 448);

#[link(name = "advapi32")]
extern "system" {
    fn StartTraceW(handle: *mut u64, name: *const u16, props: *mut Props) -> u32;
    fn ControlTraceW(handle: u64, name: *const u16, props: *mut Props, code: u32) -> u32;
    fn OpenTraceW(log: *mut LogFile) -> u64;
    fn ProcessTrace(handles: *const u64, count: u32, start: *const c_void, end: *const c_void) -> u32;
    fn CloseTrace(handle: u64) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenThread(access: u32, inherit: i32, tid: u32) -> isize;
    fn GetProcessIdOfThread(h: isize) -> u32;
    fn CloseHandle(h: isize) -> i32;
    fn QueryPerformanceFrequency(f: *mut i64) -> i32;
}

/// Physical disk activity of one process since the last `take`
#[derive(Clone, Copy, Default)]
pub struct ProcDisk {
    pub read: u64,
    pub write: u64,
    pub ops: u32,
    /// Sum of the disk's service time for this process's requests, in QPC ticks
    resp_ticks: u64,
}
impl ProcDisk {
    /// Average time the disk took per request, milliseconds
    pub fn resp_ms(&self, qpc: f64) -> f32 {
        if self.ops == 0 { 0.0 } else { (self.resp_ticks as f64 / self.ops as f64 / qpc * 1000.0) as f32 }
    }
}

#[derive(Default)]
struct Acc {
    per: HashMap<u32, ProcDisk>,
    /// thread id -> process id (cleared on every take, thread ids get reused)
    tids: HashMap<u32, u32>,
    /// pid -> (bytes received, bytes sent) over the network (loopback left out)
    net: HashMap<u32, (u64, u64)>,
}

fn acc() -> &'static Mutex<Acc> {
    static A: OnceLock<Mutex<Acc>> = OnceLock::new();
    A.get_or_init(Default::default)
}

/// Name of the session we started, so `stop` can end it
static RUNNING: OnceLock<&'static str> = OnceLock::new();

fn props(guid: Guid, mode: u32) -> Props {
    Props {
        buffer_size: std::mem::size_of::<Props>() as u32,
        provider_id: 0,
        historical_context: 0,
        timestamp: 0,
        guid,
        client_context: 1, // QPC timestamps: the response times are in QPC ticks
        flags: WNODE_FLAG_TRACED_GUID,
        buffer_kb: 64,
        min_buffers: 4,
        max_buffers: 16,
        max_file_size: 0,
        log_file_mode: mode,
        flush_timer: 1,
        enable_flags: EVENT_TRACE_FLAG_DISK_IO | EVENT_TRACE_FLAG_NETWORK_TCPIP,
        age_limit: 0,
        number_of_buffers: 0,
        free_buffers: 0,
        events_lost: 0,
        buffers_written: 0,
        log_buffers_lost: 0,
        real_time_buffers_lost: 0,
        logger_thread_id: 0,
        log_file_name_offset: 0,
        logger_name_offset: std::mem::offset_of!(Props, name) as u32,
        name: [0; 64],
    }
}

/// Starts (or restarts, if a previous run left it behind) a session; returns the status code
fn start_session(name: &str, guid: Guid, mode: u32) -> u32 {
    let n = wide(name);
    let mut h = 0u64;
    let mut p = props(guid, mode);
    let mut r = unsafe { StartTraceW(&mut h, n.as_ptr(), &mut p) };
    if r == ERROR_ALREADY_EXISTS {
        let mut q = props(guid, mode);
        unsafe { ControlTraceW(0, n.as_ptr(), &mut q, EVENT_TRACE_CONTROL_STOP) };
        let mut p = props(guid, mode);
        r = unsafe { StartTraceW(&mut h, n.as_ptr(), &mut p) };
    }
    r
}

/// TcpIp / UdpIp send and receive: 10 send, 11 receive (IPv4), 26 / 27 the same over IPv6
unsafe fn on_net(r: &EventRecord) {
    let (recv, v6) = match r.opcode {
        10 => (false, false),
        11 => (true, false),
        26 => (false, true),
        27 => (true, true),
        _ => return,
    };
    let len = r.user_data_len as usize;
    // TcpIp/UdpIp_TypeGroup1: PID, size, daddr, saddr, ... (addresses 4 or 16 bytes)
    if r.user_data.is_null() || len < if v6 { 40 } else { 16 } {
        return;
    }
    let d = r.user_data;
    let pid = std::ptr::read_unaligned(d as *const u32);
    let size = std::ptr::read_unaligned(d.add(4) as *const u32) as u64;
    // Traffic that never leaves the machine (127.x.x.x, ::1) is not network use
    let loopback = if v6 {
        let a = std::ptr::read_unaligned(d.add(8) as *const [u8; 16]);
        a == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
    } else {
        *d.add(8) == 127
    };
    if loopback {
        return;
    }
    let Ok(mut a) = acc().lock() else { return };
    let e = a.net.entry(pid).or_default();
    if recv {
        e.0 += size;
    } else {
        e.1 += size;
    }
}

unsafe extern "system" fn on_event(r: *const EventRecord) {
    let r = &*r;
    if r.provider == TCP_IP || r.provider == UDP_IP {
        return on_net(r);
    }
    if r.provider != DISK_IO {
        return;
    }
    // 10 = read, 11 = write (completions, with the disk's service time)
    let write = match r.opcode {
        10 => false,
        11 => true,
        _ => return,
    };
    let len = r.user_data_len as usize;
    if r.user_data.is_null() || len < 48 {
        return;
    }
    // DiskIo_TypeGroup1 (64-bit): DiskNumber, IrpFlags, TransferSize, Reserved, ByteOffset,
    // FileObject, Irp, HighResResponseTime, IssuingThreadId (Windows 8+)
    let d = r.user_data;
    let size = std::ptr::read_unaligned(d.add(8) as *const u32) as u64;
    let resp = std::ptr::read_unaligned(d.add(40) as *const u64);
    let tid = if len >= 52 { std::ptr::read_unaligned(d.add(48) as *const u32) } else { 0 };
    let Ok(mut a) = acc().lock() else { return };
    let mut pid = u32::MAX;
    if tid != 0 {
        pid = match a.tids.get(&tid) {
            Some(&p) => p,
            None => {
                let h = OpenThread(0x0800, 0, tid); // THREAD_QUERY_LIMITED_INFORMATION
                let p = if h != 0 {
                    let p = GetProcessIdOfThread(h);
                    CloseHandle(h);
                    if p == 0 { u32::MAX } else { p }
                } else {
                    u32::MAX
                };
                a.tids.insert(tid, p);
                p
            }
        };
    }
    if pid == u32::MAX && r.process_id != u32::MAX {
        pid = r.process_id;
    }
    if pid == u32::MAX {
        pid = 0; // issued by a thread that is already gone: counted as unattributed
    }
    let e = a.per.entry(pid).or_default();
    if write {
        e.write += size;
    } else {
        e.read += size;
    }
    e.ops += 1;
    e.resp_ticks += resp;
}

pub struct DiskTrace {
    /// QPC ticks per second, to turn response times into milliseconds
    pub qpc: f64,
}

impl DiskTrace {
    /// Starts tracing; `None` without administrator rights or when tracing is unavailable
    pub fn start() -> Option<DiskTrace> {
        let mut name = SESSION_NAME;
        if start_session(SESSION_NAME, SESSION_GUID, EVENT_TRACE_REAL_TIME_MODE | EVENT_TRACE_SYSTEM_LOGGER_MODE) != 0 {
            // Before Windows 8 only the shared kernel logger can carry kernel events
            if start_session(KERNEL_LOGGER, SYSTEM_TRACE_CONTROL, EVENT_TRACE_REAL_TIME_MODE) != 0 {
                return None;
            }
            name = KERNEL_LOGGER;
        }
        let _ = RUNNING.set(name);
        let wname: &'static [u16] = Box::leak(wide(name).into_boxed_slice());
        let mut lf: LogFile = unsafe { std::mem::zeroed() };
        lf.logger_name = wname.as_ptr();
        lf.process_trace_mode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        lf.event_record_callback = Some(on_event);
        let th = unsafe { OpenTraceW(&mut lf) };
        if th == INVALID_TRACE {
            stop();
            return None;
        }
        std::thread::Builder::new()
            .name("purgeon-etw".into())
            .stack_size(256 * 1024)
            .spawn(move || unsafe {
                // blocks, calling on_event, until the session stops
                ProcessTrace(&th, 1, std::ptr::null(), std::ptr::null());
                CloseTrace(th);
            })
            .ok()?;
        let mut f = 0i64;
        unsafe { QueryPerformanceFrequency(&mut f) };
        Some(DiskTrace { qpc: f.max(1) as f64 })
    }

    /// Activity per process id since the previous call (pid 0 = issuer already exited)
    pub fn take(&self) -> HashMap<u32, ProcDisk> {
        let Ok(mut a) = acc().lock() else { return HashMap::new() };
        a.tids.clear();
        std::mem::take(&mut a.per)
    }

    /// Network bytes (received, sent) per process id since the previous call
    pub fn take_net(&self) -> HashMap<u32, (u64, u64)> {
        let Ok(mut a) = acc().lock() else { return HashMap::new() };
        std::mem::take(&mut a.net)
    }
}

/// Ends the trace session (call on exit: kernel sessions outlive the process otherwise)
pub fn stop() {
    if let Some(name) = RUNNING.get() {
        let n = wide(name);
        let mut p = props(if *name == KERNEL_LOGGER { SYSTEM_TRACE_CONTROL } else { SESSION_GUID }, 0);
        unsafe { ControlTraceW(0, n.as_ptr(), &mut p, EVENT_TRACE_CONTROL_STOP) };
    }
}
