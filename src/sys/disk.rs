//! Physical disk activity from the PhysicalDisk performance counters (what Task Manager's
//! disk graphs use), plus volume sizes.

use super::pdh::{Counter, Query};
use super::win::wide;

#[derive(Clone, Default)]
pub struct Disk {
    /// Counter instance, e.g. "0 C: D:"
    pub id: String,
    pub index: u32,
    pub letters: Vec<String>,
    pub read: f64,
    pub write: f64,
    /// Share of time the disk was busy, 0..100
    pub active: f32,
    /// Average time per transfer, milliseconds
    pub resp_ms: f32,
    pub queue: f32,
    /// (letter, total bytes, free bytes)
    pub volumes: Vec<(String, u64, u64)>,
}

pub struct DiskSampler {
    q: Query,
    read: Counter,
    write: Counter,
    idle: Counter,
    resp: Counter,
    queue: Counter,
    ticks: u32,
    volumes: Vec<(String, u64, u64)>,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetDiskFreeSpaceExW(root: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    fn GetLogicalDrives() -> u32;
    fn GetDriveTypeW(root: *const u16) -> u32;
}

fn volumes() -> Vec<(String, u64, u64)> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| {
            let l = format!("{}:", (b'A' + i) as char);
            let root = wide(&format!("{l}\\"));
            // fixed and removable disks only (no network, optical or RAM drives)
            if !matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3) {
                return None;
            }
            let (mut a, mut t, mut f) = (0u64, 0u64, 0u64);
            (unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut a, &mut t, &mut f) } != 0).then_some((l, t, f))
        })
        .collect()
}

impl DiskSampler {
    pub fn new() -> Option<DiskSampler> {
        let q = Query::new()?;
        let s = DiskSampler {
            read: q.add("\\PhysicalDisk(*)\\Disk Read Bytes/sec"),
            write: q.add("\\PhysicalDisk(*)\\Disk Write Bytes/sec"),
            idle: q.add("\\PhysicalDisk(*)\\% Idle Time"),
            resp: q.add("\\PhysicalDisk(*)\\Avg. Disk sec/Transfer"),
            queue: q.add("\\PhysicalDisk(*)\\Current Disk Queue Length"),
            q,
            ticks: 0,
            volumes: volumes(),
        };
        s.q.collect();
        Some(s)
    }

    pub fn sample(&mut self) -> Vec<Disk> {
        self.q.collect();
        // free space changes slowly: refresh it every ~10 samples
        self.ticks += 1;
        if self.ticks % 10 == 0 {
            self.volumes = volumes();
        }
        let mut disks: Vec<Disk> = Vec::new();
        let rows = [
            (self.q.read(self.read, false), 0),
            (self.q.read(self.write, false), 1),
            (self.q.read(self.idle, false), 2),
            (self.q.read(self.resp, false), 3),
            (self.q.read(self.queue, false), 4),
        ];
        for (vals, which) in rows {
            for (id, v) in vals {
                let Some(i) = slot(&mut disks, &id) else { continue };
                let d = &mut disks[i];
                match which {
                    0 => d.read = v,
                    1 => d.write = v,
                    2 => d.active = (100.0 - v as f32).clamp(0.0, 100.0),
                    3 => d.resp_ms = v as f32 * 1000.0,
                    _ => d.queue = v as f32,
                }
            }
        }
        for d in disks.iter_mut() {
            d.volumes = self.volumes.iter().filter(|v| d.letters.contains(&v.0)).cloned().collect();
        }
        disks.sort_by_key(|d| d.index);
        disks
    }
}

/// Index of the disk with this counter instance, adding it on first sight (skips "_Total")
fn slot(disks: &mut Vec<Disk>, id: &str) -> Option<usize> {
    if id == "_Total" {
        return None;
    }
    if let Some(i) = disks.iter().position(|d| d.id == id) {
        return Some(i);
    }
    let mut it = id.split_whitespace();
    let index = it.next().and_then(|n| n.parse().ok()).unwrap_or(u32::MAX);
    let letters: Vec<String> = it.map(str::to_string).collect();
    disks.push(Disk { id: id.to_string(), index, letters, ..Default::default() });
    Some(disks.len() - 1)
}
