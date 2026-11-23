//! Boost: one game the user picks gets a higher CPU priority while it runs, and Windows switches
//! to its High performance power plan for as long as that app is open. Everything goes back the
//! way it was when the app closes, when the boost is turned off, or when Purgeon quits.
//!
//! What it touches, and nothing else:
//! - The boosted game's priority class, from Normal to Above normal (never High or Realtime, which
//!   can starve Windows itself). A priority someone already changed is left alone. This needs a
//!   handle with PROCESS_SET_INFORMATION on that game only (helpers and anti-cheat in its
//!   folder are skipped, see `games::Target`), asked for only when the priority changes
//!   (and at each switch of the FPS test, see `trial`) and closed straight away. Games whose
//!   anti-cheat refuses it simply stay at Normal and aren't asked again.
//! - With "Steadier frames" switched on (experimental, off by default): which cores the game
//!   prefers, and the priority of background apps the user picked. See `steady`.
//! - The active power plan. The plan that was active is written to settings.ini first, so even if
//!   Purgeon crashes, the next start puts it back.

use crate::games::{self, Target};
use crate::steady;
use crate::sys::{win, Snapshot};
use std::ffi::c_void;
use std::sync::Mutex;

const NORMAL: u32 = 0x20;
const ABOVE_NORMAL: u32 = 0x8000;
const PROCESS_SET_INFORMATION: u32 = 0x0200;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Guid(u32, u16, u16, [u8; 8]);

/// Windows' built-in "High performance" plan
const HIGH_PERF: Guid = Guid(0x8c5e7fda, 0xe8bf, 0x4a96, [0x9a, 0x85, 0xa6, 0xe2, 0x3a, 0x8c, 0x63, 0x5c]);

#[link(name = "powrprof")]
extern "system" {
    fn PowerGetActiveScheme(root: isize, active: *mut *mut Guid) -> u32;
    fn PowerSetActiveScheme(root: isize, scheme: *const Guid) -> u32;
    fn PowerReadFriendlyName(root: isize, scheme: *const Guid, sub: *const Guid, setting: *const Guid, buf: *mut u8, size: *mut u32) -> u32;
}
#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(p: *mut c_void) -> *mut c_void;
    fn GetPriorityClass(h: isize) -> u32;
    fn SetPriorityClass(h: isize, class: u32) -> i32;
}

impl Guid {
    fn text(&self) -> String {
        let d = self.3;
        format!("{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}", self.0, self.1, self.2, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7])
    }
    fn parse(s: &str) -> Option<Guid> {
        let h: String = s.chars().filter(|c| *c != '-').collect();
        if h.len() != 32 {
            return None;
        }
        let n = |a: usize, b: usize| u64::from_str_radix(&h[a..b], 16).ok();
        let mut d = [0u8; 8];
        for (i, b) in d.iter_mut().enumerate() {
            *b = n(16 + i * 2, 18 + i * 2)? as u8;
        }
        Some(Guid(n(0, 8)? as u32, n(8, 12)? as u16, n(12, 16)? as u16, d))
    }
}

fn active_plan() -> Option<Guid> {
    unsafe {
        let mut p = std::ptr::null_mut();
        if PowerGetActiveScheme(0, &mut p) != 0 || p.is_null() {
            return None;
        }
        let g = *p;
        LocalFree(p as _);
        Some(g)
    }
}

fn plan_name(g: &Guid) -> String {
    unsafe {
        let mut size = 0u32;
        PowerReadFriendlyName(0, g, std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), &mut size);
        if size < 2 {
            return "your usual plan".into();
        }
        let mut buf = vec![0u16; size as usize / 2];
        if PowerReadFriendlyName(0, g, std::ptr::null(), std::ptr::null(), buf.as_mut_ptr() as _, &mut size) != 0 {
            return "your usual plan".into();
        }
        String::from_utf16_lossy(&buf).trim_end_matches('\0').to_string()
    }
}

/// How raising the priority went
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Not running yet
    Waiting,
    Raised,
    /// Someone (the app, the user) had already set it to something other than Normal
    LeftAlone,
    /// The app refused the handle (anti-cheat, or it runs as administrator)
    Refused,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Plan {
    /// The app isn't running, so the usual plan is active
    Usual,
    /// Switched to High performance; the name of the plan that goes back afterwards
    Switched(String),
    AlreadyHigh,
    /// This PC has no High performance plan (common on laptops with Modern Standby)
    Missing,
}

#[derive(Clone)]
pub struct Status {
    /// Which processes are boosted, and how to show them ("Roblox")
    pub target: Target,
    pub name: String,
    pub running: bool,
    pub priority: Priority,
    pub plan: Plan,
}

struct State {
    target: Option<(Target, String)>,
    /// What happened per boosted process id; Raised ones are put back on stop
    pids: Vec<(u32, Priority)>,
    /// The plan to go back to, while we've switched
    saved: Option<Guid>,
    plan: Plan,
    /// The FPS test has Boost off for one of its rounds
    held: bool,
    /// Purgeon is quitting: nothing gets boosted again
    closed: bool,
}

static STATE: Mutex<State> = Mutex::new(State { target: None, pids: Vec::new(), saved: None, plan: Plan::Usual, held: false, closed: false });

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// At start-up: put back a power plan a crashed run left switched, then pick up the saved boost
pub fn load() {
    // a screenshot run never boosts, and never undoes what the running copy of Purgeon did
    if crate::shot::on() {
        return;
    }
    if let Some(g) = crate::settings::text("boost_plan_restore").and_then(|t| Guid::parse(&t)) {
        unsafe { PowerSetActiveScheme(0, &g) };
        crate::settings::set_text("boost_plan_restore", None);
    }
    steady::load();
    if let Some(exe) = crate::settings::text("boost") {
        let saved = crate::settings::text("boost_dirs").unwrap_or_default();
        let dirs: Vec<String> = saved.split('|').filter(|d| games::safe_dir(d)).map(String::from).collect();
        // folders that don't pass never fall back to a name like javaw.exe
        if (saved.is_empty() && !exe.is_empty()) || !dirs.is_empty() {
            let name = crate::settings::text("boost_name").unwrap_or_else(|| exe.clone());
            state().target = Some((Target { exe, dirs }, name));
        }
    }
}

pub fn status() -> Option<Status> {
    let st = state();
    let (target, name) = st.target.clone()?;
    let running = !st.pids.is_empty();
    let priority = if !running {
        Priority::Waiting
    } else if st.pids.iter().any(|p| p.1 == Priority::Raised) {
        Priority::Raised
    } else if st.pids.iter().all(|p| p.1 == Priority::Refused) {
        Priority::Refused
    } else {
        Priority::LeftAlone
    };
    Some(Status { target, name, running, priority, plan: st.plan.clone() })
}

/// Starts boosting this game, now or whenever it opens; `icon` is the file its icon comes from
pub fn start(target: &Target, name: &str, icon: Option<&str>) {
    if crate::shot::on() {
        return;
    }
    stop();
    state().target = Some((target.clone(), name.to_string()));
    crate::settings::set_text("boost", Some(&target.exe));
    let dirs = target.dirs.join("|");
    crate::settings::set_text("boost_dirs", (!dirs.is_empty()).then_some(dirs.as_str()));
    crate::settings::set_text("boost_name", Some(name));
    crate::settings::set_text("boost_path", icon);
}

/// Turns the boost off and puts everything back
pub fn stop() {
    {
        let mut st = state();
        release(&mut st);
        // under the same lock, so a sample can't boost it again in between
        st.target = None;
    }
    steady::release();
    crate::settings::set_text("boost", None);
    crate::settings::set_text("boost_name", None);
    crate::settings::set_text("boost_path", None);
    crate::settings::set_text("boost_dirs", None);
}

/// Puts everything back but keeps the choice for next time (Purgeon is quitting)
pub fn on_quit() {
    let mut st = state();
    st.closed = true;
    release(&mut st);
    steady::on_quit();
}

/// The boosted game's processes
pub fn pids() -> Vec<u32> {
    state().pids.iter().map(|p| p.0).collect()
}

/// For the FPS test: off (`true`) or back on, keeping the pick. With "Steadier frames" on only
/// that part switches, so the test shows what it adds to Boost. Otherwise everything Boost does.
pub fn hold(off: bool) {
    let mut st = state();
    if st.closed || st.target.is_none() {
        return;
    }
    if steady::on() {
        steady::hold(off);
        // Boost itself stays on throughout
        if !st.held {
            return;
        }
        return unhold(&mut st);
    }
    if st.held == off {
        return;
    }
    st.held = off;
    if off {
        lower(&st.pids);
        restore_plan(&mut st);
    } else {
        unhold(&mut st);
    }
}

fn unhold(st: &mut State) {
    st.held = false;
    if !st.pids.is_empty() {
        // only what Boost had raised; a game that refused isn't asked again
        for &(pid, p) in &st.pids {
            if p == Priority::Raised {
                raise(pid);
            }
        }
        switch_plan(st);
    }
}

fn release(st: &mut State) {
    lower(&st.pids);
    st.pids.clear();
    st.held = false;
    restore_plan(st);
}

fn lower(pids: &[(u32, Priority)]) {
    for &(pid, p) in pids {
        if p == Priority::Raised {
            unsafe {
                let h = win::OpenProcess(PROCESS_SET_INFORMATION | win::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if h != 0 {
                    // only if it is still what we set
                    if GetPriorityClass(h) == ABOVE_NORMAL {
                        SetPriorityClass(h, NORMAL);
                    }
                    win::CloseHandle(h);
                }
            }
        }
    }
}

fn restore_plan(st: &mut State) {
    if let Some(g) = st.saved.take() {
        unsafe { PowerSetActiveScheme(0, &g) };
        crate::settings::set_text("boost_plan_restore", None);
    }
    st.plan = Plan::Usual;
}

/// Raises one process's priority if it is at Normal
fn raise(pid: u32) -> Priority {
    unsafe {
        let h = win::OpenProcess(PROCESS_SET_INFORMATION | win::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h == 0 {
            return Priority::Refused;
        }
        let r = match GetPriorityClass(h) {
            NORMAL if SetPriorityClass(h, ABOVE_NORMAL) != 0 => Priority::Raised,
            NORMAL => Priority::Refused,
            _ => Priority::LeftAlone,
        };
        win::CloseHandle(h);
        r
    }
}

/// Follows the boosted app on every sample (also while Purgeon sits in the tray)
pub fn step(s: &Snapshot) {
    let round = steady::round();
    let game = follow(s);
    let picked = state().target.is_some();
    // outside the lock: it asks Windows about sound. A stop meanwhile makes the round stale.
    if picked {
        steady::step(s, &game, round);
    }
}

/// The priority and power plan part; returns the game's processes
fn follow(s: &Snapshot) -> Vec<u32> {
    let mut st = state();
    if st.closed {
        return vec![];
    }
    let Some((target, _)) = st.target.clone() else { return vec![] };
    // session 0 is services, which a game's window never runs as
    let live: Vec<u32> = s.procs.iter().filter(|p| p.session != 0 && target.matches(p)).map(|p| p.pid).collect();
    st.pids.retain(|p| live.contains(&p.0));
    if live.is_empty() {
        st.held = false;
        restore_plan(&mut st);
        return live;
    }
    if st.held {
        return live;
    }
    for pid in &live {
        if !st.pids.iter().any(|p| p.0 == *pid) {
            let r = raise(*pid);
            st.pids.push((*pid, r));
        }
    }
    switch_plan(&mut st);
    live
}

fn switch_plan(st: &mut State) {
    if st.plan == Plan::Usual {
        st.plan = {
            match active_plan() {
                Some(g) if g == HIGH_PERF => Plan::AlreadyHigh,
                Some(g) => {
                    // written down before switching, so a crash can't leave the PC on it
                    crate::settings::set_text("boost_plan_restore", Some(&g.text()));
                    if unsafe { PowerSetActiveScheme(0, &HIGH_PERF) } == 0 {
                        st.saved = Some(g);
                        Plan::Switched(plan_name(&g))
                    } else {
                        crate::settings::set_text("boost_plan_restore", None);
                        Plan::Missing
                    }
                }
                None => Plan::Missing,
            }
        };
    }
}
