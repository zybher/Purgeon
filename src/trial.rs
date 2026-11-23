//! The FPS test behind Boost's before and after. It only runs when the user starts it.
//!
//! A single before and after is mostly noise, because a game's scenes change. So Boost is switched
//! off and on over 24 rounds in the order off, on, on, off: each pair of neighbouring rounds is
//! one comparison, and the order cancels a scene that slowly gets lighter or heavier. A change
//! only counts when it is clearly bigger than the swing between those pairs.
//!
//! With "Steadier frames" on, Boost stays on and only that part switches.
//!
//! Drops are measured two ways: the 1% low, and the lowest FPS held over a fifth of a second in
//! each round, whatever caused it (an explosion, getting shot or flung, anything). Dips come at
//! random and not every round, so only the rounds that had one say how deep they are.
//!
//! Frames come from the ETW counter (`sys::etw`), so nothing here opens or reads the game.

use crate::boost;
use crate::games::Target;
use crate::sys::etw;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const ROUNDS: usize = 24;
/// Seconds left out after each switch, and seconds counted
const SKIP: f64 = 3.0;
const MEASURE: f64 = 10.0;
/// What a whole test takes when nothing pauses it
pub const SECS: f64 = ROUNDS as f64 * (SKIP + MEASURE);
/// The t value for twelve pairs at 98.3%: three numbers are judged, so together that is about 95%
const T: f32 = 2.82;
/// How long FPS has to stay down to be a dip, in seconds
const DIP: f64 = 0.2;
/// A round dipped when it fell below this share of the whole test's average
const DIPPED: f32 = 0.8;
/// The same t value for fewer samples, from 2 degrees of freedom up to 11
const T_FEW: [f32; 10] = [7.65, 4.86, 3.96, 3.53, 3.29, 3.13, 3.02, 2.93, 2.87, T];
/// Smaller changes can't be felt, however steady they are
const MIN_CHANGE: f32 = 2.0;
const GIVE_UP: Duration = Duration::from_secs(20 * 60);

/// Whether Boost is on in this round
fn on_in(round: usize) -> bool {
    matches!(round % 4, 1 | 2)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Better,
    Worse,
    Same,
}

/// One number without Boost and with it
#[derive(Clone, Copy, Debug)]
pub struct Change {
    pub off: f32,
    pub on: f32,
    pub pct: f32,
    /// How far, in percent, the result could be out by chance
    pub noise: f32,
    pub verdict: Verdict,
}

#[derive(Clone, Copy, Debug)]
pub struct Report {
    pub avg: Change,
    pub low1: Change,
    pub dip: Change,
    /// It compared Boost alone against Boost with "Steadier frames"
    pub steady: bool,
    /// Both sat at this common frame cap, so a gain had no room to show
    pub capped: Option<u32>,
}

#[derive(Clone, Copy)]
pub enum View {
    Idle,
    /// `waiting`: the game isn't in front or isn't drawing, so nothing is counted
    Running { round: usize, waiting: bool, left: f32 },
    Done(Report),
    Failed(&'static str),
}

/// One round's average, 1% low and deepest dip
type Round = (f32, f32, f32);

fn change(rounds: &[Round], get: fn(&Round) -> f32) -> Change {
    let (mut off, mut on) = (Vec::new(), Vec::new());
    for (i, r) in rounds.iter().enumerate() {
        if on_in(i) { on.push(get(r)) } else { off.push(get(r)) }
    }
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let (off, on) = (mean(&off), mean(&on));
    let base = off.max(0.01);
    let diffs: Vec<f32> = rounds
        .chunks_exact(2)
        .enumerate()
        .map(|(i, pair)| {
            let d = get(&pair[1]) - get(&pair[0]);
            (if on_in(2 * i + 1) { d } else { -d }) / base * 100.0
        })
        .collect();
    let pct = mean(&diffs);
    let n = diffs.len() as f32;
    let var = diffs.iter().map(|d| (d - pct).powi(2)).sum::<f32>() / (n - 1.0).max(1.0);
    let noise = T * (var / n.max(1.0)).sqrt();
    Change { off, on, pct, noise, verdict: judge(pct, noise) }
}

fn judge(pct: f32, noise: f32) -> Verdict {
    match pct {
        p if p.abs() < MIN_CHANGE || p.abs() <= noise => Verdict::Same,
        p if p > 0.0 => Verdict::Better,
        _ => Verdict::Worse,
    }
}

/// How deep the dips were, over the rounds that had one. With fewer than three such rounds on
/// either side, every round counts, pair by pair.
fn depth(rounds: &[Round]) -> Change {
    let limit = DIPPED * rounds.iter().map(|r| r.0).sum::<f32>() / rounds.len().max(1) as f32;
    let (mut off, mut on) = (Vec::new(), Vec::new());
    for (i, r) in rounds.iter().enumerate() {
        if r.2 < limit {
            if on_in(i) { on.push(r.2) } else { off.push(r.2) }
        }
    }
    let least = off.len().min(on.len());
    if least < 3 {
        return change(rounds, |r| r.2);
    }
    let stats = |v: &[f32]| {
        let n = v.len() as f32;
        let mean = v.iter().sum::<f32>() / n;
        (mean, v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / (n - 1.0) / n)
    };
    let ((a, va), (b, vb)) = (stats(&off), stats(&on));
    let base = a.max(0.01);
    let (pct, noise) = ((b - a) / base * 100.0, T_FEW[(least - 3).min(T_FEW.len() - 1)] * (va + vb).sqrt() / base * 100.0);
    Change { off: a, on: b, pct, noise, verdict: judge(pct, noise) }
}

pub fn compare(rounds: &[Round]) -> Report {
    let (avg, low1, dip) = (change(rounds, |r| r.0), change(rounds, |r| r.1), depth(rounds));
    let near = |v: f32, cap: u32| (v / cap as f32 - 1.0).abs() < 0.01;
    let capped = [30, 60, 75, 120, 144, 165, 240].into_iter().find(|&c| avg.verdict == Verdict::Same && near(avg.off, c) && near(avg.on, c));
    Report { avg, low1, dip, steady: false, capped }
}

/// A round's numbers from its frame times, or None if it didn't draw all the way through
fn measure(times: &mut [i64], from: i64, to: i64, freq: f64) -> Option<Round> {
    times.sort_unstable();
    let t: Vec<i64> = times.iter().copied().filter(|&t| t > from && t <= to).collect();
    if t.len() < 101 {
        return None;
    }
    let limit = freq as i64; // a second without a frame is a pause or a loading screen
    if t[0] - from > limit || to - t[t.len() - 1] > limit {
        return None;
    }
    let mut gaps: Vec<i64> = t.windows(2).map(|w| w[1] - w[0]).collect();
    if gaps.iter().any(|&g| g > limit) {
        return None;
    }
    let total = (t[t.len() - 1] - t[0]) as f64;
    let avg = gaps.len() as f64 * freq / total.max(1.0);
    let i = (gaps.len() * 99 / 100).min(gaps.len() - 1);
    let worst = *gaps.select_nth_unstable(i).1;
    Some((avg as f32, (freq / worst.max(1) as f64) as f32, deepest(&t, freq)))
}

/// The lowest FPS over any stretch at least `DIP` long
fn deepest(t: &[i64], freq: f64) -> f32 {
    let span = (DIP * freq) as i64;
    let (mut a, mut low) = (0, f64::MAX);
    for b in 0..t.len() {
        while a + 1 < b && t[b] - t[a + 1] >= span {
            a += 1;
        }
        if t[b] - t[a] >= span {
            low = low.min((b - a) as f64 * freq / (t[b] - t[a]) as f64);
        }
    }
    if low == f64::MAX { 0.0 } else { low as f32 }
}

/// The test's steps, apart from Windows so they can be tested with made-up frames
struct Run {
    freq: f64,
    rounds: Vec<Round>,
    /// The round under way: when it counts from and to
    span: Option<(i64, i64)>,
    times: Vec<i64>,
    /// Frames up to here are collected
    mark: i64,
    /// What Boost should be right now
    boosted: bool,
}

impl Run {
    fn new(freq: f64) -> Run {
        Run { freq, rounds: Vec::new(), span: None, times: Vec::new(), mark: 0, boosted: true }
    }

    fn begin(&mut self, now: i64) {
        let from = now + (SKIP * self.freq) as i64;
        self.span = Some((from, from + (MEASURE * self.freq) as i64));
        self.times.clear();
        self.mark = from;
        self.boosted = on_in(self.rounds.len());
    }

    fn pause(&mut self) {
        self.span = None;
        self.times.clear();
        self.boosted = true;
    }

    /// `now`: the clock; `settled`: frames up to here are sure to have arrived
    fn tick(&mut self, now: i64, settled: i64, frames: impl FnOnce(i64, i64) -> Vec<i64>) -> Option<Report> {
        let Some((from, to)) = self.span else {
            self.begin(now);
            return None;
        };
        let until = settled.min(to);
        if until > self.mark {
            self.times.extend(frames(self.mark, until));
            self.mark = until;
        }
        if settled < to {
            return None;
        }
        if let Some(r) = measure(&mut self.times, from, to, self.freq) {
            self.rounds.push(r);
            if self.rounds.len() == ROUNDS {
                self.pause();
                return Some(compare(&self.rounds));
            }
        }
        // the next round, or this one again if it had a pause in it
        self.begin(now);
        None
    }

    fn left(&self, now: i64) -> f32 {
        let this = self.span.map_or(SKIP + MEASURE, |s| ((s.1 - now) as f64 / self.freq).max(0.0));
        ((ROUNDS - self.rounds.len()).saturating_sub(1) as f64 * (SKIP + MEASURE) + this) as f32
    }
}

struct State {
    /// Goes up with every start and cancel, so an old thread knows to leave
    id: u32,
    target: Option<Target>,
    view: View,
}

static STATE: Mutex<State> = Mutex::new(State { id: 0, target: None, view: View::Idle });

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn view(target: &Target) -> View {
    let st = state();
    if st.target.as_ref() == Some(target) { st.view } else { View::Idle }
}

pub fn start(target: &Target) {
    if crate::shot::on() {
        return;
    }
    let id = {
        let mut st = state();
        st.id += 1;
        st.target = Some(target.clone());
        st.view = View::Running { round: 0, waiting: true, left: SECS as f32 };
        st.id
    };
    let target = target.clone();
    if std::thread::Builder::new().name("purgeon-boost-test".into()).spawn(move || work(id, &target)).is_err() {
        end(id, View::Failed("The test couldn't start"));
    }
}

/// Ends the test and leaves Boost on
pub fn cancel() {
    let mut st = state();
    st.id += 1;
    if matches!(st.view, View::Running { .. }) {
        st.view = View::Idle;
        boost::hold(false);
    }
}

fn end(id: u32, view: View) {
    let told = {
        let mut st = state();
        if st.id != id {
            return;
        }
        st.view = view;
        boost::hold(false);
        matches!(view, View::Done(_) | View::Failed(_))
    };
    if told {
        let body = match view {
            View::Failed(why) => why,
            _ => "The before and after is on the Dashboard. Boost is on again.",
        };
        crate::tray::notify("Boost test finished", body, false);
    }
}

fn work(id: u32, target: &Target) {
    if etw::frames() != etw::Frames::Counting {
        return end(id, View::Failed("Windows only lets administrators count frames"));
    }
    let started = Instant::now();
    let steady = crate::steady::on();
    let mut run = Run::new(etw::frame_freq() as f64);
    let mut source: Option<(u32, u8)> = None;
    loop {
        std::thread::sleep(Duration::from_millis(250));
        // a different pick, or the experiment switched: what's measured so far no longer compares
        if !boost::status().is_some_and(|b| b.target == *target) || crate::steady::on() != steady {
            return end(id, View::Idle);
        }
        let pids = boost::pids();
        if pids.is_empty() || source.is_some_and(|s| !pids.contains(&s.0)) {
            return end(id, View::Failed("The game closed before the test finished"));
        }
        if started.elapsed() > GIVE_UP {
            return end(id, View::Failed("The test was paused too long"));
        }
        let front = crate::apps::front_pid();
        let (now, settled) = etw::frame_clock();
        if pids.contains(&front) {
            // until the first round counts, whichever of its processes draws the most
            if run.rounds.is_empty() && run.span.is_none() {
                source = etw::frame_source(&pids);
            }
        }
        let waiting = !pids.contains(&front) || source.is_none();
        let report = match source {
            Some(src) if !waiting => run.tick(now, settled, |after, until| etw::frame_times(src, after, until)),
            _ => {
                run.pause();
                None
            }
        };
        if let Some(r) = report {
            return end(id, View::Done(Report { steady, ..r }));
        }
        let mut st = state();
        if st.id != id {
            return;
        }
        // under the lock, so a cancel can't be followed by a switch to off
        boost::hold(!run.boosted);
        st.view = View::Running { round: run.rounds.len(), waiting, left: run.left(now) };
    }
}

/// Made-up results for screenshot runs: PURGEON_TRIAL_DEMO=idle, run, wait, same, capped, up or
/// steady
pub fn demo() -> Option<View> {
    let want = std::env::var("PURGEON_TRIAL_DEMO").ok().filter(|_| crate::shot::on())?;
    let rounds = |base: f32, gain: f32| -> Vec<Round> {
        (0..ROUNDS)
            .map(|i| {
                let wobble = [0.2, -0.5, 0.6, -0.1, -0.4, 0.3, 0.5, -0.6, 0.1, 0.4, -0.3, -0.2][i % 12];
                let k = if on_in(i) { 1.0 + gain } else { 1.0 };
                (base * k + wobble, base * 0.72 * k + wobble * 2.0, base * 0.55 * k + wobble * 3.0)
            })
            .collect()
    };
    Some(match want.as_str() {
        "run" => View::Running { round: 8, waiting: false, left: 202.0 },
        "wait" => View::Running { round: 8, waiting: true, left: 208.0 },
        "same" => View::Done(compare(&rounds(132.0, 0.004))),
        "capped" => View::Done(compare(&rounds(60.0, 0.0))),
        "up" => View::Done(compare(&rounds(132.0, 0.07))),
        "steady" => View::Done(Report { steady: true, ..compare(&rounds(170.0, 0.001)) }),
        _ => View::Idle,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: f64 = 10_000_000.0;

    struct Rng(u64);
    impl Rng {
        fn unit(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }
        fn normal(&mut self) -> f32 {
            (-2.0 * self.unit().max(1e-6).ln()).sqrt() * (std::f32::consts::TAU * self.unit()).cos()
        }
    }

    /// Plays a whole test on a clock of its own, `fps(boosted, seconds)` being the game
    fn play(fps: impl Fn(bool, f64) -> f64, away: std::ops::Range<f64>) -> (Report, f64) {
        let mut run = Run::new(F);
        let mut frames: Vec<i64> = Vec::new();
        let (mut now, mut drawn) = (0.0f64, 0.0f64);
        loop {
            now += 0.25;
            while drawn <= now {
                drawn += 1.0 / fps(run.boosted, drawn);
                if !away.contains(&drawn) {
                    frames.push((drawn * F) as i64);
                }
            }
            if away.contains(&now) {
                run.pause();
                assert!(run.boosted, "Boost stays on while paused");
                continue;
            }
            let got = run.tick((now * F) as i64, ((now - 0.45) * F) as i64, |a, b| {
                frames[frames.partition_point(|&t| t <= a)..].iter().copied().take_while(|&t| t <= b).collect()
            });
            if let Some(r) = got {
                assert!(run.boosted, "Boost is on again at the end");
                return (r, now);
            }
            assert!(now < 700.0, "never finished");
        }
    }

    #[test]
    fn order_is_balanced() {
        assert_eq!((0..ROUNDS).filter(|&i| on_in(i)).count(), ROUNDS / 2);
        let on_first = (0..ROUNDS).step_by(2).filter(|&i| on_in(i)).count();
        assert_eq!(on_first, ROUNDS / 4, "half the pairs start with Boost on");
    }

    #[test]
    fn finds_a_real_gain() {
        let (r, secs) = play(|on, _| if on { 154.0 } else { 140.0 }, 0.0..0.0);
        assert!((r.avg.off - 140.0).abs() < 0.2 && (r.avg.on - 154.0).abs() < 0.2, "{r:?}");
        assert_eq!((r.avg.verdict, r.low1.verdict), (Verdict::Better, Verdict::Better));
        // each round ends up to half a second late, waiting for its last frames to arrive
        assert!(secs < SECS + ROUNDS as f64 * 0.75, "{secs}");
    }

    #[test]
    fn finds_a_loss() {
        let (r, _) = play(|on, _| if on { 90.0 } else { 100.0 }, 0.0..0.0);
        assert_eq!(r.avg.verdict, Verdict::Worse);
    }

    #[test]
    fn a_drifting_scene_is_not_a_gain() {
        let (r, _) = play(|_, t| 100.0 + t * 0.2, 0.0..0.0);
        assert_eq!((r.avg.verdict, r.low1.verdict), (Verdict::Same, Verdict::Same), "{r:?}");
    }

    #[test]
    fn a_cap_is_named() {
        let (r, _) = play(|_, _| 60.0, 0.0..0.0);
        assert_eq!((r.avg.verdict, r.capped), (Verdict::Same, Some(60)));
    }

    #[test]
    fn looking_away_repeats_the_round() {
        let (r, secs) = play(|on, _| if on { 120.0 } else { 100.0 }, 30.0..50.0);
        assert!(secs > SECS + 18.0, "{secs}");
        assert!((r.avg.off - 100.0).abs() < 0.2 && (r.avg.on - 120.0).abs() < 0.2, "{r:?}");
    }

    #[test]
    fn one_percent_low_sees_stutters() {
        let mut t = 0i64;
        let mut times: Vec<i64> = (0..1200)
            .map(|i| {
                t += if i % 50 == 49 { 250_000 } else { 80_000 };
                t
            })
            .collect();
        let (avg, low, _) = measure(&mut times, 0, t, F).unwrap();
        assert!((low - 40.0).abs() < 0.1 && avg > 115.0 && avg < 125.0, "{avg} {low}");
        times.retain(|&x| !(30_000_000..45_000_000).contains(&x));
        assert!(measure(&mut times, 0, t, F).is_none(), "a pause spoils the round");
    }

    #[test]
    fn deepest_dip_is_found() {
        // ten seconds at 200 FPS, but 80 for half a second
        let mut t = vec![0i64];
        while *t.last().unwrap() < 100_000_000 {
            let last = *t.last().unwrap();
            t.push(last + if (40_000_000..45_000_000).contains(&last) { 125_000 } else { 50_000 });
        }
        let low = deepest(&t, F);
        assert!((low - 80.0).abs() < 1.0, "{low}");
        let flat: Vec<i64> = (0..2000).map(|i| i * 50_000).collect();
        assert!((deepest(&flat, F) - 200.0).abs() < 1.0);
    }

    #[test]
    fn shallower_dips_count_without_a_higher_average() {
        let (r, _) = play(|on, t| if t % 5.0 < 0.4 { if on { 120.0 } else { 80.0 } } else { 190.0 }, 0.0..0.0);
        assert_eq!(r.dip.verdict, Verdict::Better, "{r:?}");
        assert!((r.dip.off - 80.0).abs() < 3.0 && (r.dip.on - 120.0).abs() < 4.0, "{r:?}");
    }

    /// `cargo test --release -- --ignored --nocapture random_dips`: how well it does when dips
    /// come at random, alike or very different in depth and length
    #[test]
    #[ignore]
    fn random_dips() {
        let mut rng = Rng(3);
        for (every, better, mixed) in [
            (4.0f32, 80.0f32, false),
            (4.0, 100.0, false),
            (4.0, 120.0, false),
            (12.0, 80.0, false),
            (12.0, 100.0, false),
            (12.0, 120.0, false),
            (4.0, 80.0, true),
            (4.0, 120.0, true),
            (12.0, 80.0, true),
            (12.0, 120.0, true),
        ] {
            let (mut dip, mut any) = (0, 0);
            for _ in 0..300 {
                let mut events: Vec<(f64, f64, f64)> = vec![];
                let mut t = 0.0f32;
                while t < 700.0 {
                    t += -every * rng.unit().max(1e-6).ln() + 0.5;
                    let (spread, secs) = if mixed { (0.35, 0.1 + 1.1 * rng.unit() as f64) } else { (0.18, 0.5) };
                    events.push((t as f64, (1.0 + spread * rng.normal()).clamp(0.4, 1.6) as f64, secs));
                }
                let (r, _) = play(|on, t| {
                    let i = events.partition_point(|e| e.0 <= t);
                    match i.checked_sub(1).map(|i| events[i]) {
                        Some((at, k, secs)) if t - at < secs => (if on { better as f64 } else { 80.0 }) * k,
                        _ => 185.0 + 10.0 * (t * 0.7).sin(),
                    }
                }, 0.0..0.0);
                dip += (r.dip.verdict == Verdict::Better) as u32;
                any += [r.avg, r.low1, r.dip].iter().any(|c| c.verdict == Verdict::Better) as u32;
            }
            println!("every {every} s{}, 80 to {better}: deepest dip {dip}/300, any number {any}/300", if mixed { " mixed" } else { "" });
        }
    }

    #[test]
    fn noise_rarely_passes_for_a_difference() {
        let mut rng = Rng(7);
        let (mut false_alarms, mut found) = (0, 0);
        for _ in 0..4000 {
            let mut game = |gain: f32| -> Vec<Round> {
                (0..ROUNDS)
                    .map(|i| {
                        let k = if on_in(i) { 1.0 + gain } else { 1.0 };
                        (120.0 * k * (1.0 + 0.03 * rng.normal()), 80.0 * k * (1.0 + 0.03 * rng.normal()), 70.0 * k * (1.0 + 0.03 * rng.normal()))
                    })
                    .collect()
            };
            let r = compare(&game(0.0));
            false_alarms += [r.avg, r.low1, r.dip].iter().any(|c| c.verdict != Verdict::Same) as u32;
            found += (compare(&game(0.10)).avg.verdict == Verdict::Better) as u32;
        }
        println!("false alarms {false_alarms}/4000, 10% gains found {found}/4000");
        assert!(false_alarms < 240, "{false_alarms}");
        assert!(found > 3400, "{found}");
    }
}
