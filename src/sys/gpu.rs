//! GPU data for every vendor from the same performance counters Task Manager reads
//! (`GPU Engine`, `GPU Process Memory`, `GPU Adapter Memory`), adapter names and sizes from
//! DXGI, and temperature / clocks / power / fan from NVML on NVIDIA cards.
#![allow(non_snake_case)]

use super::win::{wide, LUID};
use std::collections::HashMap;
use std::ffi::c_void;

// ---------------------------------------------------------------- FFI

#[repr(C)]
struct PdhItem {
    name: *const u16,
    status: u32,
    value: f64, // also read as i64 for PDH_FMT_LARGE (same 8 bytes)
}

const PDH_MORE_DATA: u32 = 0x8000_07D2;
const PDH_FMT_DOUBLE: u32 = 0x200;
const PDH_FMT_LARGE: u32 = 0x400;
const PDH_FMT_NOCAP100: u32 = 0x8000;

#[link(name = "pdh")]
extern "system" {
    fn PdhOpenQueryW(src: *const u16, user: usize, q: *mut isize) -> u32;
    fn PdhAddEnglishCounterW(q: isize, path: *const u16, user: usize, c: *mut isize) -> u32;
    fn PdhCollectQueryData(q: isize) -> u32;
    fn PdhGetFormattedCounterArrayW(c: isize, fmt: u32, size: *mut u32, count: *mut u32, items: *mut PdhItem) -> u32;
}

#[repr(C)]
struct DxgiDesc1 {
    Description: [u16; 128],
    VendorId: u32,
    DeviceId: u32,
    SubSysId: u32,
    Revision: u32,
    DedicatedVideoMemory: usize,
    DedicatedSystemMemory: usize,
    SharedSystemMemory: usize,
    AdapterLuid: LUID,
    Flags: u32,
}

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const IID_IDXGI_FACTORY1: Guid = Guid(0x770aae78, 0xf26f, 0x4dba, [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87]);

#[link(name = "dxgi")]
extern "system" {
    fn CreateDXGIFactory1(riid: *const Guid, out: *mut *mut c_void) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> isize;
    fn GetProcAddress(m: isize, name: *const u8) -> *const c_void;
}

/// Calls COM method `idx` of `obj` (vtable slot) with the given signature
unsafe fn vt<F: Copy>(obj: *mut c_void, idx: usize) -> F {
    let table = *(obj as *const *const usize);
    std::mem::transmute_copy(&*table.add(idx))
}

// ---------------------------------------------------------------- public data

#[derive(Clone, Default)]
pub struct Engine {
    pub kind: String,
    pub util: f32,
}

#[derive(Clone, Default)]
pub struct Sensors {
    pub temp: Option<f32>,
    pub core_mhz: Option<u32>,
    pub mem_mhz: Option<u32>,
    pub power_w: Option<f32>,
    pub power_limit_w: Option<f32>,
    pub fan_pct: Option<u32>,
}

#[derive(Clone, Default)]
pub struct Gpu {
    pub luid: u64,
    pub name: String,
    pub vendor: u32,
    pub dedicated_total: u64,
    pub shared_total: u64,
    /// Busiest engine, 0..100 (what Task Manager shows as the GPU's usage)
    pub util: f32,
    pub engines: Vec<Engine>,
    pub dedicated_used: u64,
    pub shared_used: u64,
    pub sensors: Sensors,
}
impl Gpu {
    pub fn vendor_name(&self) -> &'static str {
        match self.vendor {
            0x10de => "NVIDIA",
            0x1002 => "AMD",
            0x8086 => "Intel",
            _ => "",
        }
    }
}

/// One process's GPU use, summed over all adapters
#[derive(Clone, Copy, Default)]
pub struct ProcGpu {
    pub util: f32,
    pub dedicated: u64,
    pub shared: u64,
}

// ---------------------------------------------------------------- sampler

struct Adapter {
    luid: u64,
    name: String,
    vendor: u32,
    dedicated: u64,
    shared: u64,
}

pub struct GpuSampler {
    query: isize,
    engine: isize,
    proc_ded: isize,
    proc_shared: isize,
    ad_ded: isize,
    ad_shared: isize,
    adapters: Vec<Adapter>,
    nvml: Option<Nvml>,
    buf: Vec<u8>,
}

impl GpuSampler {
    pub fn new() -> Option<GpuSampler> {
        let adapters = dxgi_adapters();
        if adapters.is_empty() {
            return None;
        }
        let mut q = 0isize;
        if unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut q) } != 0 {
            return None;
        }
        let add = |path: &str| {
            let mut c = 0isize;
            let w = wide(path);
            if unsafe { PdhAddEnglishCounterW(q, w.as_ptr(), 0, &mut c) } == 0 { c } else { 0 }
        };
        let s = GpuSampler {
            query: q,
            engine: add("\\GPU Engine(*)\\Utilization Percentage"),
            proc_ded: add("\\GPU Process Memory(*)\\Dedicated Usage"),
            proc_shared: add("\\GPU Process Memory(*)\\Shared Usage"),
            ad_ded: add("\\GPU Adapter Memory(*)\\Dedicated Usage"),
            ad_shared: add("\\GPU Adapter Memory(*)\\Shared Usage"),
            nvml: Nvml::load(&adapters),
            adapters,
            buf: Vec::new(),
        };
        unsafe { PdhCollectQueryData(q) }; // rate counters need a first sample
        Some(s)
    }

    /// Reads every counter once. Returns the adapters and per-process use keyed by pid.
    pub fn sample(&mut self) -> (Vec<Gpu>, HashMap<u32, ProcGpu>) {
        unsafe { PdhCollectQueryData(self.query) };
        let mut gpus: Vec<Gpu> = self
            .adapters
            .iter()
            .map(|a| Gpu {
                luid: a.luid,
                name: a.name.clone(),
                vendor: a.vendor,
                dedicated_total: a.dedicated,
                shared_total: a.shared,
                ..Default::default()
            })
            .collect();
        let mut procs: HashMap<u32, ProcGpu> = HashMap::new();

        // Engines: sum processes per physical engine, then a process's usage is its busiest engine
        let mut per_engine: HashMap<(u64, u32), (String, f32)> = HashMap::new();
        let mut per_proc: HashMap<(u32, u64, u32), f32> = HashMap::new();
        for (inst, v) in self.read(self.engine, false) {
            let Some(e) = parse(&inst) else { continue };
            let (Some(pid), Some(eng)) = (e.pid, e.eng) else { continue };
            let v = v as f32;
            let slot = per_engine.entry((e.luid, eng)).or_insert_with(|| (e.kind.to_string(), 0.0));
            slot.1 += v;
            *per_proc.entry((pid, e.luid, eng)).or_default() += v;
        }
        for ((pid, _, _), v) in per_proc {
            let p = procs.entry(pid).or_default();
            p.util = p.util.max(v.min(100.0));
        }
        for g in gpus.iter_mut() {
            let mut kinds: Vec<Engine> = Vec::new();
            for ((luid, _), (kind, v)) in &per_engine {
                if *luid != g.luid {
                    continue;
                }
                let v = v.min(100.0);
                match kinds.iter_mut().find(|k| &k.kind == kind) {
                    Some(k) => k.util = k.util.max(v),
                    None => kinds.push(Engine { kind: kind.clone(), util: v }),
                }
            }
            kinds.sort_by(|a, b| engine_rank(&a.kind).cmp(&engine_rank(&b.kind)).then(a.kind.cmp(&b.kind)));
            g.util = kinds.iter().map(|k| k.util).fold(0.0, f32::max);
            g.engines = kinds;
        }

        for (inst, v) in self.read(self.proc_ded, true) {
            if let Some(pid) = parse(&inst).and_then(|e| e.pid) {
                procs.entry(pid).or_default().dedicated += v as u64;
            }
        }
        for (inst, v) in self.read(self.proc_shared, true) {
            if let Some(pid) = parse(&inst).and_then(|e| e.pid) {
                procs.entry(pid).or_default().shared += v as u64;
            }
        }
        for (inst, v) in self.read(self.ad_ded, true) {
            if let Some(g) = parse(&inst).and_then(|e| gpus.iter_mut().find(|g| g.luid == e.luid)) {
                g.dedicated_used += v as u64;
            }
        }
        for (inst, v) in self.read(self.ad_shared, true) {
            if let Some(g) = parse(&inst).and_then(|e| gpus.iter_mut().find(|g| g.luid == e.luid)) {
                g.shared_used += v as u64;
            }
        }
        if let Some(n) = &self.nvml {
            for g in gpus.iter_mut() {
                if let Some(s) = n.read(g.luid) {
                    g.sensors = s;
                }
            }
        }
        (gpus, procs)
    }

    /// (instance name, value) for every instance of a wildcard counter
    fn read(&mut self, c: isize, large: bool) -> Vec<(String, f64)> {
        if c == 0 {
            return Vec::new();
        }
        let fmt = if large { PDH_FMT_LARGE } else { PDH_FMT_DOUBLE | PDH_FMT_NOCAP100 };
        for _ in 0..4 {
            let (mut size, mut count) = (self.buf.len() as u32, 0u32);
            let r = unsafe { PdhGetFormattedCounterArrayW(c, fmt, &mut size, &mut count, self.buf.as_mut_ptr() as *mut PdhItem) };
            if r == PDH_MORE_DATA || (r == 0 && self.buf.is_empty()) {
                self.buf.resize(size as usize + 4096, 0);
                continue;
            }
            if r != 0 {
                return Vec::new();
            }
            let items = unsafe { std::slice::from_raw_parts(self.buf.as_ptr() as *const PdhItem, count as usize) };
            return items
                .iter()
                .filter(|it| it.status <= 1 && !it.name.is_null()) // PDH_CSTATUS_VALID_DATA / NEW_DATA
                .map(|it| {
                    let name = unsafe {
                        let mut n = 0;
                        while *it.name.add(n) != 0 {
                            n += 1;
                        }
                        String::from_utf16_lossy(std::slice::from_raw_parts(it.name, n))
                    };
                    let v = if large { it.value.to_bits() as i64 as f64 } else { it.value };
                    (name, v)
                })
                .collect();
        }
        Vec::new()
    }
}

/// Order engines the way people think about them
fn engine_rank(k: &str) -> u8 {
    match k {
        "3D" => 0,
        "Copy" => 1,
        "VideoDecode" => 2,
        "VideoEncode" => 3,
        _ if k.starts_with("Compute") => 4,
        _ => 5,
    }
}

struct Inst<'a> {
    pid: Option<u32>,
    luid: u64,
    eng: Option<u32>,
    kind: &'a str,
}

/// Parses `pid_1234_luid_0x00000000_0x0000D1B4_phys_0_eng_3_engtype_VideoDecode`
/// (and the shorter process / adapter memory forms)
fn parse(s: &str) -> Option<Inst<'_>> {
    let hex = |t: &str| u32::from_str_radix(t.trim_start_matches("0x").trim_start_matches("0X"), 16).ok();
    let mut pid = None;
    let mut luid = None;
    let mut eng = None;
    let mut kind = "";
    let parts: Vec<&str> = s.split('_').collect();
    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "pid" => {
                pid = parts.get(i + 1).and_then(|t| t.parse().ok());
                i += 2;
            }
            "luid" => {
                let hi = parts.get(i + 1).and_then(|t| hex(t))?;
                let lo = parts.get(i + 2).and_then(|t| hex(t))?;
                luid = Some(((hi as u64) << 32) | lo as u64);
                i += 3;
            }
            "eng" => {
                eng = parts.get(i + 1).and_then(|t| t.parse().ok());
                i += 2;
            }
            "engtype" => {
                // The rest (it may hold underscores itself, e.g. Compute_0)
                let at = s.find("engtype_").map(|p| p + 8).unwrap_or(s.len());
                kind = &s[at..];
                break;
            }
            _ => i += 1,
        }
    }
    Some(Inst { pid, luid: luid?, eng, kind })
}

fn luid_key(l: LUID) -> u64 {
    ((l.HighPart as u32 as u64) << 32) | l.LowPart as u64
}

fn dxgi_adapters() -> Vec<Adapter> {
    let mut out = Vec::new();
    unsafe {
        let mut f: *mut c_void = std::ptr::null_mut();
        if CreateDXGIFactory1(&IID_IDXGI_FACTORY1, &mut f) < 0 || f.is_null() {
            return out;
        }
        type Enum1 = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
        type Desc1 = unsafe extern "system" fn(*mut c_void, *mut DxgiDesc1) -> i32;
        type Release = unsafe extern "system" fn(*mut c_void) -> u32;
        let mut i = 0;
        loop {
            let mut a: *mut c_void = std::ptr::null_mut();
            if vt::<Enum1>(f, 12)(f, i, &mut a) < 0 || a.is_null() {
                break;
            }
            let mut d: DxgiDesc1 = std::mem::zeroed();
            if vt::<Desc1>(a, 10)(a, &mut d) >= 0 && d.Flags & 2 == 0 {
                let luid = luid_key(d.AdapterLuid);
                if !out.iter().any(|x: &Adapter| x.luid == luid) {
                    let n = d.Description.iter().position(|&c| c == 0).unwrap_or(128);
                    out.push(Adapter {
                        luid,
                        name: String::from_utf16_lossy(&d.Description[..n]).trim().to_string(),
                        vendor: d.VendorId,
                        dedicated: d.DedicatedVideoMemory as u64,
                        shared: d.SharedSystemMemory as u64,
                    });
                }
            }
            vt::<Release>(a, 2)(a);
            i += 1;
        }
        vt::<Release>(f, 2)(f);
    }
    out
}

// ---------------------------------------------------------------- NVML (NVIDIA only)

type NvInit = unsafe extern "C" fn() -> i32;
type NvCount = unsafe extern "C" fn(*mut u32) -> i32;
type NvHandle = unsafe extern "C" fn(u32, *mut *mut c_void) -> i32;
type NvName = unsafe extern "C" fn(*mut c_void, *mut u8, u32) -> i32;
type NvU32 = unsafe extern "C" fn(*mut c_void, *mut u32) -> i32;
type NvU32Arg = unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32;

struct Nvml {
    /// (adapter luid, device handle)
    devs: Vec<(u64, *mut c_void)>,
    temp: Option<NvU32Arg>,
    clock: Option<NvU32Arg>,
    power: Option<NvU32>,
    limit: Option<NvU32>,
    fan: Option<NvU32>,
}

// The device handles are plain ids owned by the driver; we use them from one thread only
unsafe impl Send for Nvml {}

impl Nvml {
    fn load(adapters: &[Adapter]) -> Option<Nvml> {
        let nv: Vec<&Adapter> = adapters.iter().filter(|a| a.vendor == 0x10de).collect();
        if nv.is_empty() {
            return None;
        }
        unsafe {
            let mut m = LoadLibraryW(wide("nvml.dll").as_ptr());
            if m == 0 {
                m = LoadLibraryW(wide("C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvml.dll").as_ptr());
            }
            if m == 0 {
                return None;
            }
            let sym = |n: &str| {
                let c: Vec<u8> = n.bytes().chain(Some(0)).collect();
                GetProcAddress(m, c.as_ptr())
            };
            macro_rules! f {
                ($n:expr, $t:ty) => {{
                    let p = sym($n);
                    (!p.is_null()).then(|| std::mem::transmute::<*const c_void, $t>(p))
                }};
            }
            let init = f!("nvmlInit_v2", NvInit)?;
            if init() != 0 {
                return None;
            }
            let count = f!("nvmlDeviceGetCount_v2", NvCount)?;
            let handle = f!("nvmlDeviceGetHandleByIndex_v2", NvHandle)?;
            let name = f!("nvmlDeviceGetName", NvName)?;
            let mut n = 0u32;
            count(&mut n);
            let mut devs = Vec::new();
            for i in 0..n {
                let mut h = std::ptr::null_mut();
                if handle(i, &mut h) != 0 {
                    continue;
                }
                let mut buf = [0u8; 96];
                name(h, buf.as_mut_ptr(), buf.len() as u32);
                let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                let nm = String::from_utf8_lossy(&buf[..len]).to_string();
                // Match by name; with one NVIDIA card on each side just pair them
                let a = nv.iter().find(|a| a.name == nm && !devs.iter().any(|(l, _)| *l == a.luid)).or(if n == 1 && nv.len() == 1 { nv.first() } else { None });
                if let Some(a) = a {
                    devs.push((a.luid, h));
                }
            }
            Some(Nvml {
                devs,
                temp: f!("nvmlDeviceGetTemperature", NvU32Arg),
                clock: f!("nvmlDeviceGetClockInfo", NvU32Arg),
                power: f!("nvmlDeviceGetPowerUsage", NvU32),
                limit: f!("nvmlDeviceGetEnforcedPowerLimit", NvU32),
                fan: f!("nvmlDeviceGetFanSpeed", NvU32),
            })
        }
    }

    fn read(&self, luid: u64) -> Option<Sensors> {
        let h = self.devs.iter().find(|d| d.0 == luid)?.1;
        let one = |f: Option<NvU32>| {
            let mut v = 0u32;
            f.and_then(|f| (unsafe { f(h, &mut v) } == 0).then_some(v))
        };
        let arg = |f: Option<NvU32Arg>, a: u32| {
            let mut v = 0u32;
            f.and_then(|f| (unsafe { f(h, a, &mut v) } == 0).then_some(v))
        };
        Some(Sensors {
            temp: arg(self.temp, 0).map(|t| t as f32),
            core_mhz: arg(self.clock, 0),
            mem_mhz: arg(self.clock, 2),
            power_w: one(self.power).map(|mw| mw as f32 / 1000.0),
            power_limit_w: one(self.limit).map(|mw| mw as f32 / 1000.0),
            fan_pct: one(self.fan),
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dump() {
        let mut s = super::GpuSampler::new().expect("gpu sampler");
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let t = std::time::Instant::now();
        let (g, p) = s.sample();
        println!("sample took {:?}", t.elapsed());
        for g in &g {
            println!("{} {:x} vendor {:x} ded {}/{} MB shared {} MB util {:.1}", g.name, g.luid, g.vendor, g.dedicated_used >> 20, g.dedicated_total >> 20, g.shared_used >> 20, g.util);
            for e in &g.engines {
                println!("   {} {:.1}", e.kind, e.util);
            }
            println!("   sensors temp {:?} core {:?} mem {:?} power {:?}/{:?} fan {:?}", g.sensors.temp, g.sensors.core_mhz, g.sensors.mem_mhz, g.sensors.power_w, g.sensors.power_limit_w, g.sensors.fan_pct);
        }
        let mut v: Vec<_> = p.iter().collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1.dedicated));
        for (pid, x) in v.iter().take(8) {
            println!("pid {pid} util {:.1} ded {} MB shared {} MB", x.util, x.dedicated >> 20, x.shared >> 20);
        }
    }
}
