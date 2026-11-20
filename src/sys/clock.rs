//! Live clock speed of every logical processor, and Task Manager's frequency-scaled
//! "% Processor Utility" figure (shown next to our time-based utilization).

use super::pdh::{Counter, Query};

pub struct ClockSampler {
    q: Query,
    perf: Counter,
    util: Counter,
    base_mhz: f32,
}

#[derive(Clone, Default)]
pub struct Clocks {
    /// Per logical processor, MHz (same order as the cores)
    pub mhz: Vec<f32>,
    /// Average over all processors, MHz
    pub avg_mhz: f32,
    /// What Task Manager shows as CPU utilization: busy time scaled by clock speed
    pub utility: Option<f32>,
}

impl ClockSampler {
    pub fn new(base_mhz: u32) -> Option<ClockSampler> {
        if base_mhz == 0 {
            return None;
        }
        let q = Query::new()?;
        let s = ClockSampler {
            perf: q.add(r"\Processor Information(*)\% Processor Performance"),
            util: q.add(r"\Processor Information(_Total)\% Processor Utility"),
            q,
            base_mhz: base_mhz as f32,
        };
        s.q.collect();
        Some(s)
    }

    pub fn sample(&mut self) -> Clocks {
        self.q.collect();
        let mut avg = 0.0;
        // Instances are "group,number"; "_Total" and "group,_Total" are the averages
        let mut per: Vec<((u32, u32), f32)> = Vec::new();
        for (id, v) in self.q.read(self.perf, false) {
            let mhz = (v as f32 / 100.0 * self.base_mhz).max(0.0);
            if id == "_Total" {
                avg = mhz;
                continue;
            }
            let mut it = id.split(',');
            if let (Some(Ok(g)), Some(Ok(n))) = (it.next().map(str::parse), it.next().map(str::parse)) {
                per.push(((g, n), mhz));
            }
        }
        per.sort_by_key(|x| x.0);
        let utility = self.q.read(self.util, false).first().map(|x| (x.1 as f32).clamp(0.0, 100.0));
        Clocks { mhz: per.into_iter().map(|x| x.1).collect(), avg_mhz: avg, utility }
    }
}
