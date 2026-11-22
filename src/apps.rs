//! Which apps the user has open, and which of them have sat unused for a while. Feeds the
//! Dashboard's "Boost an app" picker and its "Free up memory" card.
//!
//! An app here is every process with the same exe name ("Discord.exe" ×6). It counts as unused
//! when, for at least `IDLE_FOR`, it was never the window in front and stayed near 0% CPU and GPU.
//! Purgeon only ever suggests; nothing is closed until the user confirms it for that one app.
//!
//! Purgeon only knows what it watched. An app that hasn't been used since Purgeon started has
//! been idle at least that long, maybe much longer, so it shows as "idle 16+ min". When Purgeon
//! restarts soon after it last ran, it picks up the times it had saved (`app-use.txt`).

use crate::sys::Snapshot;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long an app must sit unused before it is suggested
pub const IDLE_FOR: Duration = Duration::from_secs(10 * 60);
/// Smallest RAM use worth suggesting
pub const MIN_RAM: u64 = 200 << 20;

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(f: unsafe extern "system" fn(isize, isize) -> i32, l: isize) -> i32;
    fn IsWindowVisible(h: isize) -> i32;
    fn GetWindow(h: isize, cmd: u32) -> isize;
    fn GetWindowThreadProcessId(h: isize, pid: *mut u32) -> u32;
    fn GetForegroundWindow() -> isize;
    fn PostMessageW(h: isize, msg: u32, w: usize, l: isize) -> i32;
}

/// Visible top-level windows (not dialogs owned by another window), with their process ids
fn windows() -> Vec<(isize, u32)> {
    unsafe extern "system" fn cb(h: isize, l: isize) -> i32 {
        unsafe {
            if IsWindowVisible(h) != 0 && GetWindow(h, 4 /* GW_OWNER */) == 0 {
                let mut pid = 0;
                GetWindowThreadProcessId(h, &mut pid);
                (*(l as *mut Vec<(isize, u32)>)).push((h, pid));
            }
        }
        1
    }
    let mut v: Vec<(isize, u32)> = Vec::new();
    unsafe { EnumWindows(cb, &mut v as *mut _ as isize) };
    v
}

#[derive(Clone)]
pub struct App {
    /// Lower-case exe name, the key everything goes by
    pub exe: String,
    /// What to call it ("Discord", "Google Chrome")
    pub name: String,
    pub path: Option<Arc<str>>,
    /// Private memory of all its processes plus the largest shared part (as on the Dashboard)
    pub mem: u64,
    /// How long it has been unused (zero when in use)
    pub idle: Duration,
    /// False when it wasn't used at all while Purgeon watched: then `idle` is only how long
    /// Purgeon has been watching, and the real time is longer
    pub idle_known: bool,
    /// It has a window on screen
    pub windowed: bool,
}

struct Track {
    /// Last moment it was in front or busy
    used: Instant,
    /// CPU share, smoothed over about two minutes
    cpu: f32,
    /// `used` is a real moment of use (seen, or saved by the last run), not just when watching began
    known: bool,
}

struct Closing {
    exe: String,
    name: String,
    pids: Vec<u32>,
    polite: bool,
    at: Instant,
    avail_before: u64,
}

#[derive(Default)]
struct State {
    started: Option<Instant>,
    tracks: HashMap<String, Track>,
    apps: Vec<App>,
    /// pids of each app, for closing it
    pids: HashMap<String, Vec<u32>>,
    closing: Option<Closing>,
    /// Result of the last close: text, and whether it worked
    note: Option<(String, bool, Instant)>,
    avail: u64,
    avail_frac: f32,
    /// Last use of each app from the previous run, as unix seconds
    restored: HashMap<String, u64>,
    saved_at: Option<Instant>,
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn use_file() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(std::env::var_os("APPDATA")?).join("Purgeon").join("app-use.txt"))
}

/// The times saved by the last run, if it was running until a few minutes ago. After a longer
/// gap the apps may have been used meanwhile, so the old times would be wrong.
fn load_use() -> HashMap<String, u64> {
    let Some(text) = use_file().and_then(|p| std::fs::read_to_string(p).ok()) else { return HashMap::new() };
    let mut lines = text.lines();
    let saved: u64 = lines.next().and_then(|l| l.strip_prefix("saved\t")).and_then(|v| v.parse().ok()).unwrap_or(0);
    if now_unix().saturating_sub(saved) > 5 * 60 {
        return HashMap::new();
    }
    lines.filter_map(|l| l.split_once('\t')).filter_map(|(k, v)| Some((k.to_string(), v.parse().ok()?))).collect()
}

fn save_use(st: &State, now: Instant) {
    let Some(p) = use_file() else { return };
    let unix = now_unix();
    let mut out = format!("saved\t{unix}\n");
    for (exe, t) in &st.tracks {
        if t.known {
            let ago = now.duration_since(t.used).as_secs();
            out.push_str(&format!("{exe}\t{}\n", unix.saturating_sub(ago)));
        }
    }
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, out);
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

fn windows_dir() -> String {
    let w = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    format!("{}\\", w.trim_end_matches('\\').to_lowercase())
}

/// Called on every sample, also while Purgeon is in the tray
pub fn step(s: &Snapshot) {
    let now = Instant::now();
    let wins = windows();
    let on_screen: HashSet<u32> = wins.iter().map(|w| w.1).collect();
    let front = unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), &mut pid);
        pid
    };
    let windir = windows_dir();
    let me = std::process::id();

    struct G<'a> {
        name: &'a str,
        path: Option<Arc<str>>,
        cpu: f32,
        gpu: f32,
        private: u64,
        shared: u64,
        pids: Vec<u32>,
        front: bool,
        windowed: bool,
        skip: bool,
    }
    let mut groups: HashMap<String, G> = HashMap::new();
    for p in &s.procs {
        let g = groups.entry(p.name.to_lowercase()).or_insert_with(|| G {
            name: &p.name,
            path: None,
            cpu: 0.0,
            gpu: 0.0,
            private: 0,
            shared: 0,
            pids: vec![],
            front: false,
            windowed: false,
            skip: false,
        });
        if g.path.is_none() {
            g.path = p.path.clone();
        }
        g.cpu += p.cpu;
        g.gpu += p.gpu;
        g.private += p.private_ws;
        g.shared = g.shared.max(p.shared());
        g.pids.push(p.pid);
        g.front |= p.pid == front;
        g.windowed |= on_screen.contains(&p.pid);
        // Windows' own processes, services, and Purgeon itself are never suggested
        g.skip |= p.pid <= 4 || p.session == 0 || p.pid == me;
    }

    with(|st| {
        if st.started.is_none() {
            st.started = Some(now);
            st.restored = load_use();
        }
        let unix = now_unix();
        st.avail = s.mem.avail;
        st.avail_frac = if s.mem.total == 0 { 1.0 } else { s.mem.avail as f32 / s.mem.total as f32 };
        let dt = s.interval.max(0.25);
        let a = (dt / 120.0).min(1.0);
        let mut apps = Vec::new();
        let mut pids = HashMap::new();
        for (exe, g) in &groups {
            let restored = &st.restored;
            let t = st.tracks.entry(exe.clone()).or_insert_with(|| match restored.get(exe) {
                Some(&at) => {
                    let ago = Duration::from_secs(unix.saturating_sub(at));
                    Track { used: now.checked_sub(ago).unwrap_or(now), cpu: g.cpu, known: true }
                }
                None => Track { used: now, cpu: g.cpu, known: false },
            });
            t.cpu += (g.cpu - t.cpu) * a;
            if g.front || t.cpu >= 0.25 || g.cpu >= 2.0 || g.gpu >= 2.0 {
                t.used = now;
                t.known = true;
            }
            let Some(path) = g.path.clone() else { continue };
            if g.skip || path.to_lowercase().starts_with(&windir) {
                continue;
            }
            let base = g.name.trim_end_matches(".exe").trim_end_matches(".EXE");
            apps.push(App {
                exe: exe.clone(),
                name: crate::platform::friendly_name(base, Some(&path)),
                path: Some(path),
                mem: g.private + g.shared,
                idle: now.duration_since(t.used),
                idle_known: t.known,
                windowed: g.windowed,
            });
            pids.insert(exe.clone(), g.pids.clone());
        }
        st.tracks.retain(|k, _| groups.contains_key(k));
        apps.sort_by(|a, b| b.mem.cmp(&a.mem));
        st.apps = apps;
        st.pids = pids;
        // only for apps already open when Purgeon started; one opened later is new
        st.restored.clear();
        if st.saved_at.map_or(true, |t| now.duration_since(t) >= Duration::from_secs(60)) {
            st.saved_at = Some(now);
            save_use(st, now);
        }

        // A few seconds after a close, report what really happened
        if let Some(c) = &st.closing {
            if now.duration_since(c.at) >= Duration::from_secs(4) {
                let alive = c.pids.iter().any(|pid| s.procs.iter().any(|p| p.pid == *pid && p.name.eq_ignore_ascii_case(&c.exe)));
                st.note = Some(if !alive {
                    (
                        format!("Closed {}. Free memory went from {} to {}.", c.name, crate::fmt::bytes(c.avail_before), crate::fmt::bytes(s.mem.avail)),
                        true,
                        now,
                    )
                } else if c.polite {
                    (format!("{} is still open. It may be asking to save something, or it hid itself in the tray. Click Close again to end it.", c.name), false, now)
                } else {
                    (format!("Couldn't close all of {}. Part of it may run as administrator.", c.name), false, now)
                });
                st.closing = None;
            }
        }
    });
}

/// Apps with a window on screen, biggest first (for the Boost picker)
pub fn open_apps() -> Vec<App> {
    with(|st| st.apps.iter().filter(|a| a.windowed).cloned().collect())
}

/// What the Free up memory card shows
pub struct Suggestions {
    pub apps: Vec<App>,
    /// Still watching: how long until the first suggestions can appear
    pub warming: Option<Duration>,
    /// Share of RAM that is free, 0..1
    pub avail_frac: f32,
    pub note: Option<(String, bool)>,
    pub busy: bool,
}

pub fn suggestions(skip: Option<&str>) -> Suggestions {
    with(|st| {
        let up = st.started.map_or(Duration::ZERO, |t| t.elapsed());
        Suggestions {
            apps: st.apps.iter().filter(|a| a.idle >= IDLE_FOR && a.mem >= MIN_RAM && Some(a.exe.as_str()) != skip).cloned().collect(),
            warming: (up < IDLE_FOR).then(|| IDLE_FOR - up),
            avail_frac: st.avail_frac,
            note: st.note.as_ref().filter(|n| n.2.elapsed() < Duration::from_secs(20)).map(|n| (n.0.clone(), n.1)),
            busy: st.closing.is_some(),
        }
    })
}

/// True when the app has a window open, so closing it asks it nicely first
pub fn has_window(exe: &str) -> bool {
    let pids = with(|st| st.pids.get(exe).cloned().unwrap_or_default());
    windows().iter().any(|w| pids.contains(&w.1))
}

/// Closes an app the user confirmed. With a window open it is asked to close, as if its X was
/// clicked, so it can ask to save. Without one (background and tray apps) it is ended.
pub fn close(exe: &str) {
    let Some((name, pids, avail)) = with(|st| {
        let a = st.apps.iter().find(|a| a.exe == exe)?;
        Some((a.name.clone(), st.pids.get(exe).cloned()?, st.avail))
    }) else {
        return;
    };
    let wins: Vec<isize> = windows().into_iter().filter(|w| pids.contains(&w.1)).map(|w| w.0).collect();
    let polite = !wins.is_empty();
    let mut err = None;
    if polite {
        for h in wins {
            unsafe { PostMessageW(h, 0x0010 /* WM_CLOSE */, 0, 0) };
        }
    } else {
        for pid in &pids {
            if let Err(e) = crate::platform::end_process(*pid, exe) {
                err.get_or_insert(e);
            }
        }
    }
    with(|st| {
        st.note = None;
        match err {
            // nothing at all could be ended
            Some(e) if pids.len() == 1 => st.note = Some((format!("Couldn't close {name}: {e}"), false, Instant::now())),
            _ => st.closing = Some(Closing { exe: exe.to_string(), name, pids, polite, at: Instant::now(), avail_before: avail }),
        }
    });
}
