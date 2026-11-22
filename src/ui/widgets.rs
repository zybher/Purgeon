//! Custom-painted controls: cards, tiles, switch, segmented control, icons.

use crate::anim::*;
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Painter, Pos2, Rect, Response, Rounding, Sense, Shape, Stroke, Ui};

pub const R_CARD: f32 = 12.0;

/// A card surface with a soft shadow and a hairline border
pub fn card(p: &Painter, r: Rect, fill: Color32) {
    let sh = egui::epaint::Shadow { offset: vec2(0.0, 4.0), blur: 14.0, spread: 0.0, color: Color32::from_black_alpha(70) };
    p.add(sh.as_shape(r, Rounding::same(R_CARD)));
    p.rect_filled(r, R_CARD, fill);
    p.rect_stroke(r, R_CARD, Stroke::new(1.0_f32, border()));
}

/// A clickable card that brightens and lifts a little under the pointer.
/// Returns the response and the lifted rect to paint the contents into.
pub fn card_button(ui: &mut Ui, r: Rect, id: egui::Id) -> (Response, Rect) {
    let resp = ui.interact(r, id, Sense::click());
    let h = spring(ui.ctx(), id.with("lift"), resp.hovered(), 18.0);
    let pr = ease(ui.ctx(), id.with("prs"), resp.is_pointer_button_down_on());
    let lr = r.translate(vec2(0.0, -2.0 * h + pr * 1.0));
    card(ui.painter(), lr, mix(crate::theme::card(), card_hi(), h));
    if h > 0.01 {
        ui.painter().rect_stroke(lr, R_CARD, Stroke::new(1.0_f32, wa((h * 14.0) as u8)));
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (resp, lr)
}

/// A caption over a value, laid out in a small tile
pub fn tile(p: &Painter, r: Rect, caption: &str, value: &str, dot: Option<Color32>) {
    p.rect_filled(r, 10.0, crate::theme::card());
    p.rect_stroke(r, 10.0, Stroke::new(1.0_f32, border()));
    let mut x = r.min.x + 14.0;
    if let Some(c) = dot {
        p.circle_filled(pos2(x + 3.0, r.min.y + 18.0), 3.5, c);
        x += 12.0;
    }
    fit(p, pos2(x, r.min.y + 18.0), Align2::LEFT_CENTER, caption, reg(11.5), r.max.x - 10.0 - x, txt_faint());
    fit(p, pos2(r.min.x + 14.0, r.max.y - 18.0), Align2::LEFT_CENTER, value, sb(17.0), r.width() - 24.0, txt());
}

/// Draws text that never spills past `max_w`: it first shrinks (down to 72% of the size),
/// then cuts the end off with an ellipsis. Returns the painted rect.
pub fn fit(p: &Painter, pos: Pos2, align: Align2, s: &str, font: egui::FontId, max_w: f32, col: Color32) -> Rect {
    let width = |t: &str, f: &egui::FontId| p.layout_no_wrap(t.to_string(), f.clone(), col).size().x;
    let max_w = max_w.max(8.0);
    let mut f = font.clone();
    let w = width(s, &f);
    if w <= max_w {
        return p.text(pos, align, s, f, col);
    }
    f.size = (font.size * max_w / w).max(font.size * 0.72).floor();
    if width(s, &f) <= max_w {
        return p.text(pos, align, s, f, col);
    }
    let mut out: String = s.to_string();
    while !out.is_empty() && width(&format!("{out}…"), &f) > max_w {
        out.pop();
    }
    p.text(pos, align, format!("{}…", out.trim_end()), f, col)
}

/// Lays out `n` equal tiles of at least `min_w` in rows; returns their rects and the height used
pub fn tile_grid(origin: Pos2, w: f32, n: usize, min_w: f32, h: f32, gap: f32) -> (Vec<Rect>, f32) {
    let cols = (((w + gap) / (min_w + gap)).floor() as usize).clamp(1, n.max(1));
    let tw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
    let rows = n.div_ceil(cols);
    let rects = (0..n)
        .map(|i| {
            let (c, r) = (i % cols, i / cols);
            Rect::from_min_size(pos2(origin.x + c as f32 * (tw + gap), origin.y + r as f32 * (h + gap)), vec2(tw, h))
        })
        .collect();
    (rects, rows as f32 * h + (rows as f32 - 1.0).max(0.0) * gap)
}

/// iOS-style switch; flips `on` when clicked
pub fn switch(ui: &mut Ui, r: Rect, on: &mut bool) -> Response {
    let resp = ui.allocate_rect(r, Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let ctx = ui.ctx();
    let t = spring(ctx, resp.id.with("knob"), *on, 22.0);
    let (h, pr) = hover_fx(ctx, &resp);
    let p = ui.painter();
    let track = mix(mix(Color32::from_gray(0x33), Color32::from_gray(0x3d), h), ACCENT, t);
    p.rect_filled(r, r.height() / 2.0, track);
    let rad = r.height() / 2.0 - 2.5;
    // the knob stretches a little while pressed, like iOS
    let stretch = pr * 4.0;
    let x0 = r.min.x + 2.5 + rad;
    let x1 = r.max.x - 2.5 - rad;
    let cx = x0 + (x1 - x0) * t;
    let kr = Rect::from_center_size(pos2(cx, r.center().y), vec2(rad * 2.0 + stretch, rad * 2.0));
    let kr = kr.translate(vec2(if *on { -stretch / 2.0 } else { stretch / 2.0 }, 0.0));
    p.rect_filled(kr.translate(vec2(0.0, 1.0)), rad, Color32::from_black_alpha(60));
    p.rect_filled(kr, rad, Color32::WHITE);
    if resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Segmented control with a sliding thumb; returns true when the pick changed
pub fn segmented(ui: &mut Ui, r: Rect, opts: &[&str], sel: &mut usize, id: egui::Id) -> bool {
    let p = ui.painter().clone();
    p.rect_filled(r, 8.0, well());
    p.rect_stroke(r, 8.0, Stroke::new(1.0_f32, border()));
    let n = opts.len().max(1) as f32;
    let sw = (r.width() - 4.0) / n;
    let tx = glide(ui.ctx(), id.with("thumb"), *sel as f32, 16.0);
    let thumb = Rect::from_min_size(pos2(r.min.x + 2.0 + tx * sw, r.min.y + 2.0), vec2(sw, r.height() - 4.0));
    p.rect_filled(thumb.translate(vec2(0.0, 1.0)), 6.0, Color32::from_black_alpha(50));
    p.rect_filled(thumb, 6.0, Color32::from_gray(0x2e));
    let mut changed = false;
    for (i, o) in opts.iter().enumerate() {
        let cr = Rect::from_min_size(pos2(r.min.x + 2.0 + i as f32 * sw, r.min.y), vec2(sw, r.height()));
        let resp = ui.interact(cr, id.with(i), Sense::click());
        let (h, _) = hover_fx(ui.ctx(), &resp);
        let on = (tx - i as f32).abs() < 0.5;
        let col = if on { hi() } else { mix(txt_faint(), txt(), h) };
        p.text(cr.center(), Align2::CENTER_CENTER, *o, sb(11.5), col);
        if resp.clicked() && *sel != i {
            *sel = i;
            changed = true;
        }
    }
    changed
}

/// A thin rounded meter bar
pub fn meter(p: &Painter, r: Rect, frac: f32, col: Color32) {
    p.rect_filled(r, r.height() / 2.0, wa(12));
    let f = frac.clamp(0.0, 1.0);
    if f > 0.0 {
        let w = (r.width() * f).max(r.height());
        p.rect_filled(Rect::from_min_size(r.min, vec2(w, r.height())), r.height() / 2.0, col);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Dashboard,
    Processes,
    Cpu,
    Memory,
    Disk,
    Network,
    Gpu,
    Sensors,
    Search,
    Settings,
}

/// Small line icons painted from shapes (18 px box around `c`)
pub fn icon(p: &Painter, kind: Icon, c: Pos2, col: Color32) {
    let s = Stroke::new(1.4_f32, col);
    match kind {
        Icon::Dashboard => {
            for (dx, dy) in [(-4.5, -4.5), (4.5, -4.5), (-4.5, 4.5), (4.5, 4.5)] {
                p.rect_stroke(Rect::from_center_size(c + vec2(dx, dy), vec2(6.5, 6.5)), 2.0, s);
            }
        }
        Icon::Processes => {
            for k in [-5.0, 0.0, 5.0] {
                p.circle_filled(c + vec2(-6.0, k), 1.3, col);
                p.line_segment([c + vec2(-2.5, k), c + vec2(7.0, k)], s);
            }
        }
        Icon::Cpu => {
            p.rect_stroke(Rect::from_center_size(c, vec2(11.0, 11.0)), 2.5, s);
            p.rect_filled(Rect::from_center_size(c, vec2(4.0, 4.0)), 1.0, col);
            for k in [-3.0, 0.0, 3.0] {
                p.line_segment([c + vec2(k, -5.5), c + vec2(k, -8.0)], s);
                p.line_segment([c + vec2(k, 5.5), c + vec2(k, 8.0)], s);
                p.line_segment([c + vec2(-5.5, k), c + vec2(-8.0, k)], s);
                p.line_segment([c + vec2(5.5, k), c + vec2(8.0, k)], s);
            }
        }
        Icon::Memory => {
            p.rect_stroke(Rect::from_center_size(c + vec2(0.0, -1.0), vec2(16.0, 9.0)), 2.0, s);
            for k in [-4.5, -1.5, 1.5, 4.5] {
                p.rect_filled(Rect::from_center_size(c + vec2(k, -1.0), vec2(1.6, 4.0)), 0.5, col);
            }
            for k in [-6.0, -3.0, 0.0, 3.0, 6.0] {
                p.line_segment([c + vec2(k, 3.5), c + vec2(k, 6.5)], s);
            }
        }
        Icon::Disk => {
            p.rect_stroke(Rect::from_center_size(c, vec2(15.0, 12.0)), 2.5, s);
            p.line_segment([c + vec2(-7.5, 2.0), c + vec2(7.5, 2.0)], s);
            p.circle_filled(c + vec2(4.5, 4.0), 1.1, col);
        }
        Icon::Network => {
            // a globe: circle, equator and two meridians
            p.circle_stroke(c, 7.0, s);
            p.line_segment([c + vec2(-7.0, 0.0), c + vec2(7.0, 0.0)], s);
            let mer: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI * (i as f32 / 12.0 - 0.5);
                    c + vec2(a.cos() * 3.2, a.sin() * 7.0)
                })
                .collect();
            p.add(Shape::line(mer.iter().map(|q| pos2(2.0 * c.x - q.x, q.y)).collect(), s));
            p.add(Shape::line(mer, s));
        }
        Icon::Gpu => {
            p.rect_stroke(Rect::from_center_size(c + vec2(1.0, 0.0), vec2(14.0, 10.0)), 2.0, s);
            p.circle_stroke(c + vec2(1.0, 0.0), 2.8, s);
            p.line_segment([c + vec2(-6.0, -5.0), c + vec2(-8.0, -5.0)], s);
            p.line_segment([c + vec2(-8.0, -5.0), c + vec2(-8.0, 7.0)], s);
        }
        Icon::Sensors => {
            p.line_segment([c + vec2(0.0, -7.0), c + vec2(0.0, 2.5)], Stroke::new(3.6_f32, col));
            p.line_segment([c + vec2(0.0, -6.5), c + vec2(0.0, 2.5)], Stroke::new(1.6_f32, chrome()));
            p.circle_filled(c + vec2(0.0, 4.5), 3.6, col);
            p.line_segment([c + vec2(0.0, -1.0), c + vec2(0.0, 4.0)], Stroke::new(1.6_f32, col));
        }
        Icon::Search => {
            p.circle_stroke(c + vec2(-1.5, -1.5), 4.6, s);
            p.line_segment([c + vec2(2.0, 2.0), c + vec2(5.5, 5.5)], Stroke::new(1.6_f32, col));
        }
        Icon::Settings => {
            p.circle_stroke(c, 2.4, s);
            p.circle_stroke(c, 5.6, s);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * 5.6, c + d * 8.0], Stroke::new(2.4_f32, col));
            }
        }
    }
}

/// The Purgeon mark (assets/purgeon-mark-white.svg): a white chip outline with pins and a
/// blue core, drawn from shapes so it stays sharp at any scale. `size` is the mark's width.
pub fn logo(p: &Painter, c: Pos2, size: f32) {
    // the SVG's viewBox spans 16..84, centred on 50
    let k = size / 68.0;
    let at = |x: f32, y: f32| c + vec2((x - 50.0) * k, (y - 50.0) * k);
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(x0, y0), at(x1, y1));
    p.rect_stroke(rect(29.0, 29.0, 71.0, 71.0), 11.0 * k, Stroke::new(7.0 * k, Color32::WHITE));
    // pins: round-capped strokes 5.5 wide
    let h = 2.75;
    for v in [43.0, 57.0] {
        for (a0, a1) in [(20.0, 29.0), (71.0, 80.0)] {
            p.rect_filled(rect(v - h, a0 - h, v + h, a1 + h), h * k, Color32::WHITE);
            p.rect_filled(rect(a0 - h, v - h, a1 + h, v + h), h * k, Color32::WHITE);
        }
    }
    p.rect_filled(rect(41.0, 41.0, 59.0, 59.0), 5.0 * k, ACCENT);
}

/// A letter avatar for a process (stable hue from its name)
pub fn avatar(p: &Painter, r: Rect, name: &str) {
    let h = name.bytes().fold(2166136261u32, |a, b| (a ^ b.to_ascii_lowercase() as u32).wrapping_mul(16777619));
    let hue = (h % 360) as f32 / 360.0;
    let col: Color32 = egui::ecolor::Hsva::new(hue, 0.45, 0.55, 1.0).into();
    p.rect_filled(r, 5.0, col.gamma_multiply(0.85));
    let ch = name.chars().next().unwrap_or('?').to_ascii_uppercase();
    p.text(r.center() + vec2(0.0, 0.5), Align2::CENTER_CENTER, ch, sb(10.5), Color32::WHITE);
}
