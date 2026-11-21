//! Data collection on a background thread. Every tick reads the whole system with a handful
//! of calls (one NtQuerySystemInformation for all processes) and publishes an immutable
//! `Snapshot` the UI picks up.

pub mod clock;
pub mod disk;
pub mod etw;
pub mod gpu;
pub mod net;
mod pdh;
pub mod ping;
pub mod thermal;
pub mod win;
#[cfg(feature = "bench")]
pub mod bench;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Samples kept for the graphs
pub const HIST: usize = 120;

#[derive(Clone, Default)]
#[allow(dead_code)] // ppid / session feed the upcoming tree view
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    /// Executable path, when Windows lets us read it (shared between processes of the same exe)
    pub path: Option<Arc<str>>,
    /// Share of the whole machine, 0..100, from CPU cycles actually spent (not 15.6 ms timer ticks)
    pub cpu: f32,
    /// Private working set: what Task Manager calls "Memory"
    pub private_ws: u64,
    pub working_set: u64,
    /// Private committed bytes
    pub commit: u64,
    pub io_read: f64,
    pub io_write: f64,
    pub threads: u32,
    pub handles: u32,
    pub session: u32,
    /// Busiest GPU engine this process uses, 0..100
    pub gpu: f32,
    /// Dedicated GPU memory (VRAM) and shared GPU memory
    pub vram: u64,
    pub gpu_shared: u64,
    /// Physical disk reads / writes in bytes per second (administrator only)
    pub disk_read: f64,
    pub disk_write: f64,
    /// Average time the disk took to serve this process's requests, milliseconds
    pub disk_resp: f32,
    /// Network received / sent, bytes per second (administrator only)
    pub net_rx: f64,
    pub net_tx: f64,
    /// Open connections (no administrator needed)
    pub conns: net::Conns,
}

impl Proc {
    /// RAM this process shares with others (DLLs, mapped files, shared memory): the part of its
    /// working set that is not private
    pub fn shared(&self) -> u64 {
        self.working_set.saturating_sub(self.private_ws)
    }
}

#[derive(Clone, Copy, Default)]
pub struct MemLists {
    pub free: u64,
    pub zero: u64,
    pub modified: u64,
    pub standby: u64,
}

#[derive(Clone, Default)]
pub struct MemInfo {
    pub total: u64,
    pub avail: u64,
    pub commit: u64,
    pub commit_limit: u64,
    pub commit_peak: u64,
    pub cache: u64,
    pub paged: u64,
    pub nonpaged: u64,
    /// Standby / modified / free lists (needs admin)
    pub lists: Option<MemLists>,
}
impl MemInfo {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.avail)
    }
    pub fn used_frac(&self) -> f32 {
        if self.total == 0 { 0.0 } else { self.used() as f32 / self.total as f32 }
    }
}

pub struct StaticInfo {
    pub cpu_name: String,
    pub logical: usize,
    pub physical: usize,
    pub base_mhz: u32,
    pub admin: bool,
}

pub struct Snapshot {
    pub seq: u64,
    pub at: Instant,
    pub interval: f32,
    pub info: Arc<StaticInfo>,
    pub cpu: f32,
    pub kernel: f32,
    pub cores: Vec<f32>,
    pub mem: MemInfo,
    pub procs: Vec<Proc>,
    pub nprocs: u32,
    pub threads: u32,
    pub handles: u32,
    pub uptime: Duration,
    pub h_cpu: Vec<f32>,
    pub h_kernel: Vec<f32>,
    pub h_mem: Vec<f32>,
    pub h_commit: Vec<f32>,
    pub h_cores: Vec<Vec<f32>>,
    pub gpus: Vec<gpu::Gpu>,
    /// Per GPU (same order as `gpus`): usage and dedicated memory use, both 0..100
    pub h_gpu: Vec<Vec<f32>>,
    pub h_vram: Vec<Vec<f32>>,
    pub disks: Vec<disk::Disk>,
    /// Per disk (same order as `disks`): active time 0..100, read and write bytes/s
    pub h_disk: Vec<[Vec<f32>; 3]>,
    pub clocks: clock::Clocks,
    /// Per-process physical disk figures are live (running as administrator)
    pub disk_trace: bool,
    /// Disk bytes/s whose issuing thread had already exited (read, write)
    pub disk_orphan: (f64, f64),
    /// Per-process GPU counters could be opened. When false, GPU and VRAM per process are
    /// unknown (shown as "—"), not zero.
    pub gpu_proc: bool,
    /// Connected network adapters, busiest first
    pub nets: Vec<net::Adapter>,
    /// Per adapter (same order as `nets`): received and sent bytes/s
    pub h_net: Vec<[Vec<f32>; 2]>,
    /// Per-process network speed is live (running as administrator)
    pub net_trace: bool,
    /// Per GPU (same order as `gpus`): temperature °C and power W (0 when unknown)
    pub h_sens: Vec<[Vec<f32>; 2]>,
    /// Average CPU clock, MHz
    pub h_mhz: Vec<f32>,
    /// ACPI thermal zones: (name, °C). Empty on most desktops.
    pub zones: Vec<(String, f32)>,
}

pub struct Shared {
    latest: Mutex<Option<Arc<Snapshot>>>,
    /// 0 = paused
    pub interval_ms: AtomicU32,
    /// False while the window is minimized: samples keep coming (graphs stay continuous)
    /// but the UI is not woken up to draw them
    pub visible: AtomicBool,
    /// The window was closed to the tray
    pub hidden: AtomicBool,
}
impl Shared {
    /// The newest snapshot, if one arrived since the last call
    pub fn take(&self) -> Option<Arc<Snapshot>> {
        self.latest.lock().ok()?.take()
    }
}

/// Starts sampling on a background thread. `on_sample` sees every snapshot as it is made, even
/// while the window is hidden (alerts and the tray tooltip use it).
pub fn start(ctx: eframe::egui::Context, mut on_sample: impl FnMut(&Snapshot) + Send + 'static) -> Arc<Shared> {
    let shared = Arc::new(Shared {
        latest: Mutex::new(None),
        interval_ms: AtomicU32::new(1000),
        visible: AtomicBool::new(true),
        hidden: AtomicBool::new(false),
    });
    let s = shared.clone();
    std::thread::Builder::new()
        .name("purgeon-sampler".into())
        .spawn(move || {
            let mut c = Collector::new();
            // Prime the deltas, then a quick first real sample so the window fills fast
            c.sample(0.25);
            std::thread::sleep(Duration::from_millis(250));
            let mut paused = false;
            loop {
                let t0 = Instant::now();
                let ms = s.interval_ms.load(Ordering::Relaxed);
                if ms == 0 {
                    // Paused: no sampling at all
                    paused = true;
                    std::thread::sleep(Duration::from_millis(150));
                    continue;
                }
                let iv = Duration::from_millis(ms as u64);
                if paused {
                    // Resuming: take a fresh baseline so the first rates aren't averaged over the pause
                    paused = false;
                    c.sample(0.0);
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
                let snap = c.sample(iv.as_secs_f32());
                on_sample(&snap);
                if let Ok(mut l) = s.latest.lock() {
                    *l = Some(Arc::new(snap));
                }
                // The UI draws only when there is new data (or the user moves something)
                if s.visible.load(Ordering::Relaxed) || crate::ui::overlay::is_open() {
                    ctx.request_repaint();
                }
                let el = t0.elapsed();
                if el < iv {
                    std::thread::sleep(iv - el);
                }
            }
        })
        .expect("spawn sampler thread");
    shared
}

fn push(h: &mut VecDeque<f32>, v: f32) {
    if h.len() == HIST {
        h.pop_front();
    }
    h.push_back(v);
}

struct Collector {
    seq: u64,
    info: Arc<StaticInfo>,
    buf: Vec<u64>,
    prev_at: Option<Instant>,
    prev_sys: Option<(u64, u64, u64)>,
    prev_cores: Vec<win::SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION>,
    /// (pid, create time) -> (cpu time, bytes read, bytes written, cpu cycles)
    prev_procs: HashMap<(u32, i64), (u64, u64, u64, u64)>,
    prev_idle_cycles: u64,
    clock: Option<clock::ClockSampler>,
    trace: Option<etw::DiskTrace>,
    /// (pid, create time) -> executable path, looked up once per process
    paths: HashMap<(u32, i64), Option<Arc<str>>>,
    interned: HashMap<String, Arc<str>>,
    h_cpu: VecDeque<f32>,
    h_kernel: VecDeque<f32>,
    h_mem: VecDeque<f32>,
    h_commit: VecDeque<f32>,
    h_cores: Vec<VecDeque<f32>>,
    gpu: Option<gpu::GpuSampler>,
    disk: Option<disk::DiskSampler>,
    h_disk: HashMap<String, [VecDeque<f32>; 3]>,
    /// luid -> (usage, vram) history
    h_gpu: HashMap<u64, (VecDeque<f32>, VecDeque<f32>)>,
    net: net::NetSampler,
    /// luid -> (received, sent) history
    h_net: HashMap<u64, [VecDeque<f32>; 2]>,
    h_sens: HashMap<u64, [VecDeque<f32>; 2]>,
    h_mhz: VecDeque<f32>,
    thermal: Option<thermal::ThermalSampler>,
}

impl Collector {
    fn new() -> Self {
        let admin = unsafe { win::IsUserAnAdmin() } != 0;
        if admin {
            // lets us read the standby / modified memory lists
            win::enable_privilege("SeProfileSingleProcessPrivilege");
        }
        let key = "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0";
        let cores = read_cores();
        let info = StaticInfo {
            cpu_name: win::reg_string(key, "ProcessorNameString").unwrap_or_else(|| "Unknown CPU".into()),
            logical: cores.len().max(1),
            physical: win::physical_cores(),
            base_mhz: win::reg_dword(key, "~MHz").unwrap_or(0),
            admin,
        };
        let n = info.logical;
        let clock = clock::ClockSampler::new(info.base_mhz);
        let trace = if admin { etw::DiskTrace::start() } else { None };
        Collector {
            seq: 0,
            info: Arc::new(info),
            buf: vec![0u64; 512 * 1024 / 8],
            prev_at: None,
            prev_sys: None,
            prev_cores: Vec::new(),
            prev_procs: HashMap::new(),
            prev_idle_cycles: 0,
            clock,
            trace,
            paths: HashMap::new(),
            interned: HashMap::new(),
            h_cpu: VecDeque::with_capacity(HIST),
            h_kernel: VecDeque::with_capacity(HIST),
            h_mem: VecDeque::with_capacity(HIST),
            h_commit: VecDeque::with_capacity(HIST),
            h_cores: vec![VecDeque::with_capacity(HIST); n],
            gpu: gpu::GpuSampler::new(),
            disk: disk::DiskSampler::new(),
            h_disk: HashMap::new(),
            h_gpu: HashMap::new(),
            net: net::NetSampler::default(),
            h_net: HashMap::new(),
            h_sens: HashMap::new(),
            h_mhz: VecDeque::with_capacity(HIST),
            thermal: thermal::ThermalSampler::new(),
        }
    }

    fn sample(&mut self, interval: f32) -> Snapshot {
        let now = Instant::now();
        let dt = self.prev_at.map(|p| (now - p).as_secs_f64()).unwrap_or(0.0);
        self.prev_at = Some(now);

        // Whole-machine CPU (kernel time includes idle time)
        let (mut idle, mut kern, mut user) = Default::default();
        unsafe { win::GetSystemTimes(&mut idle, &mut kern, &mut user) };
        let cur = (idle.v(), kern.v(), user.v());
        let (cpu, kernel) = match self.prev_sys {
            Some((pi, pk, pu)) => {
                let (di, dk, du) = (cur.0.saturating_sub(pi), cur.1.saturating_sub(pk), cur.2.saturating_sub(pu));
                let tot = (dk + du).max(1) as f32;
                ((dk + du).saturating_sub(di) as f32 / tot * 100.0, dk.saturating_sub(di) as f32 / tot * 100.0)
            }
            None => (0.0, 0.0),
        };
        self.prev_sys = Some(cur);

        // Per logical processor
        let cores_now = read_cores();
        let cores: Vec<f32> = cores_now
            .iter()
            .enumerate()
            .map(|(i, c)| match self.prev_cores.get(i) {
                Some(p) => {
                    let di = (c.IdleTime - p.IdleTime).max(0) as f32;
                    let tot = ((c.KernelTime - p.KernelTime) + (c.UserTime - p.UserTime)).max(1) as f32;
                    ((tot - di) / tot * 100.0).clamp(0.0, 100.0)
                }
                None => 0.0,
            })
            .collect();
        self.prev_cores = cores_now;

        // Memory
        let mut ms = win::MEMORYSTATUSEX { dwLength: std::mem::size_of::<win::MEMORYSTATUSEX>() as u32, ..Default::default() };
        let mut pi = win::PERFORMANCE_INFORMATION::default();
        unsafe {
            win::GlobalMemoryStatusEx(&mut ms);
            win::K32GetPerformanceInfo(&mut pi, std::mem::size_of::<win::PERFORMANCE_INFORMATION>() as u32);
        }
        let pg = pi.PageSize.max(4096) as u64;
        let mem = MemInfo {
            total: ms.ullTotalPhys,
            avail: ms.ullAvailPhys,
            commit: pi.CommitTotal as u64 * pg,
            commit_limit: pi.CommitLimit as u64 * pg,
            commit_peak: pi.CommitPeak as u64 * pg,
            cache: pi.SystemCache as u64 * pg,
            paged: pi.KernelPaged as u64 * pg,
            nonpaged: pi.KernelNonpaged as u64 * pg,
            lists: if self.info.admin { read_mem_lists(pg) } else { None },
        };

        let mut procs = self.read_procs(dt);
        let (gpus, pg) = self.gpu.as_mut().map(|g| g.sample()).unwrap_or_default();
        if !pg.is_empty() {
            for p in procs.iter_mut() {
                if let Some(g) = pg.get(&p.pid) {
                    p.gpu = g.util;
                    p.vram = g.dedicated;
                    p.gpu_shared = g.shared;
                }
            }
        }
        let mut disk_orphan = (0.0, 0.0);
        if let Some(t) = &self.trace {
            let per = t.take();
            if dt > 0.0 {
                for p in procs.iter_mut() {
                    if let Some(d) = per.get(&p.pid) {
                        p.disk_read = d.read as f64 / dt;
                        p.disk_write = d.write as f64 / dt;
                        p.disk_resp = d.resp_ms(t.qpc);
                    }
                }
                if let Some(d) = per.get(&0) {
                    disk_orphan = (d.read as f64 / dt, d.write as f64 / dt);
                }
            }
        }
        if let Some(t) = &self.trace {
            let per = t.take_net();
            if dt > 0.0 {
                for p in procs.iter_mut() {
                    if let Some(&(rx, tx)) = per.get(&p.pid) {
                        p.net_rx = rx as f64 / dt;
                        p.net_tx = tx as f64 / dt;
                    }
                }
            }
        }
        let conns = self.net.connections();
        for p in procs.iter_mut() {
            if let Some(c) = conns.get(&p.pid) {
                p.conns = *c;
            }
        }
        let nets = self.net.adapters(dt);
        let mut h_net = Vec::with_capacity(nets.len());
        for a in &nets {
            let h = self.h_net.entry(a.luid).or_insert_with(|| std::array::from_fn(|_| VecDeque::with_capacity(HIST)));
            push(&mut h[0], a.rx as f32);
            push(&mut h[1], a.tx as f32);
            h_net.push(std::array::from_fn(|i| h[i].iter().copied().collect()));
        }
        self.h_net.retain(|k, _| nets.iter().any(|a| a.luid == *k));
        let clocks = self.clock.as_mut().map(|c| c.sample()).unwrap_or_default();
        let disks = self.disk.as_mut().map(|d| d.sample()).unwrap_or_default();
        let mut h_disk = Vec::with_capacity(disks.len());
        for d in &disks {
            let h = self.h_disk.entry(d.id.clone()).or_insert_with(|| std::array::from_fn(|_| VecDeque::with_capacity(HIST)));
            push(&mut h[0], d.active);
            push(&mut h[1], d.read as f32);
            push(&mut h[2], d.write as f32);
            h_disk.push(std::array::from_fn(|i| h[i].iter().copied().collect()));
        }
        let mut h_sens = Vec::with_capacity(gpus.len());
        for g in &gpus {
            let h = self.h_sens.entry(g.luid).or_insert_with(|| std::array::from_fn(|_| VecDeque::with_capacity(HIST)));
            push(&mut h[0], g.sensors.temp.unwrap_or(0.0));
            push(&mut h[1], g.sensors.power_w.unwrap_or(0.0));
            h_sens.push(std::array::from_fn(|i| h[i].iter().copied().collect()));
        }
        push(&mut self.h_mhz, clocks.avg_mhz);
        let zones = self.thermal.as_mut().map(|t| t.sample()).unwrap_or_default();
        let (mut h_gpu, mut h_vram) = (Vec::new(), Vec::new());
        for g in &gpus {
            let h = self.h_gpu.entry(g.luid).or_insert_with(|| (VecDeque::with_capacity(HIST), VecDeque::with_capacity(HIST)));
            push(&mut h.0, g.util);
            push(&mut h.1, if g.dedicated_total > 0 { g.dedicated_used as f32 / g.dedicated_total as f32 * 100.0 } else { 0.0 });
            h_gpu.push(h.0.iter().copied().collect());
            h_vram.push(h.1.iter().copied().collect());
        }

        push(&mut self.h_cpu, cpu);
        push(&mut self.h_kernel, kernel);
        push(&mut self.h_mem, mem.used_frac() * 100.0);
        push(&mut self.h_commit, if mem.commit_limit > 0 { mem.commit as f32 / mem.commit_limit as f32 * 100.0 } else { 0.0 });
        if self.h_cores.len() < cores.len() {
            self.h_cores.resize(cores.len(), VecDeque::with_capacity(HIST));
        }
        for (h, v) in self.h_cores.iter_mut().zip(&cores) {
            push(h, *v);
        }

        self.seq += 1;
        Snapshot {
            seq: self.seq,
            at: now,
            interval,
            info: self.info.clone(),
            cpu,
            kernel,
            cores,
            mem,
            procs,
            nprocs: pi.ProcessCount,
            threads: pi.ThreadCount,
            handles: pi.HandleCount,
            uptime: Duration::from_millis(unsafe { win::GetTickCount64() }),
            h_cpu: self.h_cpu.iter().copied().collect(),
            h_kernel: self.h_kernel.iter().copied().collect(),
            h_mem: self.h_mem.iter().copied().collect(),
            h_commit: self.h_commit.iter().copied().collect(),
            h_cores: self.h_cores.iter().map(|h| h.iter().copied().collect()).collect(),
            gpus,
            h_gpu,
            h_vram,
            disks,
            h_disk,
            clocks,
            disk_trace: self.trace.is_some(),
            disk_orphan,
            gpu_proc: self.gpu.is_some(),
            nets,
            h_net,
            net_trace: self.trace.is_some(),
            h_sens,
            h_mhz: self.h_mhz.iter().copied().collect(),
            zones,
        }
    }

    fn query_procs(&mut self) -> bool {
        for _ in 0..8 {
            let mut ret = 0u32;
            let len = (self.buf.len() * 8) as u32;
            let st = unsafe { win::NtQuerySystemInformation(win::SYSTEM_PROCESS_INFORMATION_CLASS, self.buf.as_mut_ptr() as _, len, &mut ret) };
            if st == win::STATUS_INFO_LENGTH_MISMATCH {
                let need = (ret as usize + 128 * 1024).max(self.buf.len() * 16) / 8;
                self.buf.resize(need, 0);
                continue;
            }
            return st >= 0;
        }
        false
    }

    fn read_procs(&mut self, dt: f64) -> Vec<Proc> {
        let mut out = Vec::with_capacity(self.prev_procs.len() + 16);
        if !self.query_procs() {
            return out;
        }
        let ncpu = self.info.logical.max(1) as f64;
        let mut next = HashMap::with_capacity(self.prev_procs.len() + 16);
        let mut cycles: Vec<u64> = Vec::with_capacity(self.prev_procs.len() + 16);
        let mut busy_cycles = 0u64;
        let base = self.buf.as_ptr() as *const u8;
        let cap = self.buf.len() * 8;
        let mut off = 0usize;
        loop {
            if off + std::mem::size_of::<win::SYSTEM_PROCESS_INFORMATION>() > cap {
                break;
            }
            let p = unsafe { std::ptr::read_unaligned(base.add(off) as *const win::SYSTEM_PROCESS_INFORMATION) };
            let pid = p.UniqueProcessId as u32;
            let name = if p.ImageName.Buffer.is_null() || p.ImageName.Length == 0 {
                if pid == 0 { "System Idle Process".to_string() } else { "System".to_string() }
            } else {
                let s = unsafe { std::slice::from_raw_parts(p.ImageName.Buffer, p.ImageName.Length as usize / 2) };
                String::from_utf16_lossy(s)
            };
            let cpu_t = (p.UserTime + p.KernelTime).max(0) as u64;
            let (rd, wr) = (p.ReadTransferCount.max(0) as u64, p.WriteTransferCount.max(0) as u64);
            let key = (pid, p.CreateTime);
            let (cpu, io_r, io_w, dcyc) = match self.prev_procs.get(&key) {
                Some(&(pc, pr, pw, pcy)) if dt > 0.0 => (
                    (cpu_t.saturating_sub(pc) as f64 / (dt * 1e7 * ncpu) * 100.0) as f32,
                    rd.saturating_sub(pr) as f64 / dt,
                    wr.saturating_sub(pw) as f64 / dt,
                    p.CycleTime.saturating_sub(pcy),
                ),
                _ => (0.0, 0.0, 0.0, 0),
            };
            next.insert(key, (cpu_t, rd, wr, p.CycleTime));
            if pid != 0 {
                cycles.push(dcyc);
                busy_cycles += dcyc;
            }
            let path = match self.paths.get(&key) {
                Some(p) => p.clone(),
                None => {
                    let p = if pid > 4 {
                        win::process_path(pid).map(|s| self.interned.entry(s.clone()).or_insert_with(|| Arc::from(s.as_str())).clone())
                    } else {
                        None
                    };
                    self.paths.insert(key, p.clone());
                    p
                }
            };
            if pid != 0 {
                out.push(Proc {
                    pid,
                    ppid: p.InheritedFromUniqueProcessId as u32,
                    name,
                    path,
                    cpu: cpu.clamp(0.0, 100.0),
                    private_ws: p.WorkingSetPrivateSize.max(0) as u64,
                    working_set: p.WorkingSetSize as u64,
                    commit: p.PrivatePageCount as u64,
                    io_read: io_r,
                    io_write: io_w,
                    threads: p.NumberOfThreads,
                    handles: p.HandleCount,
                    session: p.SessionId,
                    ..Default::default()
                });
            }
            if p.NextEntryOffset == 0 {
                break;
            }
            off += p.NextEntryOffset as usize;
        }
        self.paths.retain(|k, _| next.contains_key(k));
        self.prev_procs = next;
        // Precise CPU: each process's share of all cycles the processors ran (busy + idle),
        // the way Process Explorer does it. Timer-based times are only charged in whole
        // 15.6 ms ticks, so short bursts get missed or billed to the wrong process.
        //
        //   cpu%(p) = 100 * dCycleTime(p) / (sum of dCycleTime over processes + sum of d idle cycles)
        //
        // Boosted cores: Windows reads the invariant TSC for CycleTime, which ticks at a fixed
        // rate whatever the core clock, so a cycle is a unit of time and turbo does not inflate
        // anyone's share. Parked cores: they run their idle thread, whose cycles keep counting,
        // so they show up as idle capacity, as in perfmon's % Processor Time.
        // If idle cycles are unavailable, the timer-based values computed above are kept.
        // Details and caveats: docs/ACCURACY.md.
        let idle = idle_cycles();
        let didle = idle.saturating_sub(self.prev_idle_cycles);
        let first = self.prev_idle_cycles == 0;
        self.prev_idle_cycles = idle;
        let total = (busy_cycles + didle) as f64;
        if !first && idle > 0 && total > 0.0 && cycles.len() == out.len() {
            for (p, c) in out.iter_mut().zip(&cycles) {
                p.cpu = (*c as f64 / total * 100.0).clamp(0.0, 100.0) as f32;
            }
        }
        out
    }
}

/// Cycles all processors spent idle since boot
fn idle_cycles() -> u64 {
    let mut v = [0u64; 512];
    let mut ret = 0u32;
    let st = unsafe { win::NtQuerySystemInformation(win::SYSTEM_PROCESSOR_IDLE_CYCLE_CLASS, v.as_mut_ptr() as _, std::mem::size_of_val(&v) as u32, &mut ret) };
    if st < 0 {
        return 0;
    }
    v[..(ret as usize / 8).min(v.len())].iter().sum()
}

fn read_cores() -> Vec<win::SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION> {
    let mut v = vec![win::SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION::default(); 256];
    let mut ret = 0u32;
    let sz = std::mem::size_of::<win::SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION>();
    let st = unsafe { win::NtQuerySystemInformation(win::SYSTEM_PROCESSOR_PERFORMANCE_CLASS, v.as_mut_ptr() as _, (v.len() * sz) as u32, &mut ret) };
    if st < 0 {
        return Vec::new();
    }
    v.truncate(ret as usize / sz);
    v
}

fn read_mem_lists(page: u64) -> Option<MemLists> {
    let mut m = win::SYSTEM_MEMORY_LIST_INFORMATION::default();
    let mut ret = 0u32;
    let st = unsafe { win::NtQuerySystemInformation(win::SYSTEM_MEMORY_LIST_CLASS, &mut m as *mut _ as _, std::mem::size_of_val(&m) as u32, &mut ret) };
    (st >= 0).then(|| MemLists {
        free: m.FreePageCount as u64 * page,
        zero: m.ZeroPageCount as u64 * page,
        modified: m.ModifiedPageCount as u64 * page,
        standby: m.PageCountByPriority.iter().sum::<usize>() as u64 * page,
    })
}

#[cfg(test)]
mod compare {
    use super::*;

    /// Side-by-side with the counters Task Manager and perfmon read. `cargo test --release compare -- --nocapture`
    #[test]
    fn compare() {
        let mut q = pdh::Query::new().unwrap();
        let time = q.add(r"\Processor Information(_Total)\% Processor Time");
        let util = q.add(r"\Processor Information(_Total)\% Processor Utility");
        let pws = q.add(r"\Process(*)\Working Set - Private");
        let mut c = Collector::new();
        c.sample(1.0);
        q.collect();
        let mut tb: HashMap<(u32, i64), u64> = HashMap::new();
        for _ in 0..3 {
            std::thread::sleep(Duration::from_secs(2));
            let s = c.sample(2.0);
            q.collect();
            let t = q.read(time, false).first().map(|x| x.1).unwrap_or(-1.0);
            let u = q.read(util, false).first().map(|x| x.1).unwrap_or(-1.0);
            let sum: f32 = s.procs.iter().map(|p| p.cpu).sum();
            println!("\nCPU total  purgeon {:.1}%   perfmon %ProcessorTime {:.1}%   TaskMgr %Utility {:.1}% (ours {:?})   sum of per-process {:.1}%   avg clock {:.0} MHz",
                s.cpu, t, u, s.clocks.utility, sum, s.clocks.avg_mhz);
            let mut v: Vec<&Proc> = s.procs.iter().collect();
            v.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
            // timer-based figure for the same processes, from the times kept in prev_procs
            for p in v.iter().take(6) {
                let key = c.prev_procs.keys().find(|k| k.0 == p.pid).copied();
                let tnow = key.and_then(|k| c.prev_procs.get(&k)).map(|x| x.0).unwrap_or(0);
                let tprev = key.and_then(|k| tb.get(&k)).copied().unwrap_or(tnow);
                let timed = (tnow - tprev) as f64 / (2.0 * 1e7 * s.info.logical as f64) * 100.0;
                println!("  {:<28} cycles {:>6.2}%   timer ticks {:>6.2}%", p.name, p.cpu, timed);
            }
            tb = c.prev_procs.iter().map(|(k, v)| (*k, v.0)).collect();
            let pw: HashMap<String, f64> = q.read(pws, true).into_iter().collect();
            let mut m: Vec<&Proc> = s.procs.iter().collect();
            m.sort_by(|a, b| b.private_ws.cmp(&a.private_ws));
            for p in m.iter().take(3) {
                let stem = p.name.trim_end_matches(".exe");
                println!("  mem {:<24} purgeon {:>8} KB   perfmon {:>8} KB (first instance named {stem})", p.name, p.private_ws / 1024, pw.get(stem).map(|x| *x as u64 / 1024).unwrap_or(0));
            }
            for d in &s.disks {
                println!("  disk {}  read {:.0} B/s  write {:.0} B/s  active {:.1}%  resp {:.2} ms", d.id, d.read, d.write, d.active, d.resp_ms);
            }
        }
        println!("disk trace running: {} (needs administrator)", c.trace.is_some());
    }
}
