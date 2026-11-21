//! ACPI thermal zones through the Windows performance counters (no administrator, no driver).
//! Most laptops expose one or more; many desktops expose none. A zone is a motherboard
//! reading chosen by the PC maker, not the CPU's own core sensor, and some report a fixed value.

use super::pdh::{Counter, Query};

pub struct ThermalSampler {
    q: Query,
    /// tenths of a kelvin (Windows 10 1709+)
    precise: Counter,
    /// whole kelvin
    coarse: Counter,
}

impl ThermalSampler {
    pub fn new() -> Option<ThermalSampler> {
        let q = Query::new()?;
        let s = ThermalSampler {
            precise: q.add(r"\Thermal Zone Information(*)\High Precision Temperature"),
            coarse: q.add(r"\Thermal Zone Information(*)\Temperature"),
            q,
        };
        s.q.collect();
        Some(s)
    }

    /// (zone name, °C) for every zone reporting a plausible temperature
    pub fn sample(&mut self) -> Vec<(String, f32)> {
        self.q.collect();
        let mut v: Vec<(String, f32)> = self.q.read(self.precise, false).into_iter().map(|(n, t)| (n, t as f32 / 10.0 - 273.15)).collect();
        if v.is_empty() {
            v = self.q.read(self.coarse, false).into_iter().map(|(n, t)| (n, t as f32 - 273.15)).collect();
        }
        v.retain(|(_, c)| (1.0..150.0).contains(c));
        for (n, _) in v.iter_mut() {
            // "\_TZ.THM0" -> "THM0"
            if let Some(i) = n.rfind('.') {
                *n = n[i + 1..].to_string();
            }
        }
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }
}
