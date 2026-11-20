//! Data collection on a background thread. Every tick reads the whole system with a handful
//! of calls (one NtQuerySystemInformation for all processes) and publishes an immutable
//! `Snapshot` the UI picks up.

pub mod gpu;
pub mod win;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
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
    /// Share of the whole machine, 0..100
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
}

pub struct Shared {
    latest: Mutex<Option<Arc<Snapshot>>>,
    pub interval_ms: AtomicU32,
}
impl Shared {
    /// The newest snapshot, if one arrived since the last call
    pub fn take(&self) -> Option<Arc<Snapshot>> {
        self.latest.lock().ok()?.take()
    }
}

pub fn start(ctx: eframe::egui::Context) -> Arc<Shared> {
    let shared = Arc::new(Shared { latest: Mutex::new(None), interval_ms: AtomicU32::new(1000) });
    let s = shared.clone();
    std::thread::Builder::new()
        .name("purgeon-sampler".into())
        .spawn(move || {
            let mut c = Collector::new();
            // Prime the deltas, then a quick first real sample so the window fills fast
            c.sample(0.25);
            std::thread::sleep(Duration::from_millis(250));
            loop {
                let t0 = Instant::now();
                let iv = Duration::from_millis(s.interval_ms.load(Ordering::Relaxed) as u64);
                let snap = c.sample(iv.as_secs_f32());
                if let Ok(mut l) = s.latest.lock() {
                    *l = Some(Arc::new(snap));
                }
                ctx.request_repaint();
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
    /// (pid, create time) -> (cpu time, bytes read, bytes written)
    prev_procs: HashMap<(u32, i64), (u64, u64, u64)>,
    h_cpu: VecDeque<f32>,
    h_kernel: VecDeque<f32>,
    h_mem: VecDeque<f32>,
    h_commit: VecDeque<f32>,
    h_cores: Vec<VecDeque<f32>>,
    gpu: Option<gpu::GpuSampler>,
    /// luid -> (usage, vram) history
    h_gpu: HashMap<u64, (VecDeque<f32>, VecDeque<f32>)>,
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
        Collector {
            seq: 0,
            info: Arc::new(info),
            buf: vec![0u64; 512 * 1024 / 8],
            prev_at: None,
            prev_sys: None,
            prev_cores: Vec::new(),
            prev_procs: HashMap::new(),
            h_cpu: VecDeque::with_capacity(HIST),
            h_kernel: VecDeque::with_capacity(HIST),
            h_mem: VecDeque::with_capacity(HIST),
            h_commit: VecDeque::with_capacity(HIST),
            h_cores: vec![VecDeque::with_capacity(HIST); n],
            gpu: gpu::GpuSampler::new(),
            h_gpu: HashMap::new(),
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
            let (cpu, io_r, io_w) = match self.prev_procs.get(&key) {
                Some(&(pc, pr, pw)) if dt > 0.0 => (
                    (cpu_t.saturating_sub(pc) as f64 / (dt * 1e7 * ncpu) * 100.0) as f32,
                    rd.saturating_sub(pr) as f64 / dt,
                    wr.saturating_sub(pw) as f64 / dt,
                ),
                _ => (0.0, 0.0, 0.0),
            };
            next.insert(key, (cpu_t, rd, wr));
            if pid != 0 {
                out.push(Proc {
                    pid,
                    ppid: p.InheritedFromUniqueProcessId as u32,
                    name,
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
        self.prev_procs = next;
        out
    }
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
