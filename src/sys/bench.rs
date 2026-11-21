//! Accuracy and overhead check, built only with `--features bench`:
//!
//! ```text
//! cargo run --release --features bench -- --bench [idle seconds]
//! ```
//!
//! Two parts, writing `bench-results.md` in the working directory:
//!
//! 1. **Idle**: samples once a second next to the perfmon counters Task Manager reads, and
//!    measures what Purgeon itself costs.
//! 2. **Known loads**: starts a copy of itself (`purgeon-load.exe` in the temp folder, below-normal
//!    priority) that does an exactly measured amount of work, then checks how much of it Purgeon
//!    and Windows' own counters saw. The loads are kept small on purpose:
//!    - steady CPU: 4 threads spinning for 15 s
//!    - bursty CPU: 4 threads that run 3 ms, sleep 10 ms, for 15 s (the case 15.6 ms timer
//!      ticks get wrong)
//!    - memory: 512 MB allocated and touched, held 8 s
//!    - disk: 256 MB written through to a temp file at 32 MB/s; the file is opened
//!      delete-on-close, so Windows removes it even if the test is killed
//!
//!
//! `--bench-mem` runs only two memory cases that private working set misses:
//!    - reserve: 512 MB committed but never touched (uses no RAM)
//!    - shared: 256 MB of shared memory (a page-file-backed section) touched; it uses RAM but
//!      counts neither as private working set nor as the process's commit
//!
//!    Every load process exits on its own after at most 60 s, and the bench kills it after 60 s
//!    too. Nothing touches the GPU, needs administrator, or loads a driver.

use super::*;
use std::fmt::Write as _;
use std::io::{Read as _, Write as _};

fn pct(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[((s.len() - 1) as f64 * q).round() as usize]
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

// ---------------------------------------------------------------------------------------------
// The load process
// ---------------------------------------------------------------------------------------------

const LOAD_EXE: &str = "purgeon-load.exe";
const LOAD_NAME: &str = "purgeon-load";
/// Hard limit on any load process's life, enforced inside it and by the bench
const LOAD_MAX: Duration = Duration::from_secs(60);

/// Spins until `until`, returning the time this thread actually ran. Gaps over 50 µs between
/// two clock reads mean the thread was paused (preempted or interrupted) and are left out.
fn spin(until: Instant) -> f64 {
    let mut ran = 0.0;
    let mut last = Instant::now();
    while last < until {
        let now = Instant::now();
        let d = (now - last).as_secs_f64();
        if d < 50e-6 {
            ran += d;
        }
        last = now;
    }
    ran
}

/// `--load <cpu|burst|mem|disk> <amount>`: does the work, prints `TRUTH <value>` and exits.
pub fn load(args: &[String]) {
    std::thread::spawn(|| {
        std::thread::sleep(LOAD_MAX);
        std::process::exit(3);
    });
    let kind = args.first().map(String::as_str).unwrap_or("");
    let amount: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    // let the bench see the process before any work, so no work lands in its first interval
    std::thread::sleep(Duration::from_secs(2));
    let truth: f64 = match kind {
        "cpu" | "burst" => {
            let end = Instant::now() + Duration::from_secs(15);
            let burst = kind == "burst";
            let hs: Vec<_> = (0..amount.clamp(1, 4))
                .map(|_| {
                    std::thread::spawn(move || {
                        let mut ran = 0.0;
                        while Instant::now() < end {
                            if burst {
                                ran += spin((Instant::now() + Duration::from_millis(3)).min(end));
                                std::thread::sleep(Duration::from_millis(10));
                            } else {
                                ran += spin(end);
                            }
                        }
                        ran
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap_or(0.0)).sum()
        }
        "mem" => {
            let n = (amount.min(1024) as usize) << 20;
            let mut v: Vec<u8> = Vec::with_capacity(n);
            // write every page so it is really in RAM, not just reserved
            v.resize(n, 1);
            for i in (0..n).step_by(4096) {
                v[i] = (i >> 12) as u8;
            }
            std::thread::sleep(Duration::from_secs(8));
            std::hint::black_box(&v);
            let n = v.len() as f64;
            drop(v);
            n
        }
        "disk" => {
            use std::os::windows::fs::OpenOptionsExt;
            const WRITE_THROUGH: u32 = 0x8000_0000;
            const DELETE_ON_CLOSE: u32 = 0x0400_0000;
            let path = std::env::temp_dir().join(format!("purgeon-bench-{}.tmp", std::process::id()));
            let mut written = 0u64;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(WRITE_THROUGH | DELETE_ON_CLOSE)
                .open(&path)
            {
                let chunk = vec![0x5Au8; 1 << 20];
                let total = amount.min(512);
                let t0 = Instant::now();
                for i in 0..total {
                    if f.write_all(&chunk).is_err() {
                        break;
                    }
                    written += chunk.len() as u64;
                    // pace to 32 MB/s
                    let due = t0 + Duration::from_secs_f64((i + 1) as f64 / 32.0);
                    if let Some(w) = due.checked_duration_since(Instant::now()) {
                        std::thread::sleep(w);
                    }
                }
                let _ = f.sync_all();
            }
            written as f64
        }
        "reserve" => {
            let n = (amount.min(1024) as usize) << 20;
            // committed by the allocator, never written, so never given RAM
            let v: Vec<u8> = Vec::with_capacity(n);
            std::hint::black_box(&v);
            std::thread::sleep(Duration::from_secs(8));
            drop(v);
            n as f64
        }
        "shared" => shared_section((amount.min(512) as usize) << 20),
        _ => 0.0,
    };
    println!("TRUTH {truth}");
    let _ = std::io::stdout().flush();
    // stay idle a moment so the last busy interval is counted before the process disappears
    std::thread::sleep(Duration::from_secs(3));
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileMappingW(file: isize, attr: *const u8, protect: u32, hi: u32, lo: u32, name: *const u16) -> isize;
    fn MapViewOfFile(map: isize, access: u32, hi: u32, lo: u32, size: usize) -> *mut u8;
    fn UnmapViewOfFile(base: *const u8) -> i32;
    fn CloseHandle(h: isize) -> i32;
}

/// Page-file-backed shared memory, every page written, held 8 s. Returns the bytes touched.
fn shared_section(n: usize) -> f64 {
    const PAGE_READWRITE: u32 = 0x04;
    const FILE_MAP_WRITE: u32 = 0x02;
    unsafe {
        let m = CreateFileMappingW(-1, std::ptr::null(), PAGE_READWRITE, (n >> 32) as u32, n as u32, std::ptr::null());
        if m == 0 {
            return 0.0;
        }
        let p = MapViewOfFile(m, FILE_MAP_WRITE, 0, 0, n);
        if p.is_null() {
            CloseHandle(m);
            return 0.0;
        }
        for i in (0..n).step_by(4096) {
            p.add(i).write_volatile(1);
        }
        std::thread::sleep(Duration::from_secs(8));
        UnmapViewOfFile(p);
        CloseHandle(m);
    }
    n as f64
}

/// Kills the load process if the bench leaves early (panic, timeout)
struct Guard(std::process::Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// One sample of the load process, as Purgeon and as perfmon see it
#[derive(Default, Clone, Copy)]
struct Seen {
    dt: f64,
    present: bool,
    ours_cpu: f64,
    ours_pws: f64,
    ours_ws: f64,
    ours_commit: f64,
    avail: f64,
    ours_io_write: f64,
    ours_drive_write: f64,
    pm_cpu: f64,
    pm_pws: f64,
    pm_io_write: f64,
    pm_drive_write: f64,
}

struct Perf {
    q: pdh::Query,
    cpu: pdh::Counter,
    pws: pdh::Counter,
    io_w: pdh::Counter,
    drive_w: pdh::Counter,
}

impl Perf {
    fn new() -> Perf {
        let q = pdh::Query::new().expect("PDH");
        let p = Perf {
            cpu: q.add(r"\Process(*)\% Processor Time"),
            pws: q.add(r"\Process(*)\Working Set - Private"),
            io_w: q.add(r"\Process(*)\IO Write Bytes/sec"),
            drive_w: q.add(r"\PhysicalDisk(_Total)\Disk Write Bytes/sec"),
            q,
        };
        p.q.collect();
        p
    }
    fn get(&mut self, c: pdh::Counter, large: bool) -> Option<f64> {
        self.q.read(c, large).into_iter().find(|(n, _)| n == LOAD_NAME).map(|x| x.1)
    }
}

/// Runs one load and samples it once a second until it exits
fn watch(c: &mut Collector, pf: &mut Perf, exe: &std::path::Path, kind: &str, amount: u64) -> Option<(f64, Vec<Seen>)> {
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x4000;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    use std::os::windows::process::CommandExt;
    let child = std::process::Command::new(exe)
        .args(["--load", kind, &amount.to_string()])
        .stdout(std::process::Stdio::piped())
        .creation_flags(BELOW_NORMAL_PRIORITY_CLASS | CREATE_NO_WINDOW)
        .spawn()
        .ok()?;
    let pid = child.id();
    let mut g = Guard(child);
    let start = Instant::now();
    let mut seen = Vec::new();
    let mut last = Instant::now();
    c.sample(1.0);
    pf.q.collect();
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let s = c.sample(1.0);
        pf.q.collect();
        let now = Instant::now();
        let mut row = Seen { dt: (now - last).as_secs_f64(), ..Default::default() };
        last = now;
        if let Some(p) = s.procs.iter().find(|p| p.pid == pid) {
            row.present = true;
            row.ours_cpu = p.cpu as f64 / 100.0 * c.info.logical as f64;
            row.ours_pws = p.private_ws as f64;
            row.ours_ws = p.working_set as f64;
            row.ours_commit = p.commit as f64;
            row.ours_io_write = p.io_write;
        }
        row.avail = s.mem.avail as f64;
        row.ours_drive_write = s.disks.iter().map(|d| d.write).sum();
        row.pm_cpu = pf.get(pf.cpu, false).unwrap_or(0.0) / 100.0;
        row.pm_pws = pf.get(pf.pws, true).unwrap_or(0.0);
        row.pm_io_write = pf.get(pf.io_w, false).unwrap_or(0.0);
        row.pm_drive_write = pf.q.read(pf.drive_w, false).first().map(|x| x.1).unwrap_or(0.0);
        seen.push(row);
        let exited = matches!(g.0.try_wait(), Ok(Some(_)));
        if exited || start.elapsed() > LOAD_MAX {
            break;
        }
    }
    let mut out = String::new();
    if let Some(mut so) = g.0.stdout.take() {
        let _ = so.read_to_string(&mut out);
    }
    let truth = out.lines().find_map(|l| l.strip_prefix("TRUTH ")?.trim().parse().ok())?;
    Some((truth, seen))
}

fn err_pct(seen: f64, truth: f64) -> f64 {
    (seen - truth) / truth.max(1e-9) * 100.0
}

// ---------------------------------------------------------------------------------------------
// The bench
// ---------------------------------------------------------------------------------------------

/// The two memory cases private working set misses. Rise from the first sample to the peak (for
/// Available, the deepest drop), in MB.
pub fn run_mem() {
    let mut c = Collector::new();
    c.sample(1.0);
    let mut pf = Perf::new();
    let exe = std::env::temp_dir().join(LOAD_EXE);
    let mut out = String::from("# Purgeon memory bench\n\n| Load | Truth (MB) | Private working set | Working set | Commit | Drop in available RAM |\n|---|---|---|---|---|---|\n");
    if std::env::current_exe().ok().and_then(|me| std::fs::copy(me, &exe).ok()).is_none() {
        out.push_str("Could not copy the load program; skipped.\n");
    }
    for (kind, mb, label) in [("reserve", 512, "512 MB committed, never touched"), ("shared", 256, "256 MB shared memory, touched")] {
        if !exe.exists() {
            break;
        }
        match watch(&mut c, &mut pf, &exe, kind, mb) {
            Some((truth, v)) => {
                let v: Vec<Seen> = v.into_iter().filter(|s| s.present).collect();
                let Some(b) = v.first().copied() else { continue };
                let rise = |f: fn(&Seen) -> f64| (v.iter().map(f).fold(f64::MIN, f64::max) - f(&b)) / 1048576.0;
                let drop = (b.avail - v.iter().map(|s| s.avail).fold(f64::MAX, f64::min)) / 1048576.0;
                let _ = writeln!(
                    out,
                    "| {label} | {:.0} | {:.1} | {:.1} | {:.1} | {:.0} |",
                    truth / 1048576.0,
                    rise(|s| s.ours_pws),
                    rise(|s| s.ours_ws),
                    rise(|s| s.ours_commit),
                    drop
                );
            }
            None => {
                let _ = writeln!(out, "| {label} | failed | | | | |");
            }
        }
    }
    let _ = std::fs::remove_file(&exe);
    let _ = std::fs::write("bench-mem.md", &out);
    println!("{out}");
}

pub fn run(secs: u32) {
    let mut q = pdh::Query::new().expect("PDH");
    let time = q.add(r"\Processor Information(_Total)\% Processor Time");
    let util = q.add(r"\Processor Information(_Total)\% Processor Utility");
    let pws = q.add(r"\Process(*)\Working Set - Private");
    let mut c = Collector::new();
    c.sample(1.0);
    q.collect();
    let me = std::process::id();

    // per sample: (sample ms, ours total, perfmon total, task manager utility, sum of per-process, own cpu %, own private ws)
    let mut rows: Vec<(f64, f64, f64, f64, f64, f64, u64)> = Vec::new();
    let mut mem_err: Vec<f64> = Vec::new();
    for _ in 0..secs.max(5) {
        std::thread::sleep(Duration::from_secs(1));
        let t0 = Instant::now();
        let s = c.sample(1.0);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        q.collect();
        let t = q.read(time, false).first().map(|x| x.1).unwrap_or(f64::NAN);
        let u = q.read(util, false).first().map(|x| x.1).unwrap_or(f64::NAN);
        let sum: f64 = s.procs.iter().map(|p| p.cpu as f64).sum();
        let own = s.procs.iter().find(|p| p.pid == me);
        rows.push((ms, s.cpu as f64, t, u, sum, own.map_or(0.0, |p| p.cpu as f64), own.map_or(0, |p| p.private_ws)));

        // Private working set of processes whose name is unique (perfmon instance names are
        // ambiguous for duplicates such as chrome#3)
        let pw: HashMap<String, f64> = q.read(pws, true).into_iter().collect();
        let mut counts: HashMap<&str, u32> = HashMap::new();
        for p in &s.procs {
            *counts.entry(p.name.as_str()).or_default() += 1;
        }
        for p in s.procs.iter().filter(|p| counts[p.name.as_str()] == 1 && p.private_ws > 50 << 20) {
            if let Some(v) = pw.get(p.name.trim_end_matches(".exe")) {
                mem_err.push((p.private_ws as f64 - v).abs() / v.max(1.0) * 100.0);
            }
        }
    }

    let col = |f: fn(&(f64, f64, f64, f64, f64, f64, u64)) -> f64| rows.iter().map(f).collect::<Vec<f64>>();
    let sample_ms = col(|r| r.0);
    let total_diff = col(|r| (r.1 - r.2).abs());
    let sum_diff = col(|r| (r.4 - r.1).abs());
    let tm_gap = col(|r| r.3 - r.2);
    let own_cpu = col(|r| r.5);
    let own_mem = rows.iter().map(|r| r.6).max().unwrap_or(0);

    let mut out = String::new();
    let _ = writeln!(out, "# Purgeon bench\n\n{} on {} logical processors\n", c.info.cpu_name, c.info.logical);
    let _ = writeln!(out, "## Idle ({} samples at 1 s)\n", rows.len());
    let _ = writeln!(out, "| Check | Mean | p95 |\n|---|---|---|");
    let _ = writeln!(out, "| Total CPU vs perfmon % Processor Time (abs. difference, points) | {:.2} | {:.2} |", mean(&total_diff), pct(&total_diff, 0.95));
    let _ = writeln!(out, "| Sum of per-process CPU vs total (abs. difference, points) | {:.2} | {:.2} |", mean(&sum_diff), pct(&sum_diff, 0.95));
    let _ = writeln!(out, "| Task Manager % Utility minus % Processor Time (points) | {:.2} | {:.2} |", mean(&tm_gap), pct(&tm_gap, 0.95));
    let _ = writeln!(out, "| Private working set vs perfmon (error %, unique names over 50 MB) | {:.3} | {:.3} |", mean(&mem_err), pct(&mem_err, 0.95));
    let _ = writeln!(out, "| Time per update (ms) | {:.2} | {:.2} |", mean(&sample_ms), pct(&sample_ms, 0.95));
    let _ = writeln!(out, "| Purgeon's own CPU, sampler only (%) | {:.3} | {:.3} |", mean(&own_cpu), pct(&own_cpu, 0.95));
    let _ = writeln!(out, "\nPeak private working set of the bench process (sampler only, no window): {:.1} MB", own_mem as f64 / 1048576.0);
    let _ = writeln!(out, "Disk trace running: {} (needs administrator)", c.trace.is_some());
    println!("{out}");

    // ---- known loads ----
    let exe = std::env::temp_dir().join(LOAD_EXE);
    let copied = std::env::current_exe().ok().and_then(|me| std::fs::copy(me, &exe).ok()).is_some();
    let mut pf = Perf::new();
    let _ = writeln!(out, "\n## Known loads\n");
    if !copied {
        let _ = writeln!(out, "Could not copy the load program to the temp folder; skipped.");
    } else {
        let _ = writeln!(out, "Each load runs in `{LOAD_EXE}` at below-normal priority. \"Truth\" is what the load measured about itself. Error is (seen − truth) / truth.\n");
        let _ = writeln!(out, "| Load | Truth | Purgeon | Error | Windows counter | Error |\n|---|---|---|---|---|---|");
        let integ = |v: &[Seen], f: fn(&Seen) -> f64| v.iter().filter(|s| s.present).map(|s| f(s) * s.dt).sum::<f64>();
        for kind in ["cpu", "burst"] {
            match watch(&mut c, &mut pf, &exe, kind, 4) {
                Some((truth, v)) => {
                    let ours = integ(&v, |s| s.ours_cpu);
                    let pm = integ(&v, |s| s.pm_cpu);
                    let label = if kind == "cpu" { "Steady CPU, 4 threads 15 s (CPU seconds)" } else { "Bursty CPU, 4 threads 3 ms on / 10 ms off, 15 s (CPU seconds)" };
                    let _ = writeln!(out, "| {label} | {truth:.2} | {ours:.2} | {:+.1}% | {pm:.2} (`Process % Processor Time`) | {:+.1}% |", err_pct(ours, truth), err_pct(pm, truth));
                }
                None => {
                    let _ = writeln!(out, "| {kind} | failed | | | | |");
                }
            }
        }
        match watch(&mut c, &mut pf, &exe, "mem", 512) {
            Some((truth, v)) => {
                let base = v.iter().find(|s| s.present).copied().unwrap_or_default();
                let peak = |f: fn(&Seen) -> f64| v.iter().filter(|s| s.present).map(f).fold(0.0, f64::max);
                let ours = peak(|s| s.ours_pws) - base.ours_pws;
                let pm = peak(|s| s.pm_pws) - base.pm_pws;
                let mb = |b: f64| b / 1048576.0;
                let _ = writeln!(
                    out,
                    "| Memory, 512 MB touched (rise in private working set, MB) | {:.1} | {:.1} | {:+.2}% | {:.1} (`Process Working Set - Private`) | {:+.2}% |",
                    mb(truth),
                    mb(ours),
                    err_pct(ours, truth),
                    mb(pm),
                    err_pct(pm, truth)
                );
            }
            None => {
                let _ = writeln!(out, "| mem | failed | | | | |");
            }
        }
        match watch(&mut c, &mut pf, &exe, "disk", 256) {
            Some((truth, v)) => {
                let ours = integ(&v, |s| s.ours_io_write);
                let pm = integ(&v, |s| s.pm_io_write);
                let mb = |b: f64| b / 1048576.0;
                let _ = writeln!(
                    out,
                    "| Disk, 256 MB written by one program (MB) | {:.1} | {:.1} | {:+.1}% | {:.1} (`Process IO Write Bytes/sec`) | {:+.1}% |",
                    mb(truth),
                    mb(ours),
                    err_pct(ours, truth),
                    mb(pm),
                    err_pct(pm, truth)
                );
                // whole-drive writes include everything else writing at the time, so only a floor
                let all = |f: fn(&Seen) -> f64| v.iter().map(|s| f(s) * s.dt).sum::<f64>();
                let _ = writeln!(
                    out,
                    "| Disk, same test, all drives (MB, includes other programs) | ≥ {:.1} | {:.1} | | {:.1} (`PhysicalDisk Disk Write Bytes/sec`) | |",
                    mb(truth),
                    mb(all(|s| s.ours_drive_write)),
                    mb(all(|s| s.pm_drive_write))
                );
            }
            None => {
                let _ = writeln!(out, "| disk | failed | | | | |");
            }
        }
        let _ = std::fs::remove_file(&exe);
    }

    let _ = writeln!(out, "\n## Raw idle samples\n\n| ms | ours | % Processor Time | % Utility | per-process sum |\n|---|---|---|---|---|");
    for r in &rows {
        let _ = writeln!(out, "| {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |", r.0, r.1, r.2, r.3, r.4);
    }
    let _ = std::fs::write("bench-results.md", &out);
    println!("{out}");
}
