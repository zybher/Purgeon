//! Number formatting for the UI.

use std::time::Duration;

const K: f64 = 1024.0;

pub fn bytes(b: u64) -> String {
    let f = b as f64;
    if f < K {
        format!("{b} B")
    } else if f < K * K {
        format!("{:.0} KB", f / K)
    } else if f < K * K * K {
        format!("{:.1} MB", f / (K * K))
    } else {
        format!("{:.2} GB", f / (K * K * K))
    }
}

/// Gigabytes with one decimal, no unit
pub fn gb(b: u64) -> String {
    format!("{:.1}", b as f64 / (K * K * K))
}

pub fn rate(b: f64) -> String {
    if b < 1.0 {
        "0".into()
    } else if b < K {
        format!("{b:.0} B/s")
    } else if b < K * K {
        format!("{:.0} KB/s", b / K)
    } else if b < K * K * K {
        format!("{:.1} MB/s", b / (K * K))
    } else {
        format!("{:.2} GB/s", b / (K * K * K))
    }
}

/// Network speed in bits per second, the unit internet plans are sold in
pub fn bits(bytes_per_sec: f64) -> String {
    let b = bytes_per_sec * 8.0;
    if b < 1.0 {
        "0".into()
    } else if b < 1e3 {
        format!("{b:.0} bps")
    } else if b < 1e6 {
        format!("{:.0} Kbps", b / 1e3)
    } else if b < 1e9 {
        format!("{:.1} Mbps", b / 1e6)
    } else {
        format!("{:.2} Gbps", b / 1e9)
    }
}

/// An adapter's link speed (already in bits per second): "1 Gbps", "866.7 Mbps"
pub fn link(bps: u64) -> String {
    let b = bps as f64;
    let trim = |v: f64| {
        let s = format!("{v:.1}");
        s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
    };
    if b >= 1e9 {
        format!("{} Gbps", trim(b / 1e9))
    } else if b >= 1e6 {
        format!("{} Mbps", trim(b / 1e6))
    } else {
        format!("{} Kbps", trim(b / 1e3))
    }
}

pub fn pct(v: f32) -> String {
    if v < 0.05 {
        "0".into()
    } else {
        format!("{v:.1}%")
    }
}

pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn uptime(d: Duration) -> String {
    let s = d.as_secs();
    let (days, h, m, s) = (s / 86400, s / 3600 % 24, s / 60 % 60, s % 60);
    if days > 0 {
        format!("{days}d {h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

pub fn ago(secs: f32) -> String {
    if secs < 0.5 {
        "now".into()
    } else if secs < 60.0 {
        format!("{secs:.0}s ago")
    } else {
        format!("{:.0}m {:.0}s ago", (secs / 60.0).floor(), secs % 60.0)
    }
}
