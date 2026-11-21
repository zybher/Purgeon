//! The overlay growing into the full window.
//!
//! The overlay and the main window are two separate windows, so the morph is played inside the
//! main window: just before it appears, the part of the screen it will cover is copied, and its
//! first frames draw that copy as their background, with the overlay's bar on top exactly where
//! the overlay was. The bar then grows into the window while the numbers that also appear on the
//! Dashboard (CPU, GPU, temperature, RAM) glide and grow into their places there, the rest of
//! the bar fades, and the page fades and rises in. Off when Windows' animation effects are off.

use super::widgets;
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Stroke};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Length of the whole morph, seconds
const DUR: f32 = 0.66;
/// The bar grows over this part of it
const GROW: f32 = 0.44;

/// Something the overlay drew, in its own points, so the morph can draw it again
#[derive(Clone)]
pub enum Piece {
    /// `key` names the Dashboard number it flies to; None fades away with the bar
    Text { key: Option<&'static str>, text: String, pos: Pos2, align: Align2, font: FontId, col: Color32 },
    Icon { name: String, path: Option<Arc<str>>, rect: Rect },
    Meter { rect: Rect, frac: f32, col: Color32 },
}

impl Piece {
    fn scaled(&self, f: &dyn Fn(Pos2) -> Pos2, k: f32) -> Piece {
        match self.clone() {
            Piece::Text { key, text, pos, align, mut font, col } => {
                font.size *= k;
                Piece::Text { key, text, pos: f(pos), align, font, col }
            }
            Piece::Icon { name, path, rect } => Piece::Icon { name, path, rect: Rect::from_min_max(f(rect.min), f(rect.max)) },
            Piece::Meter { rect, frac, col } => Piece::Meter { rect: Rect::from_min_max(f(rect.min), f(rect.max)), frac, col },
        }
    }
}

/// What the overlay drew in its last frame
static DRAWN: Mutex<Vec<Piece>> = Mutex::new(Vec::new());
/// A morph asked for by the overlay, waiting for the main window's next frame
static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
/// Where the Dashboard drew each number last frame: left-centre position and font size
static TARGETS: Mutex<Option<HashMap<&'static str, (Pos2, f32)>>> = Mutex::new(None);

struct Pending {
    pieces: Vec<Piece>,
    /// The overlay window on screen, physical pixels
    bar: [i32; 4],
    /// The screen under where the main window will be, and where that is
    shot: Option<egui::ColorImage>,
    shot_at: [i32; 4],
}

/// Called by the overlay at the start of each frame
pub fn record_clear() {
    DRAWN.lock().unwrap_or_else(|e| e.into_inner()).clear();
}
pub fn record(p: Piece) {
    DRAWN.lock().unwrap_or_else(|e| e.into_inner()).push(p);
}

/// Called by the Dashboard for each number the morph can fly to
pub fn mark(key: &'static str, pos: Pos2, size: f32) {
    TARGETS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key, (pos, size));
}

fn target(key: &str) -> Option<(Pos2, f32)> {
    TARGETS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.get(key).copied()
}

/// The overlay's expand button was clicked: copies the screen where the main window will
/// appear, then shows it. Returns false when there will be no morph (the window just opens).
pub fn begin() -> bool {
    if crate::anim::reduced() {
        return false;
    }
    let Some(bar) = super::overlay::bar_rect() else { return false };
    let Some(at) = super::overlay::main_rect() else { return false };
    let shot = super::icons::grab_screen(at);
    if shot.is_none() {
        return false;
    }
    let pieces = DRAWN.lock().unwrap_or_else(|e| e.into_inner()).clone();
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Pending { pieces, bar, shot, shot_at: at });
    true
}

pub struct Run {
    start: Instant,
    bg: Option<(egui::TextureHandle, Rect)>,
    /// The bar's rect at the start, in the main window's points
    from: Rect,
    pieces: Vec<Piece>,
    /// Dev check only: held at this point (`PURGEON_MORPH_AT`, with `PURGEON_SHOT`)
    frozen: Option<f32>,
}

/// Dev check: in screenshot mode, `PURGEON_MORPH_AT=0.3` plays the morph from a bar drawn at
/// the top centre of the (off-screen) window, held at that point, so the screenshot shows it.
/// No screen is copied: the background is a plain grey.
pub fn test_begin(ctx: &egui::Context, snap: Option<&crate::sys::Snapshot>) -> bool {
    let Some(at) = std::env::var("PURGEON_MORPH_AT").ok().and_then(|v| v.parse::<f32>().ok()) else { return false };
    if std::env::var_os("PURGEON_SHOT").is_none() || snap.is_none() {
        return false;
    }
    let Some((ox, oy)) = crate::platform::client_origin() else { return false };
    let ppp = ctx.pixels_per_point();
    // the overlay's drawing, made off to the side and invisible, just to note its pieces down
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(860.0, super::overlay::H));
    let mut ui = egui::Ui::new(ctx.clone(), egui::LayerId::background(), egui::Id::new("morph_test"), egui::UiBuilder::new().max_rect(area).invisible());
    super::overlay::test_bar(&mut ui, snap);
    let pieces = DRAWN.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let w = ctx.screen_rect().width() * ppp;
    let (bw, bh) = (860.0 * ppp, super::overlay::H * ppp);
    let x = ox + ((w - bw) / 2.0) as i32;
    let y = oy + (10.0 * ppp) as i32;
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Pending { pieces, bar: [x, y, x + bw as i32, y + bh as i32], shot: None, shot_at: [0; 4] });
    FREEZE.lock().unwrap_or_else(|e| e.into_inner()).replace(at.clamp(0.0, 1.0));
    true
}

static FREEZE: Mutex<Option<f32>> = Mutex::new(None);

/// The main window picks up a morph the overlay asked for
pub fn take(ctx: &egui::Context) -> Option<Run> {
    let p = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take()?;
    let (ox, oy) = crate::platform::client_origin()?;
    let ppp = ctx.pixels_per_point();
    let to_pts = |x: i32, y: i32| pos2((x - ox) as f32 / ppp, (y - oy) as f32 / ppp);
    let screen = ctx.screen_rect();
    // the bar in window points, kept inside the window when it was elsewhere on the screen
    let mut from = Rect::from_min_max(to_pts(p.bar[0], p.bar[1]), to_pts(p.bar[2], p.bar[3]));
    let shift = vec2(
        (screen.min.x - from.min.x).max(0.0) + (screen.max.x - from.max.x).min(0.0),
        (screen.min.y - from.min.y).max(0.0) + (screen.max.y - from.max.y).min(0.0),
    );
    from = from.translate(shift);
    // overlay points to main window points
    let k = (p.bar[3] - p.bar[1]) as f32 / super::overlay::H / ppp;
    let f = |q: Pos2| from.min + (q.to_vec2() * k);
    let pieces = p.pieces.iter().map(|x| x.scaled(&f, k)).collect();
    let bg = p.shot.map(|img| {
        let tex = ctx.load_texture("morph_bg", img, egui::TextureOptions::LINEAR);
        (tex, Rect::from_min_max(to_pts(p.shot_at[0], p.shot_at[1]), to_pts(p.shot_at[2], p.shot_at[3])))
    });
    let frozen = *FREEZE.lock().unwrap_or_else(|e| e.into_inner());
    Some(Run { start: Instant::now(), bg, from, pieces, frozen })
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
fn in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}
fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect::from_min_max(a.min.lerp(b.min, t), a.max.lerp(b.max, t))
}

impl Run {
    pub fn done(&self) -> bool {
        self.frozen.is_none() && self.start.elapsed().as_secs_f32() >= DUR
    }

    /// How far along, 0..1
    fn t(&self) -> f32 {
        if let Some(f) = self.frozen {
            return f;
        }
        (self.start.elapsed().as_secs_f32() / DUR).min(1.0)
    }

    /// The bar's shape now, in window points
    pub fn shape(&self, full: Rect) -> Rect {
        lerp_rect(self.from, full, in_out(self.t() * DUR / GROW))
    }

    /// How visible the real window contents are now
    pub fn content(&self) -> f32 {
        smooth(0.35, 0.8, self.t())
    }

    /// Draws the copied screen, then the growing bar (under the window's own contents)
    pub fn paint_back(&self, p: &egui::Painter, full: Rect) {
        match &self.bg {
            Some((tex, r)) => {
                p.image(tex.id(), *r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                p.rect_filled(full, 0.0, Color32::from_gray(0x55));
            }
        }
        let g = in_out(self.t() * DUR / GROW);
        let s = self.shape(full);
        let round = 10.0 + 4.0 * g;
        p.rect_filled(s, round, mix(card(), chrome(), g));
        p.rect_stroke(s, round, Stroke::new(1.0_f32, border().gamma_multiply(1.0 - g)));
    }

    /// Draws the bar's pieces on top: numbers fly to their places, the rest rides and fades
    pub fn paint_front(&self, ctx: &egui::Context, full: Rect) {
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("morph")));
        let t = self.t();
        let s = self.shape(full);
        let ride = s.min - self.from.min;
        let fade = 1.0 - smooth(0.0, 0.32, t);
        let mut flown = 0;
        for piece in &self.pieces {
            match piece {
                Piece::Text { key: Some(key), text, pos, align, font, col } if target(key).is_some() => {
                    let (to, size) = target(key).unwrap_or((*pos, font.size));
                    // each number leaves a little after the one before
                    let d = flown as f32 * 0.05;
                    flown += 1;
                    let u = in_out((t - d) / (0.62 - d));
                    // its left edge, so it lines up with where the page draws it
                    let w = p.layout_no_wrap(text.clone(), font.clone(), *col).size().x;
                    let start = *pos - vec2(w * align.x().to_factor(), 0.0);
                    let f = FontId::new(font.size + (size - font.size) * u, font.family.clone());
                    // lands, then hands over to the page's own number
                    let a = 1.0 - smooth(0.6, 0.84, t);
                    // ends in the colour the page draws it in
                    let goal = if matches!(*key, "cpu" | "gpu") { txt() } else { txt_faint() };
                    let c = mix(*col, goal, u).gamma_multiply(a);
                    p.text(start.lerp(to, u), Align2::LEFT_CENTER, text, f, c);
                }
                Piece::Text { text, pos, align, font, col, .. } => {
                    p.text(*pos + ride, *align, text, font.clone(), col.gamma_multiply(fade));
                }
                Piece::Icon { name, path, rect } => {
                    if fade > 0.01 {
                        let mut lp = p.clone();
                        lp.set_opacity(fade);
                        super::icons::paint(ctx, &lp, rect.translate(ride), name, path.as_ref());
                    }
                }
                Piece::Meter { rect, frac, col } => {
                    if fade > 0.01 {
                        let mut lp = p.clone();
                        lp.set_opacity(fade);
                        widgets::meter(&lp, rect.translate(ride), *frac, *col);
                    }
                }
            }
        }
    }
}
