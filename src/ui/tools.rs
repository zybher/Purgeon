//! The Dashboard's two helpers, each on a row of its own: "Boost a game", then "Free up memory".
//! Both say plainly what they do and what they can't do.

use super::widgets;
use crate::apps;
use crate::boost::{self, Plan, Priority};
use crate::fmt;
use crate::games::{self, Game, Target};
use crate::anim::*;
use crate::steady::{self, App, Cores};
use crate::sys::Snapshot;
use crate::theme::*;
use crate::trial::{self, Change, Verdict, View};
use eframe::egui::{self, pos2, vec2, Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, Ui};
use std::sync::Mutex;

struct State {
    /// The Boost row is unfolded to every game
    all: bool,
    /// The tiles' order (game names), held while the pointer is on the row
    order: Vec<String>,
    /// Close waiting for the user's yes: exe, name, has a window
    confirm: Option<(String, String, bool)>,
    /// The Windows prompt for the FPS test was declined
    declined: bool,
}
static STATE: Mutex<State> = Mutex::new(State { all: false, order: Vec::new(), confirm: None, declined: false });

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn cards(ui: &mut Ui, s: &Snapshot, w: f32, gap: f32) {
    boost_row(ui, s, w);
    ui.add_space(gap);
    free_card(ui, w);
    confirm(ui.ctx());
}

/// A button placed at `r` without moving the page's layout
pub(super) fn button(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, label: &str, primary: bool, enabled: bool) -> bool {
    let mut b = egui::Button::new(RichText::new(label).font(sb(12.5)).color(if primary && enabled { Color32::WHITE } else { txt() }));
    if primary {
        b = b.fill(ACCENT);
    }
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(r).id_salt(id));
    // add_sized centres the label
    c.add_enabled_ui(enabled, |ui| ui.add_sized(r.size(), b).clicked()).inner
}

/// A status line with a coloured dot
fn line(ui: &Ui, x: f32, y: f32, w: f32, dot: Color32, text: &str) {
    let p = ui.painter();
    p.circle_filled(pos2(x + 4.0, y), 3.5, dot);
    widgets::fit(p, pos2(x + 16.0, y), Align2::LEFT_CENTER, text, reg(12.5), w - 16.0, txt());
}

const OK: Color32 = GPU_COL;
const FREE_ROWS: usize = 4;
const FREE_ROW_H: f32 = 34.0;

/// A lightning bolt, 16 x 18 at `k` = 1
fn bolt(p: &egui::Painter, c: Pos2, k: f32, col: Color32) {
    for t in [[(2.0, -9.0), (1.0, 1.5), (-6.0, 1.5)], [(-2.0, 9.0), (-1.0, -1.5), (6.0, -1.5)]] {
        p.add(egui::Shape::convex_polygon(t.iter().map(|&(x, y)| c + vec2(x, y) * k).collect(), col, Stroke::NONE));
    }
}

/// CPU, GPU and memory of a game's processes, None while it isn't running
fn usage(s: &Snapshot, target: &Target) -> Option<(f32, f32, u64)> {
    let (mut cpu, mut gpu, mut private, mut shared) = (0.0, 0.0, 0, 0);
    let mut any = false;
    for pr in s.procs.iter().filter(|pr| pr.session != 0 && target.matches(pr)) {
        any = true;
        cpu += pr.cpu;
        gpu += pr.gpu;
        private += pr.private_ws;
        shared = pr.shared().max(shared);
    }
    any.then_some((cpu, gpu, private + shared))
}

/// Screenshot runs can show a running boost without boosting anything: PURGEON_BOOST_DEMO=roblox
/// (part of a game's name)
fn shown_boost(s: &Snapshot, found: &[Game]) -> Option<boost::Status> {
    if let Some(want) = std::env::var("PURGEON_BOOST_DEMO").ok().filter(|_| crate::shot::on()) {
        let g = found.iter().find(|g| g.name.to_lowercase().contains(&want))?;
        return Some(boost::Status {
            target: g.target.clone(),
            name: g.name.clone(),
            running: usage(s, &g.target).is_some(),
            priority: Priority::Raised,
            plan: Plan::Switched("Balanced".into()),
        });
    }
    boost::status()
}

/// The full-width Boost row: the installed games as icons to click, or the game being boosted
fn boost_row(ui: &mut Ui, s: &Snapshot, w: f32) {
    let open: Vec<std::sync::Arc<str>> = apps::open_apps().into_iter().filter(|a| a.exe != "purgeon.exe").filter_map(|a| a.path).collect();
    let ctx = ui.ctx().clone();
    let found = games::list(&open, move || ctx.request_repaint());
    let list = found.as_deref().map_or(&[][..], |v| v.as_slice());
    match shown_boost(s, list) {
        Some(b) => boosted(ui, s, w, &b, list),
        None => picker(ui, s, w, list, found.is_none()),
    }
}

fn boost_surface(p: &egui::Painter, r: Rect, strength: f32) {
    widgets::card(p, r, mix(card(), ACCENT, 0.05 + 0.07 * strength));
    p.rect_stroke(r, widgets::R_CARD, Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.45 + 0.4 * strength)));
}

const TILE_H: f32 = 124.0;
const TILE_GAP: f32 = 10.0;

fn picker(ui: &mut Ui, s: &Snapshot, w: f32, list: &[Game], looking: bool) {
    let mut tiles: Vec<(&Game, Option<(f32, f32, u64)>)> = list.iter().map(|g| (g, usage(s, &g.target))).collect();
    let ctx = ui.ctx().clone();
    let (all, order) = {
        let st = state();
        (st.all, st.order.clone())
    };
    let hover_id = egui::Id::new("boost_row_hovered");
    // running ones first (biggest first), then by name; but the tiles hold still under the pointer
    if ctx.data(|d| d.get_temp::<bool>(hover_id)).unwrap_or(false) {
        tiles.sort_by_key(|(g, _)| order.iter().position(|n| *n == g.name).unwrap_or(usize::MAX));
    } else {
        tiles.sort_by_key(|(_, u)| std::cmp::Reverse(u.map(|u| u.2)));
        state().order = tiles.iter().map(|(g, _)| g.name.clone()).collect();
    }

    let inner = w - 36.0;
    let cols = (((inner + TILE_GAP) / (128.0 + TILE_GAP)).floor() as usize).max(1);
    let more = tiles.len() > cols;
    // one line of tiles, the last being "more" when they don't all fit
    let shown = if more && !all { cols - 1 } else { tiles.len() };
    let cells = shown + more as usize;
    let across = cells.clamp(1, cols); // few games share the whole row
    let tw = (inner - TILE_GAP * (across as f32 - 1.0)) / across as f32;
    let lines = cells.max(1).div_ceil(cols) as f32;
    let h = glide(&ctx, egui::Id::new("boost_row_h"), 58.0 + lines * (TILE_H + TILE_GAP) - TILE_GAP + 18.0, 14.0);

    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let over = ctx.pointer_hover_pos().is_some_and(|q| r.contains(q));
    ctx.data_mut(|d| d.insert_temp(hover_id, over));
    boost_surface(ui.painter(), r, 0.0);
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    bolt(&p, pos2(r.min.x + 27.0, r.min.y + 29.0), 1.1, ACCENT);
    p.text(pos2(r.min.x + 45.0, r.min.y + 29.0), Align2::LEFT_CENTER, "Boost a game", sb(17.0), hi());

    let origin = pos2(r.min.x + 18.0, r.min.y + 58.0);
    if tiles.is_empty() {
        let er = Rect::from_min_size(origin, vec2(inner, TILE_H));
        p.rect_filled(er, 10.0, card());
        let t = if looking { "Looking for games…" } else { "No games found on this PC" };
        p.text(er.center(), Align2::CENTER_CENTER, t, reg(13.0), txt_faint());
        return;
    }
    for i in 0..cells {
        let at = origin + vec2((i % cols) as f32 * (tw + TILE_GAP), (i / cols) as f32 * (TILE_H + TILE_GAP));
        let tr = Rect::from_min_size(at, vec2(tw, TILE_H));
        // while the row is still growing, what's cut off can't be clicked
        let hit = tr.intersect(r);
        if !hit.is_positive() {
            continue;
        }
        let tile = tiles.get(i).filter(|_| i < shown);
        let id = egui::Id::new(("boost_tile", tile.map(|(g, _)| &g.name)));
        let resp = ui.interact(hit, id, Sense::click());
        let (hk, pr) = hover_fx(&ctx, &resp);
        if resp.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        p.rect_filled(tr, 10.0, mix(card(), ACCENT, 0.16 * hk + 0.10 * pr));
        p.rect_stroke(tr, 10.0, Stroke::new(1.0_f32, mix(border(), ACCENT, hk)));
        let (cx, text_w) = (tr.center().x, tw - 16.0);
        let ic = pos2(cx, tr.min.y + 38.0);
        let Some(&(g, used)) = tile else {
            // the tile that unfolds or folds the row
            let d = if all { -1.0 } else { 1.0 };
            let st = Stroke::new(1.8_f32, mix(txt_dim(), hi(), hk));
            p.line_segment([ic + vec2(-7.0, -3.5 * d), ic + vec2(0.0, 3.5 * d)], st);
            p.line_segment([ic + vec2(0.0, 3.5 * d), ic + vec2(7.0, -3.5 * d)], st);
            let label = if all { "Show less".to_string() } else { format!("{} more", tiles.len() - shown) };
            p.text(pos2(cx, tr.min.y + 80.0), Align2::CENTER_CENTER, label, sb(12.5), mix(txt_dim(), hi(), hk));
            if resp.clicked() {
                state().all = !all;
            }
            continue;
        };
        super::icons::paint(&ctx, &p, Rect::from_center_size(ic, vec2(48.0, 48.0)), &g.name, g.icon.as_ref());
        widgets::fit(&p, pos2(cx, tr.min.y + 80.0), Align2::CENTER_CENTER, &g.name, sb(12.5), text_w, hi());
        let line_y = tr.min.y + 103.0;
        if hk > 0.5 {
            let tc = hi().gamma_multiply((hk - 0.5) * 2.0);
            bolt(&p, pos2(cx - 22.0, line_y), 0.7, ACCENT);
            p.text(pos2(cx - 12.0, line_y), Align2::LEFT_CENTER, "Boost", sb(12.5), tc);
        } else {
            let (t, col) = match used {
                Some((cpu, _, mem)) => {
                    let cpu = if cpu < 0.05 { "0%".to_string() } else { fmt::pct(cpu) };
                    (format!("{cpu} CPU  ·  {}", fmt::bytes(mem)), OK)
                }
                None => ("Not running".to_string(), txt_faint()),
            };
            widgets::fit(&p, pos2(cx, line_y), Align2::CENTER_CENTER, &t, reg(11.0), text_w, col.gamma_multiply(1.0 - hk * 2.0));
        }
        if resp.clicked() {
            boost::start(&g.target, &g.name, g.icon.as_deref());
            state().all = false;
        }
    }
}

fn boosted(ui: &mut Ui, s: &Snapshot, w: f32, b: &boost::Status, list: &[Game]) {
    let ctx = ui.ctx().clone();
    let view = trial::demo().unwrap_or_else(|| trial::view(&b.target));
    let testing = matches!(view, View::Running { .. });
    let sv = steady::view();
    let steady_h = STEADY_H + if sv.on { sv.apps.len().max(1) as f32 * STEADY_ROW + 8.0 } else { 0.0 };
    // the test's part: a heading with its button, and the three numbers once there are some
    let extra = match view {
        View::Done(_) => TEST_H + RESULT_H,
        _ if b.running || testing => TEST_H,
        _ => 0.0,
    };
    let h = glide(&ctx, egui::Id::new("boosted_h"), 142.0 + steady_h + extra, 14.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    boost_surface(ui.painter(), r, if b.running { 1.0 } else { 0.0 });
    // clipped while it grows, but not the outline
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));

    let ir = Rect::from_min_size(r.min + vec2(22.0, 22.0), vec2(64.0, 64.0));
    // the game's own icon, the saved one until it's found; a boost saved by exe name takes the
    // running program's
    let path = list.iter().find(|g| g.name == b.name).and_then(|g| g.icon.clone()).or_else(|| crate::settings::text("boost_path").map(Into::into)).or_else(|| {
        if !b.target.dirs.is_empty() {
            return None;
        }
        s.procs.iter().find(|pr| b.target.matches(pr) && pr.path.is_some()).and_then(|pr| pr.path.clone())
    });
    super::icons::paint(&ctx, &p, ir, &b.name, path.as_ref());

    let br = Rect::from_min_size(pos2(r.max.x - 22.0 - 100.0, r.min.y + 37.0), vec2(100.0, 34.0));
    if button(ui, br, "boost_stop", "Stop boost", false, true) {
        trial::cancel();
        boost::stop();
    }

    // what it is using right now; a narrow window leaves the GPU out
    let mut right = br.min.x - 10.0;
    if b.running {
        let (cpu, gpu, mem) = usage(s, &b.target).unwrap_or_default();
        let pct = |v: f32| if v < 0.05 { "0%".to_string() } else { fmt::pct(v) };
        let mut stats = vec![("Memory", fmt::bytes(mem), MEM_COL), ("CPU", pct(cpu), CPU_COL)];
        if s.gpu_proc && w >= 720.0 {
            stats.insert(1, ("GPU", pct(gpu), GPU_COL));
        }
        for (label, value, col) in stats {
            right -= 88.0;
            p.circle_filled(pos2(right + 3.0, r.min.y + 43.0), 3.0, col);
            p.text(pos2(right + 11.0, r.min.y + 43.0), Align2::LEFT_CENTER, label, reg(11.5), txt_faint());
            p.text(pos2(right, r.min.y + 65.0), Align2::LEFT_CENTER, value, sb(17.0), txt());
        }
    }

    let x = ir.max.x + 18.0;
    let name_w = right - 14.0 - x;
    widgets::fit(&p, pos2(x, r.min.y + 43.0), Align2::LEFT_CENTER, &b.name, sb(22.0), name_w, hi());
    let (state_text, col) = if testing { ("Testing", ACCENT) } else if b.running { ("Boosted", ACCENT) } else { ("Waiting for it to open", txt_faint()) };
    bolt(&p, pos2(x + 6.0, r.min.y + 70.0), 0.8, col);
    widgets::fit(&p, pos2(x + 18.0, r.min.y + 70.0), Align2::LEFT_CENTER, state_text, sb(12.5), name_w - 18.0, col);

    p.line_segment([pos2(r.min.x + 22.0, r.min.y + 102.0), pos2(r.max.x - 22.0, r.min.y + 102.0)], Stroke::new(1.0_f32, wa(14)));
    let (fx, fy) = (r.min.x + 22.0, r.min.y + 122.0);
    let half = (w - 56.0) / 2.0;
    steady_part(ui, &p, w, b.running, pos2(r.min.x, r.min.y + 142.0), r.max.y, &sv);
    if extra > 0.0 {
        test_part(ui, &p, s, w, b, &sv, pos2(r.min.x, r.min.y + 142.0 + steady_h), r.max.y, &view);
    }
    if testing {
        let what = if sv.on { "Boost stays on, Steadier frames switches off and on" } else { "Boost switches off and on until the test ends" };
        line(ui, fx, fy, half * 2.0, ACCENT, what);
        return;
    }
    if !b.running {
        line(ui, fx, fy, half * 2.0, txt_faint(), &format!("It gets boosted as soon as {} opens", b.name));
        return;
    }
    let (dot, t) = match b.priority {
        Priority::Raised => (OK, "Priority: Above normal"),
        Priority::LeftAlone => (txt_faint(), "Priority: already changed, left alone"),
        Priority::Refused => (WARN_COL, "Priority: blocked by the app"),
        Priority::Waiting => (txt_faint(), "Priority: waiting"),
    };
    line(ui, fx, fy, half, dot, t);
    let (dot, t) = match &b.plan {
        Plan::Switched(was) => (OK, format!("Power plan: High performance (was {was})")),
        Plan::AlreadyHigh => (OK, "Power plan: High performance".to_string()),
        Plan::Missing => (txt_faint(), "Power plan: none faster on this PC".to_string()),
        Plan::Usual => (txt_faint(), "Power plan: switching…".to_string()),
    };
    line(ui, fx + half + 12.0, fy, half, dot, &t);
}

const TEST_H: f32 = 70.0;
const RESULT_H: f32 = 130.0;
const STEADY_H: f32 = 64.0;
const STEADY_ROW: f32 = 34.0;

/// The experimental switch, and the background apps it may lower, each with its own yes
fn steady_part(ui: &mut Ui, p: &egui::Painter, w: f32, running: bool, at: Pos2, bottom: f32, v: &steady::View) {
    let (x0, x1) = (at.x + 22.0, at.x + w - 22.0);
    p.line_segment([pos2(x0, at.y), pos2(x1, at.y)], Stroke::new(1.0_f32, wa(14)));
    let sr = Rect::from_min_size(pos2(x1 - 40.0, at.y + 21.0), vec2(40.0, 22.0));
    if sr.max.y <= bottom {
        let mut on = v.on;
        // in a child, so it doesn't move the page's layout
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(sr).id_salt("steady_switch"));
        widgets::switch(&mut c, sr, &mut on);
        if on != v.on {
            trial::cancel();
            steady::set_on(on);
        }
    }
    let t = p.text(pos2(x0, at.y + 22.0), Align2::LEFT_CENTER, "Steadier frames", sb(15.0), hi());
    p.text(pos2(t.max.x + 10.0, at.y + 23.0), Align2::LEFT_CENTER, "Experimental", reg(12.0), WARN_COL);
    let (text, col) = match (steady::fast(), v.on, v.cores) {
        (Some(f), false, _) => (format!("Prefer the {} fast cores, lower background apps you pick", f.cores), txt_dim()),
        (None, false, _) => ("Lower background apps you pick while you play".to_string(), txt_dim()),
        (None, true, _) => ("Cores: no fast group on this CPU to prefer".to_string(), txt_dim()),
        (Some(f), true, Cores::Preferred) if running => (format!("Cores: preferring the {} fast ones", f.cores), OK),
        (_, true, Cores::Refused) if running => ("Cores: blocked by the game".to_string(), WARN_COL),
        (_, true, Cores::LeftAlone) if running => ("Cores: already chosen, left alone".to_string(), txt_dim()),
        (Some(f), true, _) => (format!("Cores: the {} fast ones, once the game runs", f.cores), txt_dim()),
    };
    widgets::fit(p, pos2(x0, at.y + 44.0), Align2::LEFT_CENTER, &text, reg(12.0), sr.min.x - 14.0 - x0, col);
    if !v.on {
        return;
    }
    if v.apps.is_empty() {
        let y = at.y + STEADY_H + STEADY_ROW / 2.0;
        widgets::fit(p, pos2(x0, y), Align2::LEFT_CENTER, "No background app worth lowering right now", reg(12.5), x1 - x0, txt_faint());
    }
    for (i, a) in v.apps.iter().enumerate() {
        let cy = at.y + STEADY_H + (i as f32 + 0.5) * STEADY_ROW;
        let br = Rect::from_min_size(pos2(x1 - 70.0, cy - 13.0), vec2(70.0, 26.0));
        if br.max.y > bottom {
            break;
        }
        super::icons::paint(ui.ctx(), p, Rect::from_center_size(pos2(x0 + 9.0, cy), vec2(18.0, 18.0)), &a.name, Some(&a.path));
        let (state, col) = match a.app {
            App::Offered => (format!("{} CPU", fmt::pct(a.cpu)), txt_faint()),
            App::Waiting => (if running { "Not running or not eligible" } else { "When the game runs" }.to_string(), txt_faint()),
            App::Lowered => ("Below normal".to_string(), OK),
            App::InUse => ("In use, left at Normal".to_string(), txt_faint()),
            App::LeftAlone => ("Left alone".to_string(), txt_faint()),
        };
        let tr = p.text(pos2(br.min.x - 12.0, cy), Align2::RIGHT_CENTER, state, reg(12.0), col);
        widgets::fit(p, pos2(x0 + 26.0, cy), Align2::LEFT_CENTER, &a.name, reg(12.5), tr.min.x - 12.0 - (x0 + 26.0), txt());
        let picked = a.app != App::Offered;
        if button(ui, br, ("steady_app", &a.key), if picked { "Undo" } else { "Lower" }, false, true) {
            trial::cancel();
            steady::pick(&a.key, !picked);
        }
    }
}

/// The before and after under the boosted game: what it found, or the button that finds out
#[allow(clippy::too_many_arguments)]
fn test_part(ui: &mut Ui, p: &egui::Painter, s: &Snapshot, w: f32, b: &boost::Status, sv: &steady::View, at: Pos2, bottom: f32, view: &View) {
    let (x0, x1) = (at.x + 22.0, at.x + w - 22.0);
    p.line_segment([pos2(x0, at.y), pos2(x1, at.y)], Stroke::new(1.0_f32, wa(14)));
    let changes = if sv.on { sv.changes() } else { b.priority == Priority::Raised || matches!(b.plan, Plan::Switched(_)) };
    // screenshot runs show it as it is for an administrator
    let admin = s.info.admin || trial::demo().is_some();
    let running = matches!(view, View::Running { .. });
    let label = match view {
        View::Running { .. } => "Cancel",
        _ if !admin => "Elevate to test",
        View::Done(_) | View::Failed(_) => "Test again",
        View::Idle => "Run 5 min test",
    };
    let bw = if admin { 112.0 } else { 124.0 };
    let br = Rect::from_min_size(pos2(x1 - bw, at.y + 18.0), vec2(bw, 34.0));
    let can = running || (b.running && changes);
    // not while the card is still growing to fit it
    if can && br.max.y <= bottom && button(ui, br, "boost_test", label, !running && !matches!(view, View::Done(_)), true) {
        if running {
            trial::cancel();
        } else if admin {
            trial::start(&b.target);
        } else if crate::platform::relaunch_as_admin(&[]) {
            crate::quit();
        } else {
            state().declined = true;
        }
    }

    let (title, col, under) = match *view {
        View::Running { waiting: true, .. } => (format!("Paused, switch to {}", b.name), WARN_COL, "It counts only while the game is in front".to_string()),
        View::Running { round, left, .. } => {
            let secs = left.max(0.0) as u32;
            (format!("Round {} of {}", (round + 1).min(trial::ROUNDS), trial::ROUNDS), hi(), format!("{}:{:02} left, keep playing the same way", secs / 60, secs % 60))
        }
        View::Failed(why) => ("No result".to_string(), WARN_COL, why.to_string()),
        View::Done(r) => {
            let any = |v: Verdict| [r.avg, r.low1, r.dip].iter().any(|c| c.verdict == v);
            let (helped, hurt) = if r.steady {
                ("Steadier frames helped", "Steadier frames made it worse")
            } else {
                ("Boost helped", "Boost made it worse")
            };
            let (title, col) = match (any(Verdict::Better), any(Verdict::Worse)) {
                (true, false) => (helped, OK),
                (false, true) => (hurt, WARN_COL),
                (true, true) => ("Mixed result", WARN_COL),
                _ => ("No measurable difference", hi()),
            };
            let under = match r.capped {
                Some(cap) => format!("FPS looks capped at {cap}, so a gain had no room to show"),
                None => format!("{} rounds, alternating off and on", trial::ROUNDS),
            };
            (title.to_string(), col, under)
        }
        View::Idle => {
            let under = if !changes {
                if sv.on { "Nothing to compare until it changes something" } else { "Nothing to compare, Boost couldn't change anything here" }.to_string()
            } else if admin {
                "Not measured yet".to_string()
            } else if state().declined {
                "Cancelled, counting FPS needs administrator".to_string()
            } else {
                "Counting FPS needs administrator".to_string()
            };
            (if sv.on { "Dips without and with Steadier frames" } else { "FPS without and with Boost" }.to_string(), hi(), under)
        }
    };
    let tw = (if can { br.min.x - 14.0 } else { x1 }) - x0;
    widgets::fit(p, pos2(x0, at.y + 26.0), Align2::LEFT_CENTER, &title, sb(15.0), tw, col);
    widgets::fit(p, pos2(x0, at.y + 46.0), Align2::LEFT_CENTER, &under, reg(12.0), tw, txt_dim());

    match *view {
        View::Running { left, .. } => {
            let done = 1.0 - left / trial::SECS as f32;
            widgets::meter(p, Rect::from_min_max(pos2(x0, at.y + 60.0), pos2(x1, at.y + 63.0)), done.clamp(0.0, 1.0), ACCENT);
        }
        View::Done(r) => {
            let pw = (x1 - x0 - 24.0) / 3.0;
            let names = if r.steady { ("Boost alone", "Steadier") } else { ("Without", "With Boost") };
            for (i, (label, c)) in [("Deepest dip", r.dip), ("1% low", r.low1), ("Average", r.avg)].into_iter().enumerate() {
                let pr = Rect::from_min_size(pos2(x0 + i as f32 * (pw + 12.0), at.y + TEST_H), vec2(pw, RESULT_H - 18.0));
                compared(p, pr, label, &c, names);
            }
        }
        _ => {}
    }
}

/// One number without and with Boost, as two bars on the same scale
fn compared(p: &egui::Painter, r: Rect, label: &str, c: &Change, names: (&str, &str)) {
    p.rect_filled(r, 10.0, card());
    let (x0, x1) = (r.min.x + 14.0, r.max.x - 14.0);
    let top = r.min.y + 18.0;
    widgets::fit(p, pos2(x0, top), Align2::LEFT_CENTER, label, sb(12.5), x1 - x0, txt_dim());
    let (text, col) = match c.verdict {
        Verdict::Better => (format!("{:+.1}%", c.pct), OK),
        Verdict::Worse => (format!("{:+.1}%", c.pct), WARN_COL),
        // steady, but too small to feel
        Verdict::Same if c.pct.abs() > c.noise => (format!("{:+.1}%, too small to feel", c.pct), txt_faint()),
        Verdict::Same => (format!("{:+.1}%, within ±{:.1}% noise", c.pct, c.noise), txt_faint()),
    };
    widgets::fit(p, pos2(x0, top + 19.0), Align2::LEFT_CENTER, &text, sb(12.0), x1 - x0, col);
    let most = c.off.max(c.on).max(1.0);
    let on_col = match c.verdict {
        Verdict::Better => OK,
        Verdict::Worse => WARN_COL,
        Verdict::Same => txt_dim(),
    };
    for (i, (who, v, col)) in [(names.0, c.off, txt_faint()), (names.1, c.on, on_col)].into_iter().enumerate() {
        let y = r.min.y + 63.0 + i as f32 * 27.0;
        p.text(pos2(x0, y), Align2::LEFT_CENTER, who, reg(12.0), txt_dim());
        p.text(pos2(x1, y), Align2::RIGHT_CENTER, format!("{v:.0}"), sb(16.0), hi());
        let bar = Rect::from_min_max(pos2(x0 + 80.0, y - 3.0), pos2(x1 - 44.0, y + 3.0));
        if bar.is_positive() {
            widgets::meter(p, bar, v / most, col);
        }
    }
}

fn free_card(ui: &mut Ui, w: f32) {
    let skip = boost::status().map(|b| b.target);
    let sg = apps::suggestions(skip.as_ref());
    // as tall as what it has to say
    let n = sg.apps.len().min(FREE_ROWS);
    let plenty = sg.avail_frac > 0.25 && n > 0;
    let mut h = 64.0 + if plenty { 18.0 } else { 0.0 };
    h += if n > 0 { n as f32 * FREE_ROW_H + 10.0 } else { 36.0 };
    if sg.busy || sg.note.is_some() {
        h += 28.0;
    }
    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let p = ui.painter().clone();
    widgets::card(&p, r, card());
    let (x, w) = (r.min.x + 18.0, r.width() - 36.0);
    super::pages::caption_w(&p, x, r.min.y + 24.0, "Free up memory", MEM_COL, w - 120.0);
    // how full RAM is, as a bar
    let free = sg.avail_frac.clamp(0.0, 1.0);
    let fcol = if free > 0.25 { OK } else if free > 0.1 { WARN_COL } else { DANGER };
    p.text(pos2(r.max.x - 18.0, r.min.y + 24.0), Align2::RIGHT_CENTER, format!("{:.0}% free", free * 100.0), sb(12.5), fcol);
    widgets::meter(&p, Rect::from_min_size(pos2(x, r.min.y + 44.0), vec2(w, 6.0)), 1.0 - free, MEM_COL);

    let mut y = r.min.y + 64.0;
    if plenty {
        line(ui, x, y, w, OK, "Plenty free, closing apps won't speed things up now");
        y += 18.0;
    }

    let row_h = FREE_ROW_H;
    if sg.apps.is_empty() {
        match sg.warming {
            // still watching: how far along, as a progress bar
            Some(left) => {
                line(ui, x, y + 4.0, w - 90.0, txt_faint(), "Watching for unused apps");
                p.text(pos2(x + w, y + 4.0), Align2::RIGHT_CENTER, format!("{} min left", (left.as_secs() + 59) / 60), reg(12.0), txt_faint());
                let done = 1.0 - left.as_secs_f32() / apps::IDLE_FOR.as_secs_f32();
                widgets::meter(&p, Rect::from_min_size(pos2(x, y + 22.0), vec2(w, 3.0)), done.clamp(0.0, 1.0), txt_faint());
            }
            None => line(ui, x, y + 4.0, w, OK, "No unused apps to close"),
        }
    }
    for (i, a) in sg.apps.iter().take(n).enumerate() {
        let rr = Rect::from_min_size(pos2(r.min.x + 8.0, y + i as f32 * row_h), vec2(r.width() - 16.0, row_h));
        super::icons::paint(ui.ctx(), &p, Rect::from_center_size(pos2(rr.min.x + 20.0, rr.center().y), vec2(18.0, 18.0)), &a.exe, a.path.as_ref());
        let br = Rect::from_min_size(pos2(rr.max.x - 10.0 - 70.0, rr.center().y - 13.0), vec2(70.0, 26.0));
        let mins = a.idle.as_secs() / 60;
        // "+" when it wasn't used at all while Purgeon watched: it has been idle at least this long
        let more = if a.idle_known { "" } else { "+" };
        let idle = if mins >= 120 { format!("idle {}{more} h", mins / 60) } else { format!("idle {mins}{more} min") };
        let tr = p.text(pos2(br.min.x - 12.0, rr.center().y), Align2::RIGHT_CENTER, format!("{}  ·  {idle}", fmt::bytes(a.mem)), reg(12.0), txt_faint());
        widgets::fit(&p, pos2(rr.min.x + 36.0, rr.center().y), Align2::LEFT_CENTER, &a.name, reg(12.5), tr.min.x - 12.0 - (rr.min.x + 36.0), txt());
        if button(ui, br, ("free_close", i), "Close…", false, !sg.busy) {
            state().confirm = Some((a.exe.clone(), a.name.clone(), apps::has_window(&a.exe)));
        }
    }
    let foot = if sg.busy {
        ("Closing…".to_string(), txt_faint())
    } else if let Some((t, ok)) = &sg.note {
        (t.clone(), if *ok { OK } else { WARN_COL })
    } else {
        (String::new(), txt_faint())
    };
    widgets::fit(&p, pos2(x, r.max.y - 20.0), Align2::LEFT_CENTER, &foot.0, reg(12.0), w, foot.1);
    if sg.busy {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
}

/// "Close Discord?" with Cancel and Close. Enter confirms, Esc cancels.
fn confirm(ctx: &egui::Context) {
    let Some((exe, name, windowed)) = state().confirm.clone() else { return };
    let (mut go, mut cancel) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    let screen = ctx.screen_rect();
    ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("close_dim"))).rect_filled(screen, 0.0, Color32::from_black_alpha(110));
    egui::Window::new("close_app")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([400.0, 0.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("Close {name}?")).font(sb(16.0)).color(hi()));
            ui.add_space(8.0);
            let body = if windowed {
                format!("Purgeon asks {name} to close, the same as clicking its X. If something isn't saved, it may ask you first.")
            } else {
                format!("{name} has no window open, so Purgeon ends it straight away. Anything unsaved in it is lost.")
            };
            ui.label(RichText::new(body).font(reg(12.5)).color(txt_dim()));
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let b = egui::Button::new(RichText::new("Close").font(sb(12.5)).color(Color32::WHITE)).fill(if windowed { ACCENT } else { DANGER }).min_size(vec2(80.0, 30.0));
                if ui.add(b).clicked() {
                    go = true;
                }
                if ui.add(egui::Button::new(RichText::new("Cancel").font(sb(12.5))).min_size(vec2(80.0, 30.0))).clicked() {
                    cancel = true;
                }
            });
        });
    if go {
        state().confirm = None;
        apps::close(&exe);
    } else if cancel {
        state().confirm = None;
    }
}
