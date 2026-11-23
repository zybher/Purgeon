//! Boost's experimental part, "Steadier frames". It is off until the user switches it on, and
//! then does two things, only while the boosted game runs:
//!
//! - The game prefers the fast cores of a CPU that has fast and efficient ones. This is a CPU set,
//!   the soft kind of affinity, and needs PROCESS_SET_LIMITED_INFORMATION on the game: less than
//!   the priority change does.
//! - Background apps the user picked one by one go from Normal to Below normal. Windows' own
//!   programs, services, games, launchers, anti-cheat, browsers, media and voice apps, shells and
//!   anything running as administrator can't be picked. A picked app is back at Normal while it
//!   is in front, has sound going or is drawing with the GPU. Nothing is closed.
//!
//! Every change is written to settings.ini before it is made, with the process's start time, so a
//! reused process id is never touched and the next start undoes what a crashed run left. Each
//! undo only happens when the value is still the one Purgeon set. A child inherits Below normal
//! from its parent (Windows does that for this class), so such children are put back to Normal,
//! except Windows' own programs, games, anti-cheat and anything elevated. Those are never touched
//! and keep it until they exit, as do children left by a run that crashed.

use crate::sys::{audio, win, Proc, Snapshot};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const NORMAL: u32 = 0x20;
const BELOW_NORMAL: u32 = 0x4000;
/// The base priority that class shows in the process list
const BELOW_NORMAL_BASE: i32 = 6;
const SET_INFORMATION: u32 = 0x0200;
const SET_LIMITED_INFORMATION: u32 = 0x2000;
/// Fewer fast threads than this would starve a game, so it is left on every core
const MIN_FAST: usize = 8;
/// Share of the whole CPU, smoothed, from which a background app is suggested
const BUSY: f32 = 0.5;
const SUGGESTED: usize = 3;
/// How long an app stays at Normal after it was last in front, heard or drawing
const REST: Duration = Duration::from_secs(10);
/// Asking about sound and looking for children cost some milliseconds, so not on every sample
const LOOK_EVERY: Duration = Duration::from_secs(3);

const KEY_ON: &str = "boost_steady";
const KEY_APPS: &str = "boost_steady_apps";
const KEY_UNDO: &str = "boost_steady_undo";

#[link(name = "kernel32")]
extern "system" {
    fn GetPriorityClass(h: isize) -> u32;
    fn SetPriorityClass(h: isize, class: u32) -> i32;
    fn GetProcessTimes(h: isize, created: *mut win::FILETIME, exited: *mut win::FILETIME, kernel: *mut win::FILETIME, user: *mut win::FILETIME) -> i32;
    fn GetSystemTimeAsFileTime(t: *mut win::FILETIME);
    fn GetExitCodeProcess(h: isize, code: *mut u32) -> i32;
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
    fn GetSystemCpuSetInformation(buf: *mut u8, len: u32, got: *mut u32, process: isize, flags: u32) -> i32;
    fn GetProcessDefaultCpuSets(h: isize, ids: *mut u32, n: u32, needed: *mut u32) -> i32;
    fn SetProcessDefaultCpuSets(h: isize, ids: *const u32, n: u32) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn GetTokenInformation(token: isize, class: u32, buf: *mut u32, len: u32, got: *mut u32) -> i32;
}

pub struct Fast {
    ids: Vec<u32>,
    pub cores: usize,
}

/// One logical processor: its CPU set id, its core and how fast its kind is
#[derive(Clone, Copy)]
struct Cpu {
    id: u32,
    core: (u16, u8),
    class: u8,
}

/// The fastest kind of core, when there's more than one kind and enough of that one
fn fast_of(cpus: &[Cpu]) -> Option<Fast> {
    let top = cpus.iter().map(|c| c.class).max()?;
    if cpus.iter().all(|c| c.class == top) {
        return None;
    }
    let best: Vec<&Cpu> = cpus.iter().filter(|c| c.class == top).collect();
    if best.len() < MIN_FAST {
        return None;
    }
    let cores = best.iter().map(|c| c.core).collect::<HashSet<_>>().len();
    let mut ids: Vec<u32> = best.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    Some(Fast { ids, cores })
}

fn read_cpus() -> Vec<Cpu> {
    unsafe {
        let mut len = 0u32;
        GetSystemCpuSetInformation(std::ptr::null_mut(), 0, &mut len, 0, 0);
        let mut buf = vec![0u8; len as usize];
        if len == 0 || GetSystemCpuSetInformation(buf.as_mut_ptr(), len, &mut len, 0, 0) == 0 {
            return vec![];
        }
        let buf = &buf[..(len as usize).min(buf.len())];
        let word = |at: usize| u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]);
        let (mut at, mut cpus) = (0, vec![]);
        while at + 20 <= buf.len() {
            let size = word(at) as usize;
            if size < 20 {
                break;
            }
            let flags = buf[at + 19];
            // type 0 is a CPU set; one reserved for another process (allocated, not to us) is left out
            if word(at + 4) == 0 && (flags & 2 == 0 || flags & 4 != 0) {
                let group = u16::from_le_bytes([buf[at + 12], buf[at + 13]]);
                cpus.push(Cpu { id: word(at + 8), core: (group, buf[at + 15]), class: buf[at + 18] });
            }
            at += size;
        }
        cpus
    }
}

/// None on a CPU whose cores are all alike
pub fn fast() -> Option<&'static Fast> {
    static FAST: OnceLock<Option<Fast>> = OnceLock::new();
    FAST.get_or_init(|| fast_of(&read_cpus())).as_ref()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cores {
    Waiting,
    Preferred,
    /// The game or the user had already chosen cores
    LeftAlone,
    Refused,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum App {
    /// Suggested, not picked
    Offered,
    /// Picked, waiting for the game or for the app
    Waiting,
    Lowered,
    /// Picked, but in front, heard or drawing
    InUse,
    LeftAlone,
}

#[derive(Clone)]
pub struct Pick {
    /// The exe's path in lower case, what a pick goes by
    pub key: String,
    pub name: String,
    pub path: Arc<str>,
    pub cpu: f32,
    pub app: App,
}

#[derive(Clone)]
pub struct View {
    pub on: bool,
    pub cores: Cores,
    pub apps: Vec<Pick>,
}

impl View {
    /// Something differs with it on, so a test has something to compare
    pub fn changes(&self) -> bool {
        self.on && (self.cores == Cores::Preferred || self.apps.iter().any(|a| a.app == App::Lowered))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Cores,
    Priority,
}

/// One change to undo
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Change {
    kind: Kind,
    pid: u32,
    created: u64,
    since: u64,
}

fn encode(list: &[Change]) -> String {
    list.iter()
        .map(|c| format!("{}:{}:{}:{}", if c.kind == Kind::Cores { "c" } else { "p" }, c.pid, c.created, c.since))
        .collect::<Vec<_>>()
        .join(",")
}

fn decode(text: &str) -> Vec<Change> {
    text.split(',')
        .filter_map(|item| {
            let v: Vec<&str> = item.split(':').collect();
            let [kind, pid, created, since] = v[..] else { return None };
            let kind = match kind {
                "c" => Kind::Cores,
                "p" => Kind::Priority,
                _ => return None,
            };
            Some(Change { kind, pid: pid.parse().ok().filter(|&p| p > 4)?, created: created.parse().ok()?, since: since.parse().ok()? })
        })
        .collect()
}

fn filetime_now() -> u64 {
    let mut t = win::FILETIME::default();
    unsafe { GetSystemTimeAsFileTime(&mut t) };
    t.v()
}

/// A handle, closed on drop
struct Handle(isize);

impl Handle {
    fn open(pid: u32, access: u32) -> Option<Handle> {
        let h = unsafe { win::OpenProcess(access | win::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        (h != 0).then_some(Handle(h))
    }

    /// When it started, if it is still running
    fn created(&self) -> Option<u64> {
        let (mut c, mut e, mut k, mut u) = Default::default();
        let mut code = 0;
        unsafe {
            if GetProcessTimes(self.0, &mut c, &mut e, &mut k, &mut u) == 0 || GetExitCodeProcess(self.0, &mut code) == 0 {
                return None;
            }
        }
        (code == 259).then(|| c.v())
    }

    fn class(&self) -> u32 {
        unsafe { GetPriorityClass(self.0) }
    }

    fn cpu_sets(&self) -> Option<Vec<u32>> {
        let (mut buf, mut n) = ([0u32; 512], 0u32);
        if unsafe { GetProcessDefaultCpuSets(self.0, buf.as_mut_ptr(), buf.len() as u32, &mut n) } == 0 {
            return None;
        }
        let mut v = buf.get(..n as usize)?.to_vec();
        v.sort_unstable();
        Some(v)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { win::CloseHandle(self.0) };
    }
}

/// The query-only look before any handle that could change something is asked for: the start
/// time when the change applies, or why it doesn't
fn look(kind: Kind, pid: u32, fast: &[u32]) -> Result<u64, Cores> {
    let h = Handle::open(pid, 0).ok_or(Cores::Refused)?;
    let created = h.created().ok_or(Cores::Refused)?;
    let free = match kind {
        Kind::Cores => !fast.is_empty() && h.cpu_sets().ok_or(Cores::Refused)?.is_empty(),
        Kind::Priority => h.class() == NORMAL,
    };
    if free { Ok(created) } else { Err(Cores::LeftAlone) }
}

fn apply(c: &Change, fast: &[u32]) -> bool {
    let access = if c.kind == Kind::Cores { SET_LIMITED_INFORMATION } else { SET_INFORMATION };
    let Some(h) = Handle::open(c.pid, access) else { return false };
    if h.created() != Some(c.created) {
        return false;
    }
    unsafe {
        match c.kind {
            Kind::Cores => SetProcessDefaultCpuSets(h.0, fast.as_ptr(), fast.len() as u32) != 0,
            Kind::Priority => h.class() == NORMAL && SetPriorityClass(h.0, BELOW_NORMAL) != 0,
        }
    }
}

/// True when nothing is left to undo
fn undo(c: &Change, fast: &[u32]) -> bool {
    let Some(q) = Handle::open(c.pid, 0) else { return true };
    if q.created() != Some(c.created) {
        return true;
    }
    let ours = match c.kind {
        Kind::Cores => q.cpu_sets().is_some_and(|v| !v.is_empty() && v == fast),
        Kind::Priority => q.class() == BELOW_NORMAL,
    };
    if !ours {
        return true;
    }
    let access = if c.kind == Kind::Cores { SET_LIMITED_INFORMATION } else { SET_INFORMATION };
    let Some(h) = Handle::open(c.pid, access) else { return false };
    if h.created() != Some(c.created) {
        return true;
    }
    unsafe {
        match c.kind {
            Kind::Cores => SetProcessDefaultCpuSets(h.0, std::ptr::null(), 0) != 0,
            Kind::Priority => SetPriorityClass(h.0, NORMAL) != 0,
        }
    }
}

struct Listed {
    pid: u32,
    parent: u32,
    created: u64,
    session: u32,
    base: i32,
}

fn processes() -> Vec<Listed> {
    unsafe {
        let mut buf = vec![0u64; 128 << 10];
        loop {
            let bytes = (buf.len() * 8) as u32;
            let mut need = 0u32;
            let st = win::NtQuerySystemInformation(win::SYSTEM_PROCESS_INFORMATION_CLASS, buf.as_mut_ptr() as _, bytes, &mut need);
            if st == win::STATUS_INFO_LENGTH_MISMATCH && buf.len() < 8 << 20 {
                buf.resize(buf.len() * 2, 0);
                continue;
            }
            if st != 0 {
                return vec![];
            }
            let (mut at, mut list) = (0usize, vec![]);
            let base = buf.as_ptr() as *const u8;
            loop {
                let p = std::ptr::read_unaligned(base.add(at) as *const win::SYSTEM_PROCESS_INFORMATION);
                list.push(Listed {
                    pid: p.UniqueProcessId as u32,
                    parent: p.InheritedFromUniqueProcessId as u32,
                    created: p.CreateTime as u64,
                    session: p.SessionId,
                    base: p.BasePriority,
                });
                if p.NextEntryOffset == 0 {
                    return list;
                }
                at += p.NextEntryOffset as usize;
            }
        }
    }
}

/// Children that got Below normal from a parent Purgeon lowered
fn inherited<'a>(parents: &[Change], list: &'a [Listed]) -> Vec<&'a Listed> {
    list.iter()
        .filter(|p| p.base == BELOW_NORMAL_BASE)
        .filter(|p| !parents.iter().any(|c| c.pid == p.pid && c.created == p.created))
        .filter(|p| {
            parents.iter().any(|c| {
                // not the child of a later process that got the parent's id
                let reused = || list.iter().any(|q| q.pid == c.pid && q.created != c.created && q.created <= p.created);
                c.kind == Kind::Priority && c.pid == p.parent && p.created >= c.since && !reused()
            })
        })
        .collect()
}

fn heal_children(parents: &[Change]) {
    if !parents.iter().any(|c| c.kind == Kind::Priority) {
        return;
    }
    // without the list of games there's no telling which children are games
    let Some(games) = crate::games::known() else { return };
    let session = my_session();
    for p in inherited(parents, &processes()) {
        if p.session != session {
            continue;
        }
        let Some(path) = win::process_path(p.pid) else { continue };
        if !may_heal(&path, &games) || !ordinary(p.pid) {
            continue;
        }
        undo(&Change { kind: Kind::Priority, pid: p.pid, created: p.created, since: 0 }, &[]);
    }
}

fn my_session() -> u32 {
    static MINE: OnceLock<u32> = OnceLock::new();
    *MINE.get_or_init(|| {
        let mut s = u32::MAX;
        unsafe { ProcessIdToSessionId(std::process::id(), &mut s) };
        s
    })
}

fn stem(path: &str) -> String {
    let file = path.rsplit('\\').next().unwrap_or(path).to_lowercase();
    file.strip_suffix(".exe").unwrap_or(&file).to_string()
}

/// Browsers, media, voice and capture apps (they can play without sound), and programs whose
/// whole job is starting others
fn is_shell_or_media(path: &str) -> bool {
    const NAMES: &[&str] = &[
        "chrome", "msedge", "msedgewebview2", "firefox", "opera", "brave", "vivaldi", "arc", "zen", "chromium",
        "spotify", "vlc", "mpv", "mpc-hc", "mpc-hc64", "mpc-be64", "potplayermini64", "itunes", "applemusic", "plex", "stremio", "wmplayer",
        "discord", "teams", "ms-teams", "zoom", "slack", "skype", "telegram", "whatsapp", "guilded", "ts3client_win64", "mumble",
        "obs64", "obs32", "medal", "streamlabs desktop", "parsecd", "sunshine", "voicemod",
        "windowsterminal", "wt", "cmd", "powershell", "pwsh", "conhost", "openconsole", "bash", "sh", "wsl", "explorer",
    ];
    NAMES.contains(&stem(path).as_str())
}

fn windows_own(path: &str) -> bool {
    let windows = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()).to_lowercase();
    path.to_lowercase().starts_with(&format!("{}\\", windows.trim_end_matches('\\')))
}

/// Never picked
fn off_limits(path: &str) -> bool {
    is_shell_or_media(path) || untouchable(path)
}

/// A child that inherited the lower priority may get Normal back, unless it is one of these
fn may_heal(path: &str, games: &[crate::games::Game]) -> bool {
    !untouchable(path) && !games.iter().any(|g| g.target.covers(path))
}

/// settings.ini is only text, so an undo read from it is checked like a change
fn may_restore(kind: Kind, path: &str) -> bool {
    match kind {
        Kind::Cores => !windows_own(path),
        Kind::Priority => !off_limits(path),
    }
}

/// Never touched, whatever it is doing
fn untouchable(path: &str) -> bool {
    if windows_own(path) {
        return true;
    }
    let p = path.to_lowercase();
    const FOLDERS: &[&str] = &[
        "\\steamapps\\", "\\steam\\", "\\epic games\\", "\\riot games\\", "\\riot vanguard\\", "\\xboxgames\\", "\\roblox\\", "\\gog galaxy\\",
        "\\ubisoft game launcher\\", "\\ea desktop\\", "\\electronic arts\\", "\\battle.net\\", "\\rockstar games\\", "\\easyanticheat",
        "\\battleye\\", "\\faceit", "\\windows defender", "\\microsoft\\windows defender", "\\nvidia", "\\amd\\", "\\realtek\\", "\\purgeon\\",
        "\\logi", "\\razer", "\\steelseries", "\\corsair",
    ];
    const PARTS: &[&str] = &[
        "anticheat", "anti-cheat", "anti_cheat", "battleye", "beservice", "gameguard", "gamemon", "launcher", "roblox", "purgeon", "audio", "voice",
        "antivirus",
    ];
    let name = stem(path);
    FOLDERS.iter().any(|f| p.contains(f))
        || PARTS.iter().any(|f| name.contains(f))
        || matches!(name.as_str(), "vgc" | "vgk" | "vgtray" | "steam" | "steamwebhelper")
}

/// An ordinary app of the user's: not elevated, not critical to Windows. False when it can't tell.
fn ordinary(pid: u32) -> bool {
    let Some(h) = Handle::open(pid, 0) else { return false };
    unsafe {
        let mut critical = 1;
        if win::IsProcessCritical(h.0, &mut critical) == 0 || critical != 0 {
            return false;
        }
        let mut token = 0isize;
        if win::OpenProcessToken(h.0, 0x8 /* TOKEN_QUERY */, &mut token) == 0 {
            return false;
        }
        let (mut elevated, mut got) = (1u32, 0u32);
        let ok = GetTokenInformation(token, 20 /* TokenElevation */, &mut elevated, 4, &mut got) != 0;
        win::CloseHandle(token);
        ok && elevated == 0
    }
}

struct State {
    on: bool,
    /// The FPS test has it off for one of its rounds
    held: bool,
    closed: bool,
    /// Counts releases, so a sample that began before one can't redo what it undid
    round: u64,
    changes: Vec<Change>,
    game: Vec<u32>,
    /// The picked apps' processes that may be lowered, as of the last sample
    want: Vec<u32>,
    cores: Cores,
    /// What wouldn't change, so it isn't asked again
    skipped: Vec<(Kind, u32, Cores)>,
    picked: Vec<String>,
    seen: Vec<Pick>,
    looked: Option<(Instant, Option<HashSet<u32>>)>,
    load: Option<HashMap<String, f32>>,
    busy_at: Option<HashMap<String, Instant>>,
}

static STATE: Mutex<State> = Mutex::new(State {
    on: false,
    held: false,
    closed: false,
    round: 0,
    changes: Vec::new(),
    game: Vec::new(),
    want: Vec::new(),
    cores: Cores::Waiting,
    skipped: Vec::new(),
    picked: Vec::new(),
    seen: Vec::new(),
    looked: None,
    load: None,
    busy_at: None,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn save_undo(list: &[Change]) {
    if cfg!(test) || crate::shot::on() {
        return;
    }
    let text = encode(list);
    crate::settings::set_text(KEY_UNDO, (!text.is_empty()).then_some(text.as_str()));
}

/// Undoes the changes `which` picks, children included
fn revert(st: &mut State, which: impl Fn(&Change) -> bool) {
    let going: Vec<Change> = st.changes.iter().copied().filter(|c| which(c)).collect();
    if going.is_empty() {
        return;
    }
    let ids = fast().map_or(&[][..], |f| f.ids.as_slice());
    let failed: Vec<Change> = going.iter().copied().filter(|c| !undo(c, ids)).collect();
    // after the parents, so no new child inherits it
    heal_children(&going);
    st.changes.retain(|c| !going.contains(c) || failed.contains(c));
    save_undo(&st.changes);
}

fn change(st: &mut State, kind: Kind, pid: u32) {
    if st.changes.iter().any(|c| c.kind == kind && c.pid == pid) || st.skipped.iter().any(|s| s.0 == kind && s.1 == pid) {
        return;
    }
    let ids = fast().map_or(&[][..], |f| f.ids.as_slice());
    match look(kind, pid, ids) {
        Ok(created) => {
            let c = Change { kind, pid, created, since: filetime_now() };
            st.changes.push(c);
            // written down first
            save_undo(&st.changes);
            if !apply(&c, ids) {
                st.changes.pop();
                save_undo(&st.changes);
                st.skipped.push((kind, pid, Cores::Refused));
            }
        }
        Err(why) => st.skipped.push((kind, pid, why)),
    }
}

/// Makes what is set match what should be
fn settle(st: &mut State) {
    if crate::shot::on() {
        return;
    }
    let active = st.on && !st.held && !st.closed && !st.game.is_empty();
    let (game, want) = (st.game.clone(), st.want.clone());
    revert(st, |c| !active || !(if c.kind == Kind::Cores { &game } else { &want }).contains(&c.pid));
    if !active {
        return;
    }
    if fast().is_some() {
        for &pid in &game {
            change(st, Kind::Cores, pid);
        }
    }
    for &pid in &want {
        change(st, Kind::Priority, pid);
    }
    let has = |pid: &u32| st.changes.iter().any(|c| c.kind == Kind::Cores && c.pid == *pid);
    let why = |k: Cores| st.skipped.iter().any(|s| s.0 == Kind::Cores && game.contains(&s.1) && s.2 == k);
    st.cores = if game.iter().any(has) {
        Cores::Preferred
    } else if why(Cores::Refused) {
        Cores::Refused
    } else if why(Cores::LeftAlone) {
        Cores::LeftAlone
    } else {
        Cores::Waiting
    };
}

/// At start-up: undo what a crashed run left, then pick up the choices
pub fn load() {
    if crate::shot::on() {
        return;
    }
    let mut left = crate::settings::text(KEY_UNDO).map(|t| decode(&t)).unwrap_or_default();
    let found = left.len();
    left.retain(|c| win::process_path(c.pid).is_some_and(|path| may_restore(c.kind, &path)));
    let mut st = state();
    st.on = crate::settings::flag(KEY_ON, false);
    st.picked = crate::settings::text(KEY_APPS).unwrap_or_default().split('|').filter(|p| !p.is_empty()).map(String::from).collect();
    st.changes = left;
    revert(&mut st, |_| true);
    if found > 0 {
        save_undo(&st.changes);
    }
}

pub fn on() -> bool {
    state().on
}

pub fn set_on(on: bool) {
    if crate::shot::on() {
        return;
    }
    let mut st = state();
    st.on = on;
    st.held = false;
    st.skipped.clear();
    if !on {
        st.want.clear();
        st.seen.clear();
        st.cores = Cores::Waiting;
    }
    crate::settings::set_flag(KEY_ON, on);
    settle(&mut st);
}

/// The user's yes or no for one app
pub fn pick(key: &str, yes: bool) {
    if crate::shot::on() {
        return;
    }
    let mut st = state();
    st.picked.retain(|p| p != key);
    if yes {
        st.picked.push(key.to_string());
    }
    let all = st.picked.join("|");
    crate::settings::set_text(KEY_APPS, (!all.is_empty()).then_some(all.as_str()));
    if !yes {
        // straight away; a pick takes effect with the next sample, after the checks
        let pids: Vec<u32> = processes()
            .into_iter()
            .filter(|p| st.want.contains(&p.pid) && win::process_path(p.pid).is_some_and(|path| path.to_lowercase() == key))
            .map(|p| p.pid)
            .collect();
        st.want.retain(|p| !pids.contains(p));
        settle(&mut st);
    }
}

/// For the FPS test: off (`true`) or back on
pub fn hold(off: bool) {
    let mut st = state();
    if st.held != off {
        st.held = off;
        settle(&mut st);
    }
}

/// Puts everything back; the choices stay
pub fn release() {
    let mut st = state();
    st.round += 1;
    st.game.clear();
    st.want.clear();
    st.held = false;
    st.skipped.clear();
    st.cores = Cores::Waiting;
    revert(&mut st, |_| true);
}

pub fn on_quit() {
    state().closed = true;
    release();
}

/// Why a whole app stays at Normal for now
fn in_use(procs: &[&Proc], front: u32, sound: Option<&HashSet<u32>>) -> bool {
    let Some(sound) = sound else { return true };
    procs.iter().any(|p| p.pid == front || sound.contains(&p.pid)) || procs.iter().map(|p| p.gpu).sum::<f32>() >= 2.0
}

/// Taken before the game's processes are looked up, for `step`
pub fn round() -> u64 {
    state().round
}

/// On every sample, also in the tray; `game` is the boosted game's processes as of `round`
pub fn step(s: &Snapshot, game: &[u32], round: u64) {
    sample(&s.procs, s.interval, game, round);
}

fn sample(all: &[Proc], interval: f32, game: &[u32], round: u64) {
    let (on, picked) = {
        let st = state();
        (st.on && !st.closed, st.picked.clone())
    };
    if !on {
        return;
    }
    let last = state().looked.take().filter(|l| l.0.elapsed() < LOOK_EVERY);
    let looking = last.is_none();
    let (looked_at, sound) = last.unwrap_or_else(|| (Instant::now(), audio::active()));
    let front = crate::apps::front_pid();
    // the Boost row keeps the list fresh; this only looks when it never has (started in the tray)
    let found = crate::games::known().or_else(|| crate::games::list(&[], || {}));
    let (session, me) = (my_session(), std::process::id());

    let mut groups: HashMap<String, Vec<&Proc>> = HashMap::new();
    for p in all {
        if let Some(path) = &p.path {
            groups.entry(path.to_lowercase()).or_default().push(p);
        }
    }
    // until the games are known, nothing in the background is touched
    let allowed = |key: &str, procs: &[&Proc]| {
        let Some(games) = found.as_deref() else { return false };
        !off_limits(key)
            && procs.iter().all(|p| p.session == session && p.pid > 4 && p.pid != me && !game.contains(&p.pid))
            && !games.iter().any(|g| g.target.covers(key))
    };

    let now = Instant::now();
    let (mut load, mut busy_at) = {
        let mut st = state();
        (st.load.take().unwrap_or_default(), st.busy_at.take().unwrap_or_default())
    };
    load.retain(|k, _| groups.contains_key(k));
    busy_at.retain(|_, t| now.duration_since(*t) < REST);
    let k = (interval.max(0.25) / 20.0).min(1.0);

    let mut seen: Vec<Pick> = vec![];
    let mut want = vec![];
    for (key, procs) in &groups {
        let cpu: f32 = procs.iter().map(|p| p.cpu).sum();
        let avg = load.entry(key.clone()).or_insert(cpu);
        *avg += (cpu - *avg) * k;
        let chosen = picked.contains(key);
        if (!chosen && *avg < BUSY) || !allowed(key, procs) {
            continue;
        }
        if in_use(procs, front, sound.as_ref()) {
            busy_at.insert(key.clone(), now);
        }
        let using = busy_at.contains_key(key);
        if !chosen && using {
            continue;
        }
        if !procs.iter().all(|p| ordinary(p.pid)) {
            continue;
        }
        if chosen && !using && !game.is_empty() {
            want.extend(procs.iter().map(|p| p.pid));
        }
        let Some(path) = procs[0].path.clone() else { continue };
        let base = procs[0].name.trim_end_matches(".exe").trim_end_matches(".EXE");
        seen.push(Pick {
            key: key.clone(),
            name: crate::platform::friendly_name(base, Some(&path)),
            path,
            cpu: *avg,
            app: if !chosen { App::Offered } else if using { App::InUse } else { App::Waiting },
        });
    }
    // picked ones always show, so each can be undone
    for key in &picked {
        if !seen.iter().any(|p| p.key == *key) {
            seen.push(Pick { key: key.clone(), name: stem(key), path: key.as_str().into(), cpu: 0.0, app: App::Waiting });
        }
    }
    seen.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
    let mut offered = 0;
    seen.retain(|p| {
        offered += (p.app == App::Offered) as usize;
        p.app != App::Offered || offered <= SUGGESTED
    });
    // by name, so rows hold still under the pointer
    seen.sort_by_cached_key(|p| (p.name.to_lowercase(), p.key.clone()));

    let mut st = state();
    st.load = Some(load);
    st.busy_at = Some(busy_at);
    st.looked = Some((looked_at, sound));
    if !st.on || st.closed || st.round != round {
        return;
    }
    // a pick undone while this sample was being worked out
    if st.picked != picked {
        want.clear();
    }
    st.game = game.to_vec();
    st.want = want;
    if game.is_empty() {
        st.held = false;
        st.cores = Cores::Waiting;
    }
    st.skipped.retain(|k| all.iter().any(|p| p.pid == k.1));
    if looking {
        heal_children(&st.changes);
    }
    settle(&mut st);
    for p in seen.iter_mut().filter(|p| p.app == App::Waiting) {
        let pids: Vec<u32> = groups.get(&p.key).map(|g| g.iter().map(|x| x.pid).collect()).unwrap_or_default();
        if st.changes.iter().any(|c| c.kind == Kind::Priority && pids.contains(&c.pid)) {
            p.app = App::Lowered;
        } else if st.skipped.iter().any(|c| c.0 == Kind::Priority && pids.contains(&c.1)) {
            p.app = App::LeftAlone;
        }
    }
    st.seen = seen;
}

pub fn view() -> View {
    if let Some(demo) = demo() {
        return demo;
    }
    let st = state();
    View { on: st.on, cores: st.cores, apps: st.seen.clone() }
}

/// Made-up apps for screenshot runs: PURGEON_STEADY_DEMO=on, off, wait or none
fn demo() -> Option<View> {
    let want = std::env::var("PURGEON_STEADY_DEMO").ok().filter(|_| crate::shot::on())?;
    let pick = |name: &str, cpu: f32, app: App| Pick {
        key: name.to_lowercase(),
        name: name.to_string(),
        path: format!("C:\\Example\\{name}.exe").into(),
        cpu,
        app,
    };
    let waiting = want == "wait";
    Some(View {
        on: want != "off",
        cores: if waiting { Cores::Waiting } else { Cores::Preferred },
        apps: match want.as_str() {
            "none" | "off" => vec![],
            _ => vec![
                pick("Example backup", 3.4, if waiting { App::Waiting } else { App::Lowered }),
                pick("Example sync", 1.8, App::Offered),
                pick("Example updater", 0.9, App::Offered),
            ],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;

    fn cpus(kinds: &[(usize, usize, u8)]) -> Vec<Cpu> {
        let (mut v, mut core) = (vec![], 0u8);
        for &(cores, threads, class) in kinds {
            for _ in 0..cores {
                for _ in 0..threads {
                    v.push(Cpu { id: 256 + v.len() as u32, core: (0, core), class });
                }
                core += 1;
            }
        }
        v
    }

    #[test]
    fn picks_only_the_fastest_kind() {
        let f = fast_of(&cpus(&[(8, 2, 1), (12, 1, 0)])).unwrap();
        assert_eq!((f.ids.len(), f.cores), (16, 8));
        let f = fast_of(&cpus(&[(6, 2, 2), (8, 1, 1), (2, 1, 0)])).unwrap();
        assert_eq!((f.ids.len(), f.cores), (12, 6));
        assert!(fast_of(&cpus(&[(8, 2, 0)])).is_none(), "all alike");
        assert!(fast_of(&cpus(&[(2, 2, 1), (8, 1, 0)])).is_none(), "too few fast ones");
        assert!(fast_of(&[]).is_none());
    }

    #[test]
    fn undo_list_survives_a_restart() {
        let list = vec![
            Change { kind: Kind::Cores, pid: 4120, created: 134_000_000_000_000_000, since: 7 },
            Change { kind: Kind::Priority, pid: 88, created: 9, since: 10 },
        ];
        assert_eq!(decode(&encode(&list)), list);
        assert!(decode("").is_empty() && decode("x:88:1:2,p:4:1:2,p:no:1:2,c:88:1").is_empty());
    }

    #[test]
    fn never_lists_the_protected_ones() {
        let windows = std::env::var("SystemRoot").unwrap();
        for p in [
            format!("{windows}\\System32\\svchost.exe"),
            "D:\\Apps\\chrome.exe".into(),
            "D:\\Apps\\Discord.exe".into(),
            "D:\\Apps\\Spotify.exe".into(),
            "D:\\Apps\\pwsh.exe".into(),
            "D:\\SteamLibrary\\steamapps\\common\\Game\\game.exe".into(),
            "C:\\Program Files\\Riot Vanguard\\vgtray.exe".into(),
            "C:\\Users\\a\\AppData\\Local\\Roblox\\Versions\\v\\RobloxPlayerBeta.exe".into(),
            "C:\\Program Files (x86)\\EasyAntiCheat\\EasyAntiCheat.exe".into(),
            "D:\\Tools\\Purgeon.exe".into(),
        ] {
            assert!(off_limits(&p), "{p}");
        }
        assert!(!off_limits("D:\\Apps\\Backup\\backup.exe"));
    }

    #[test]
    fn only_inherited_children_are_put_back() {
        let parent = [Change { kind: Kind::Priority, pid: 100, created: 5, since: 50 }];
        let p = |pid, parent, created, base| Listed { pid, parent, created, session: 1, base };
        let list = [p(100, 1, 5, 6), p(200, 100, 60, 6), p(300, 100, 40, 6), p(400, 100, 70, 8), p(500, 9, 80, 6)];
        let got: Vec<u32> = inherited(&parent, &list).iter().map(|p| p.pid).collect();
        assert_eq!(got, [200]);
    }

    #[test]
    fn leaves_windows_games_and_anti_cheat_alone() {
        let windows = std::env::var("SystemRoot").unwrap();
        let games = [crate::games::Game {
            name: "Example".into(),
            target: crate::games::Target { exe: "game.exe".into(), dirs: vec!["d:\\play\\example".into()] },
            icon: None,
        }];
        let system = [
            format!("{windows}\\System32\\cmd.exe"),
            format!("{windows}\\System32\\conhost.exe"),
            format!("{windows}\\explorer.exe"),
            "C:\\Program Files\\Riot Vanguard\\vgtray.exe".to_string(),
        ];
        for p in &system {
            assert!(!may_heal(p, &games) && !may_restore(Kind::Priority, p), "{p}");
        }
        // a helper too, which `Target::matches` doesn't count as the game
        for p in ["D:\\Play\\Example\\game.exe", "D:\\Play\\Example\\game_be.exe"] {
            assert!(!may_heal(p, &games), "{p}");
        }
        assert!(may_heal("D:\\Apps\\pwsh.exe", &games) && may_heal("D:\\Apps\\Backup\\backup.exe", &games));
        assert!(!may_restore(Kind::Cores, &format!("{windows}\\System32\\lsass.exe")));
        assert!(may_restore(Kind::Cores, "C:\\Users\\a\\AppData\\Local\\Roblox\\Versions\\v\\RobloxPlayerBeta.exe"));
    }

    #[test]
    fn a_reused_parent_id_is_not_a_parent() {
        let parent = [Change { kind: Kind::Priority, pid: 100, created: 5, since: 50 }];
        let p = |pid, parent, created, base| Listed { pid, parent, created, session: 1, base };
        let list = [p(100, 1, 70, 8), p(200, 100, 60, 6), p(300, 100, 80, 6)];
        let got: Vec<u32> = inherited(&parent, &list).iter().map(|p| p.pid).collect();
        assert_eq!(got, [200]);
    }

    /// Stop landing between a sample's start and its end, on this test's own child
    #[test]
    fn a_stop_during_a_sample_wins() {
        let Some(fast) = fast() else { return };
        let mut child = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .creation_flags(0x0800_0000 | NORMAL)
            .spawn()
            .unwrap();
        let sets = || Handle::open(child.id(), 0).and_then(|h| h.cpu_sets());
        state().on = true;

        let began = round();
        release(); // Stop
        sample(&[], 1.0, &[child.id()], began);
        let stale = sets();
        sample(&[], 1.0, &[child.id()], round());
        let current = sets();
        release();
        let after = sets();

        state().on = false;
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!((stale, current, after), (Some(vec![]), Some(fast.ids.clone()), Some(vec![])));
    }

    /// Both changes and their undo, on a sleeping process this test starts and nothing else
    #[test]
    fn changes_and_undoes_its_own_child() {
        let mut child = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .creation_flags(0x0800_0000 | NORMAL) // no window, Normal priority
            .spawn()
            .unwrap();
        let pid = child.id();
        let ids = fast_of(&read_cpus()).map(|f| f.ids).unwrap_or_default();
        let run = || -> Result<(), String> {
            let created = look(Kind::Priority, pid, &ids).map_err(|e| format!("look {e:?}"))?;
            let p = Change { kind: Kind::Priority, pid, created, since: filetime_now() };
            let stale = Change { created: created + 1, ..p };
            if apply(&stale, &ids) {
                return Err("changed a reused id".into());
            }
            if !apply(&p, &ids) || Handle::open(pid, 0).unwrap().class() != BELOW_NORMAL {
                return Err("not lowered".into());
            }
            if !undo(&p, &ids) || Handle::open(pid, 0).unwrap().class() != NORMAL {
                return Err("priority not put back".into());
            }
            if ids.is_empty() {
                return Ok(());
            }
            let c = Change { kind: Kind::Cores, ..p };
            if !apply(&c, &ids) || Handle::open(pid, 0).unwrap().cpu_sets() != Some(ids.clone()) {
                return Err("cores not set".into());
            }
            if look(Kind::Cores, pid, &ids) != Err(Cores::LeftAlone) {
                return Err("would overwrite someone's choice".into());
            }
            if !undo(&c, &ids) || Handle::open(pid, 0).unwrap().cpu_sets() != Some(vec![]) {
                return Err("cores not put back".into());
            }
            Ok(())
        };
        let result = run();
        child.kill().unwrap();
        child.wait().unwrap();
        println!("{} fast threads here", ids.len());
        result.unwrap();
    }

    /// A child of a lowered parent really inherits it, and is found and put back
    #[test]
    fn finds_a_child_that_inherited() {
        // a hidden shell that waits, then starts ping
        let mut shell = std::process::Command::new("cmd")
            .args(["/d", "/c", "ping -n 3 127.0.0.1 >nul & ping -n 9 127.0.0.1 >nul"])
            .creation_flags(0x0800_0000 | NORMAL)
            .spawn()
            .unwrap();
        let pid = shell.id();
        let created = look(Kind::Priority, pid, &[]).unwrap();
        let c = Change { kind: Kind::Priority, pid, created, since: filetime_now() };
        let lowered = apply(&c, &[]);
        std::thread::sleep(Duration::from_secs(4));
        undo(&c, &[]);
        let list = processes();
        let kids: Vec<(u32, u64)> = inherited(&[c], &list).iter().map(|p| (p.pid, p.created)).collect();
        let mut back = !kids.is_empty();
        for &(pid, created) in &kids {
            back &= undo(&Change { pid, created, ..c }, &[]);
            back &= Handle::open(pid, 0).map_or(true, |h| h.class() == NORMAL);
        }
        shell.wait().unwrap();
        assert!(lowered && back, "{kids:?}");
    }
}
