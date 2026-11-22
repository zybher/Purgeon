//! The ping counter: how long a packet takes to reach the server the app in front talks to.
//!
//! Which server: the address the app sends the most to (from Windows' own network events, see
//! `etw::net_flows`). How long: plain ICMP echo, like the `ping` command, one small packet a
//! second while the bar is open, sent by Purgeon itself. Nothing touches the game.
//!
//! Many game servers (Roblox, Riot, Blizzard) don't answer pings. Then Purgeon finds the last
//! router on the way that does, the entry of the game company's own network, and pings that
//! instead: the reading is then marked as an estimate ("~"). Checked against Roblox's own
//! NetworkPing: 21 ms here vs 20 ms there. When that router isn't on the game's own network
//! (cloud hosts hide the whole path) there is no honest number, so it shows a dash.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[link(name = "iphlpapi")]
extern "system" {
    fn IcmpCreateFile() -> isize;
    fn IcmpSendEcho(h: isize, dest: u32, data: *const u8, size: u16, opts: *const IpOptions, reply: *mut u8, reply_size: u32, timeout: u32) -> u32;
}

/// IP_OPTION_INFORMATION
#[repr(C)]
struct IpOptions {
    ttl: u8,
    tos: u8,
    flags: u8,
    size: u8,
    data: *const u8,
}

const IP_SUCCESS: u32 = 0;
const IP_TTL_EXPIRED_TRANSIT: u32 = 11013;

/// What the PING cell shows
#[derive(Clone, Copy, PartialEq)]
pub enum Ping {
    /// The app sends nothing to the internet (or Windows won't say, when not elevated)
    None,
    /// Found the server, still working out how to time it
    Finding,
    /// Milliseconds; `true` when timed to the game network's entry router, not the server
    Ms(f32, bool),
    /// The server and every router near it stay silent
    Hidden,
}

struct State {
    want: Option<[u8; 4]>,
    asked: Option<Instant>,
    /// for `want`: the result so far
    shown: Ping,
    /// another address that out-sends `want`, and since when (a switch waits a moment)
    lead: Option<([u8; 4], Instant)>,
    /// when `want` was last among the addresses sent to
    busy: Option<Instant>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { want: None, asked: None, shown: Ping::None, lead: None, busy: None }))
}

/// The ping to whatever these processes talk to. Called while the bar is open; the pinging
/// stops a few seconds after it isn't called any more.
pub fn reading(pids: &[u32]) -> Ping {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let _ = std::thread::Builder::new().name("purgeon-ping".into()).spawn(worker);
    });
    let flows = crate::sys::etw::net_flows(pids);
    let Ok(mut s) = state().lock() else { return Ping::None };
    s.asked = Some(Instant::now());
    let Some(best) = flows.first().map(|x| (x.0, x.3)) else {
        s.lead = None;
        // a quiet moment (a loading screen, a hitch): keep the reading a few seconds
        return if s.busy.is_some_and(|b| b.elapsed() < Duration::from_secs(5)) { s.shown } else { Ping::None };
    };
    if s.want.is_some_and(|w| flows.iter().any(|x| x.0 == w)) {
        s.busy = Some(Instant::now());
    }
    // Stay with the server while it is still one of the busy ones: games also send bursts
    // elsewhere now and then (Roblox probes 25 relay sites every few minutes), and hopping
    // over to those for a second would show their ping and restart the search each time.
    let kept = s.want.and_then(|w| flows.iter().find(|x| x.0 == w)).is_some_and(|x| x.3 * 2.0 >= best.1);
    if !kept && s.want != Some(best.0) {
        let switch = s.want.is_none() || s.lead.is_some_and(|(ip, at)| ip == best.0 && at.elapsed() >= Duration::from_secs(3));
        if switch {
            s.want = Some(best.0);
            s.shown = Ping::Finding;
            s.lead = None;
            s.busy = Some(Instant::now());
            let list: Vec<String> = flows.iter().take(6).map(|x| format!("{} {}:{} score {:.0}", if x.1 { "udp" } else { "tcp" }, ip_str(x.0), x.2, x.3)).collect();
            log(&format!("server {} (of: {})", ip_str(best.0), list.join(", ")));
        } else if s.lead.map(|l| l.0) != Some(best.0) {
            s.lead = Some((best.0, Instant::now()));
        }
    } else {
        s.lead = None;
    }
    s.shown
}

fn ip_str(ip: [u8; 4]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

/// A short record of what the ping counter picked, in %APPDATA%\Purgeon\ping-log.txt, so a
/// wrong number can be traced back (which address, timed how)
fn log(line: &str) {
    use std::io::Write;
    let Some(dir) = std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("Purgeon")) else { return };
    let path = dir.join("ping-log.txt");
    let fresh = std::fs::metadata(&path).map_or(true, |m| m.len() > 200_000);
    let _ = std::fs::create_dir_all(&dir);
    let f = std::fs::OpenOptions::new().create(true).write(true).append(!fresh).truncate(fresh).open(&path);
    if let Ok(mut f) = f {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let _ = writeln!(f, "{secs} {line}");
    }
}

/// One echo to `ip`; `ttl` 0 is the default. The reply's sender, status and round trip in ms.
fn echo(h: isize, ip: [u8; 4], ttl: u8, timeout_ms: u32) -> Option<([u8; 4], u32, f32)> {
    let data = [0x50u8; 32];
    let opts = IpOptions { ttl, tos: 0, flags: 0, size: 0, data: std::ptr::null() };
    let mut reply = [0u8; 256];
    let t = Instant::now();
    let n = unsafe {
        IcmpSendEcho(
            h,
            u32::from_ne_bytes(ip),
            data.as_ptr(),
            data.len() as u16,
            if ttl > 0 { &opts } else { std::ptr::null() },
            reply.as_mut_ptr(),
            reply.len() as u32,
            timeout_ms,
        )
    };
    let ms = t.elapsed().as_secs_f32() * 1000.0;
    if n == 0 {
        return None;
    }
    let from = [reply[0], reply[1], reply[2], reply[3]];
    let status = u32::from_ne_bytes([reply[4], reply[5], reply[6], reply[7]]);
    Some((from, status, ms))
}

/// The last router on the way to `ip` that answers, if it sits on the same network as `ip`
/// (the same /16: the game company's own entry). At most one probe per hop, one at a time.
fn entry_router(h: isize, ip: [u8; 4]) -> Option<[u8; 4]> {
    let mut last = None;
    let mut silent = 0;
    for ttl in 1..=24u8 {
        match echo(h, ip, ttl, 700) {
            Some((from, IP_TTL_EXPIRED_TRANSIT, _)) => {
                last = Some(from);
                silent = 0;
            }
            Some((_, IP_SUCCESS, _)) => return Some(ip),
            _ => {
                silent += 1;
                if silent >= 4 && last.is_some() {
                    break;
                }
            }
        }
    }
    last.filter(|r| r[..2] == ip[..2])
}

fn publish(target: [u8; 4], p: Ping) {
    if let Ok(mut s) = state().lock() {
        if s.want == Some(target) {
            s.shown = p;
        }
    }
}

fn worker() {
    let h = unsafe { IcmpCreateFile() };
    if h == -1 || h == 0 {
        return;
    }
    // what we are timing: (server, address actually pinged, is it an estimate)
    let mut cur: Option<([u8; 4], [u8; 4], bool)> = None;
    let mut samples: VecDeque<f32> = VecDeque::new();
    let mut misses = 0u32;
    let mut logged = 0u32;
    let mut hidden_since: Option<([u8; 4], Instant)> = None;
    // how each server was timed before, so coming back to a game doesn't search again
    let mut known: std::collections::HashMap<[u8; 4], [u8; 4]> = std::collections::HashMap::new();
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let (want, active) = match state().lock() {
            Ok(s) => (s.want, s.asked.is_some_and(|a| a.elapsed() < Duration::from_secs(3))),
            Err(_) => return,
        };
        let Some(target) = want.filter(|_| active) else { continue };
        if cur.map(|c| c.0) != Some(target) {
            // a server that stayed silent: try again after a minute, not every second
            if hidden_since.is_some_and(|(ip, at)| ip == target && at.elapsed() < Duration::from_secs(60)) {
                continue;
            }
            samples.clear();
            misses = 0;
            if let Some(&to) = known.get(&target) {
                cur = Some((target, to, to != target));
                continue;
            }
            // the server itself first, three tries
            let direct = (0..3).find_map(|_| echo(h, target, 0, 1000).filter(|r| r.1 == IP_SUCCESS));
            if let Some((_, _, ms)) = direct {
                known.insert(target, target);
                cur = Some((target, target, false));
                samples.push_back(ms);
                log(&format!("  {} answers pings: {ms:.1} ms", ip_str(target)));
            } else if let Some(r) = entry_router(h, target) {
                cur = Some((target, r, r != target));
                known.insert(target, r);
                log(&format!("  {} is silent; timing its network's entry router {}", ip_str(target), ip_str(r)));
            } else {
                cur = None;
                hidden_since = Some((target, Instant::now()));
                log(&format!("  {} is silent and so is its network: no number", ip_str(target)));
                publish(target, Ping::Hidden);
                continue;
            }
        } else {
            let (_, to, _) = cur.expect("checked above");
            match echo(h, to, 0, 1000).filter(|r| r.1 == IP_SUCCESS) {
                Some((_, _, ms)) => {
                    samples.push_back(ms);
                    misses = 0;
                    logged += 1;
                    if logged % 15 == 1 {
                        log(&format!("  {} -> {ms:.1} ms", ip_str(to)));
                    }
                }
                None => misses += 1,
            }
        }
        let (_, _, est) = cur.expect("set above");
        while samples.len() > 5 {
            samples.pop_front();
        }
        if misses >= 10 {
            // the route changed: look again
            cur = None;
            known.remove(&target);
            publish(target, Ping::Finding);
            continue;
        }
        if misses >= 5 || samples.is_empty() {
            publish(target, Ping::Finding);
            continue;
        }
        // A server: the median of the last few, so one late packet doesn't make it jump.
        // A router: the fastest of the last few. Routers answer pings on their slow side
        // when busy (here every other reply came back 40 ms late), which is delay in the
        // router, not on the way; the game's packets pass straight through it.
        let mut v: Vec<f32> = samples.iter().copied().collect();
        v.sort_by(|a, b| a.total_cmp(b));
        publish(target, Ping::Ms(if est { v[0] } else { v[v.len() / 2] }, est));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sends real packets (a short trace and three pings), so only on request:
    /// `cargo test entry_router -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn entry_router_roblox() {
        let h = unsafe { IcmpCreateFile() };
        let server = [128, 116, 54, 33];
        let r = entry_router(h, server);
        println!("entry router: {r:?}");
        let r = r.expect("found one");
        for _ in 0..3 {
            println!("{:?}", echo(h, r, 0, 1000));
        }
    }
}
