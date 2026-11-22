//! Live line graphs: soft gradient fill, a crisp line, a short scroll-in when a sample
//! arrives, and a hover readout.

use crate::sys::HIST;
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

pub struct Series<'a> {
    pub data: &'a [f32],
    pub col: Color32,
    pub fill: bool,
    pub width: f32,
}

pub struct Graph<'a> {
    pub series: &'a [Series<'a>],
    pub max: f32,
    /// 0..1, how far the newest sample has slid in
    pub scroll: f32,
    pub grid: bool,
    pub hover: Option<Pos2>,
    pub interval: f32,
    pub label: &'a dyn Fn(&[f32]) -> String,
}

pub fn paint(p: &Painter, rect: Rect, g: &Graph) {
    let p = p.with_clip_rect(rect.intersect(p.clip_rect()));
    if g.grid {
        for k in 1..4 {
            let y = rect.min.y + rect.height() * k as f32 / 4.0;
            p.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], Stroke::new(1.0_f32, wa(8)));
        }
    }
    let dx = rect.width() / (HIST - 1) as f32;
    let shift = (1.0 - g.scroll) * dx;
    let xof = |i: usize, n: usize| rect.max.x - (n - 1 - i) as f32 * dx + shift;
    let yof = |v: f32| rect.max.y - 1.0 - (v / g.max).clamp(0.0, 1.0) * (rect.height() - 2.0);
    for s in g.series {
        let n = s.data.len();
        if n < 2 {
            continue;
        }
        let pts: Vec<Pos2> = s.data.iter().enumerate().map(|(i, v)| pos2(xof(i, n), yof(*v))).collect();
        if s.fill {
            let mut m = egui::epaint::Mesh::default();
            let top = s.col.gamma_multiply(0.32);
            let low = s.col.gamma_multiply(0.02);
            for q in &pts {
                let k = ((q.y - rect.min.y) / rect.height()).clamp(0.0, 1.0);
                m.colored_vertex(*q, mix(top, low, k * 0.6));
                m.colored_vertex(pos2(q.x, rect.max.y), low);
            }
            for i in 1..pts.len() as u32 {
                let j = i * 2;
                m.add_triangle(j - 2, j - 1, j);
                m.add_triangle(j - 1, j + 1, j);
            }
            p.add(Shape::mesh(m));
        }
        p.add(Shape::line(pts, Stroke::new(s.width, s.col)));
    }

    // Hover readout
    let Some(hp) = g.hover.filter(|q| rect.contains(*q)) else { return };
    let Some(first) = g.series.first() else { return };
    let n = first.data.len();
    if n == 0 {
        return;
    }
    let back = ((rect.max.x + shift - hp.x) / dx).round().max(0.0) as usize;
    let i = (n - 1).saturating_sub(back);
    let x = xof(i, n);
    p.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], Stroke::new(1.0_f32, wa(40)));
    let vals: Vec<f32> = g
        .series
        .iter()
        .map(|s| s.data.len().checked_sub(n - i).and_then(|k| s.data.get(k)).copied().unwrap_or(0.0))
        .collect();
    for (s, v) in g.series.iter().zip(&vals) {
        let q = pos2(x, yof(*v));
        p.circle_filled(q, 4.0, card());
        p.circle_filled(q, 3.0, s.col);
    }
    let text = format!("{}  ·  {}", (g.label)(&vals), crate::fmt::ago((n - 1 - i) as f32 * g.interval));
    let gal = p.layout_no_wrap(text, sb(11.5), txt());
    let w = gal.size().x + 16.0;
    let bx = (x - w / 2.0).clamp(rect.min.x + 2.0, rect.max.x - w - 2.0);
    let br = Rect::from_min_size(pos2(bx, rect.min.y + 6.0), vec2(w, 22.0));
    p.rect_filled(br.translate(vec2(0.0, 2.0)), 7.0, Color32::from_black_alpha(80));
    p.rect_filled(br, 7.0, Color32::from_gray(0x2a));
    p.galley(pos2(br.min.x + 8.0, br.center().y - gal.size().y / 2.0), gal, txt());
}
