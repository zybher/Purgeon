//! Decides when something is worth interrupting the user for. Works on snapshots only, so the
//! tray (and later the overlay) just shows what it returns.
//!
//! Every rule must hold for a while before it fires, fires once, and can fire again only after
//! the problem went away and a cooldown passed. On top of that, at most one alert every two
//! minutes, so a bad moment never turns into a stream of notifications.

use crate::fmt;
use crate::sys::Snapshot;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static ON: AtomicBool = AtomicBool::new(true);

pub fn load() {
    ON.store(crate::settings::flag("alerts", true), Ordering::Relaxed);
}
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}
pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
    crate::settings::set_flag("alerts", on);
}

pub struct Alert {
    pub title: String,
    pub body: String,
}

/// RAM in use above which the PC counts as nearly out of memory, and below which it has recovered
const RAM_FULL: f32 = 0.92;
const RAM_OK: f32 = 0.85;
/// One app holding at least this share of RAM (and at least `APP_MIN` bytes)
const APP_SHARE: f32 = 0.30;
const APP_MIN: u64 = 3 << 30;
/// GPU temperature that counts as hot, and the one it must cool to before warning again
const GPU_HOT: f32 = 85.0;
const GPU_OK: f32 = 80.0;
const GAP: Duration = Duration::from_secs(120);

struct Rule {
    /// When the condition started holding, while it holds
    since: Option<Instant>,
    /// Already warned about this episode
    fired: bool,
    last: Option<Instant>,
}

#[derive(Default)]
pub struct Watch {
    rules: HashMap<String, Rule>,
    last_any: Option<Instant>,
}

impl Watch {
    /// Feeds one rule. `bad`: the problem is there now; `ok`: it is clearly over (between the
    /// two, an episode neither starts nor ends, so a value hovering at the line can't re-trigger).
    fn step(&mut self, key: &str, bad: bool, ok: bool, hold: Duration, cooldown: Duration, now: Instant) -> bool {
        let r = self.rules.entry(key.to_string()).or_insert(Rule { since: None, fired: false, last: None });
        if ok {
            r.since = None;
            r.fired = false;
            return false;
        }
        if !bad {
            return false;
        }
        let since = *r.since.get_or_insert(now);
        let rested = r.last.map_or(true, |t| now - t >= cooldown);
        let quiet = self.last_any.map_or(true, |t| now - t >= GAP);
        if !r.fired && rested && quiet && now - since >= hold {
            r.fired = true;
            r.last = Some(now);
            self.last_any = Some(now);
            return true;
        }
        false
    }

    /// The alert to show for this snapshot, if any
    pub fn check(&mut self, s: &Snapshot) -> Option<Alert> {
        if !enabled() {
            return None;
        }
        let now = Instant::now();
        let total = s.mem.total.max(1);

        // RAM per app: private memory of every process with that name, plus the largest one's
        // shared memory (the same rule as the RAM column)
        let me = std::process::id();
        let mut apps: HashMap<&str, (u64, u64)> = HashMap::new();
        for p in &s.procs {
            if p.pid <= 4 || p.pid == me || p.name.eq_ignore_ascii_case("memory compression") || p.name.eq_ignore_ascii_case("registry") {
                continue;
            }
            let a = apps.entry(p.name.as_str()).or_default();
            a.0 += p.private_ws;
            a.1 = a.1.max(p.shared());
        }
        let mut apps: Vec<(&str, u64)> = apps.into_iter().map(|(n, (pr, sh))| (n, pr + sh)).collect();
        apps.sort_by(|a, b| b.1.cmp(&a.1));
        let name = |n: &str| n.strip_suffix(".exe").or_else(|| n.strip_suffix(".EXE")).unwrap_or(n).to_string();

        let used = s.mem.used_frac();
        if self.step("ram_full", used >= RAM_FULL, used < RAM_OK, Duration::from_secs(20), Duration::from_secs(20 * 60), now) {
            let top = apps.first().map(|(n, b)| format!(" {} is using the most ({}).", name(n), fmt::bytes(*b))).unwrap_or_default();
            return Some(Alert {
                title: "Your PC is running out of memory".into(),
                body: format!("{:.0}% of your RAM is in use.{top} Closing apps you aren't using will help.", used * 100.0),
            });
        }

        let limit = ((total as f32 * APP_SHARE) as u64).max(APP_MIN);
        let mut fire = None;
        let heavy: Vec<(String, u64)> = apps.iter().filter(|a| a.1 >= limit * 4 / 5).map(|(n, b)| (n.to_string(), *b)).collect();
        for (n, b) in &heavy {
            if fire.is_none() && self.step(&format!("app:{n}"), *b >= limit, false, Duration::from_secs(60), Duration::from_secs(60 * 60), now) {
                fire = Some((n.clone(), *b));
            }
        }
        // apps that dropped well below the line (or closed) start over
        for (k, r) in self.rules.iter_mut() {
            if let Some(n) = k.strip_prefix("app:") {
                if !heavy.iter().any(|h| h.0 == n) {
                    r.since = None;
                    r.fired = false;
                }
            }
        }
        if let Some((n, b)) = fire {
            return Some(Alert {
                title: format!("{} is using a lot of memory", name(&n)),
                body: format!(
                    "It's holding {}, {:.0}% of your RAM. If your PC feels slow, try closing or restarting it.",
                    fmt::bytes(b),
                    b as f32 / total as f32 * 100.0
                ),
            });
        }

        for g in &s.gpus {
            let Some(t) = g.sensors.temp else { continue };
            if self.step(&format!("gpu:{:x}", g.luid), t >= GPU_HOT, t < GPU_OK, Duration::from_secs(30), Duration::from_secs(30 * 60), now) {
                return Some(Alert {
                    title: "Your graphics card is running hot".into(),
                    body: format!("It's at {t:.0}°C. Make sure its fans are spinning and air can flow through your PC. Games may stutter if it keeps heating up."),
                });
            }
        }
        None
    }
}
