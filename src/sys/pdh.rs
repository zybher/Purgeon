//! A thin wrapper over the Performance Data Helper (the API behind perfmon and Task Manager).

use super::win::wide;

#[repr(C)]
struct PdhItem {
    name: *const u16,
    status: u32,
    value: f64, // read as i64 for PDH_FMT_LARGE (same 8 bytes)
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
    fn PdhCloseQuery(q: isize) -> u32;
}

pub struct Query {
    h: isize,
    buf: Vec<u8>,
}

/// A counter handle (0 when the counter does not exist on this system)
#[derive(Clone, Copy)]
pub struct Counter(isize);

impl Query {
    pub fn new() -> Option<Query> {
        let mut h = 0isize;
        (unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut h) } == 0).then(|| Query { h, buf: Vec::new() })
    }

    /// Adds a counter by its English path, e.g. `\PhysicalDisk(*)\Disk Read Bytes/sec`
    pub fn add(&self, path: &str) -> Counter {
        let mut c = 0isize;
        let w = wide(path);
        Counter(if unsafe { PdhAddEnglishCounterW(self.h, w.as_ptr(), 0, &mut c) } == 0 { c } else { 0 })
    }

    pub fn collect(&self) {
        unsafe { PdhCollectQueryData(self.h) };
    }

    /// (instance, value) for every instance of a wildcard counter.
    /// `large` reads integer counters (byte sizes) exactly instead of as doubles.
    pub fn read(&mut self, c: Counter, large: bool) -> Vec<(String, f64)> {
        if c.0 == 0 {
            return Vec::new();
        }
        let fmt = if large { PDH_FMT_LARGE } else { PDH_FMT_DOUBLE | PDH_FMT_NOCAP100 };
        for _ in 0..4 {
            let (mut size, mut count) = (self.buf.len() as u32, 0u32);
            let r = unsafe { PdhGetFormattedCounterArrayW(c.0, fmt, &mut size, &mut count, self.buf.as_mut_ptr() as *mut PdhItem) };
            if r == PDH_MORE_DATA || (r == 0 && self.buf.is_empty() && count > 0) {
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

impl Drop for Query {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.h) };
    }
}
