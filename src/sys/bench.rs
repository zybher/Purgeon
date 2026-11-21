//! Accuracy and overhead check, built only with `--features bench`:
//!
//! ```text
//! cargo run --release --features bench -- --bench [seconds]
//! ```
//!
//! Samples once a second next to the perfmon counters Task Manager reads, and measures what
//! Purgeon itself costs. Writes `bench-results.md` in the working directory. It does nothing
//! heavy, but numbers are cleanest with no game or other big load running.

use super::*;
use std::fmt::Write as _;

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
    let _ = writeln!(out, "# Purgeon bench\n\n{} on {} logical processors, {} samples at 1 s\n", c.info.cpu_name, c.info.logical, rows.len());
    let _ = writeln!(out, "| Check | Mean | p95 |\n|---|---|---|");
    let _ = writeln!(out, "| Total CPU vs perfmon % Processor Time (abs. difference, points) | {:.2} | {:.2} |", mean(&total_diff), pct(&total_diff, 0.95));
    let _ = writeln!(out, "| Sum of per-process CPU vs total (abs. difference, points) | {:.2} | {:.2} |", mean(&sum_diff), pct(&sum_diff, 0.95));
    let _ = writeln!(out, "| Task Manager % Utility minus % Processor Time (points) | {:.2} | {:.2} |", mean(&tm_gap), pct(&tm_gap, 0.95));
    let _ = writeln!(out, "| Private working set vs perfmon (error %, unique names over 50 MB) | {:.3} | {:.3} |", mean(&mem_err), pct(&mem_err, 0.95));
    let _ = writeln!(out, "| Time per update (ms) | {:.2} | {:.2} |", mean(&sample_ms), pct(&sample_ms, 0.95));
    let _ = writeln!(out, "| Purgeon's own CPU, sampler only (%) | {:.3} | {:.3} |", mean(&own_cpu), pct(&own_cpu, 0.95));
    let _ = writeln!(out, "\nPeak private working set of the bench process (sampler only, no window): {:.1} MB", own_mem as f64 / 1048576.0);
    let _ = writeln!(out, "Disk trace running: {} (needs administrator)", c.trace.is_some());
    let _ = writeln!(out, "\n## Raw samples\n\n| ms | ours | % Processor Time | % Utility | per-process sum |\n|---|---|---|---|---|");
    for r in &rows {
        let _ = writeln!(out, "| {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |", r.0, r.1, r.2, r.3, r.4);
    }
    let _ = std::fs::write("bench-results.md", &out);
    println!("{out}");
}
