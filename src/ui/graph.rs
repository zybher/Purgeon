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
    let value = (g.label)(&vals);
    let ago = crate::fmt::ago((n - 1 - i) as f32 * g.interval);
    let lay = |text: String| p.layout_no_wrap(text, sb(11.5), txt());
    let room = rect.width() - 20.0;
    // A narrow graph puts the time on a second line; narrower still, only the line and dots show
    let mut gal = lay(format!("{value}  ·  {ago}"));
    if gal.size().x > room {
        gal = lay(format!("{value}
{ago}"));
        if gal.size().x > room {
            return;
        }
    }
    let w = gal.size().x + 16.0;
    // not `clamp`: it panics if rounding ever leaves the right limit a hair left of the left one
    let bx = (x - w / 2.0).min(rect.max.x - w - 2.0).max(rect.min.x + 2.0);
    let br = Rect::from_min_size(pos2(bx, rect.min.y + 6.0), vec2(w, (gal.size().y + 6.0).max(22.0)));
    p.rect_filled(br.translate(vec2(0.0, 2.0)), 7.0, Color32::from_black_alpha(80));
    p.rect_filled(br, 7.0, Color32::from_gray(0x2a));
    p.galley(pos2(br.min.x + 8.0, br.center().y - gal.size().y / 2.0), gal, txt());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paints one hovered graph; returns the readout, after checking that it lies inside the graph
    fn readout(ctx: &egui::Context, rect: Rect, at: f32, values: &[f32], label: &dyn Fn(&[f32]) -> String) -> Option<String> {
        let data: Vec<Vec<f32>> = values.iter().map(|v| vec![*v; HIST]).collect();
        let series: Vec<Series> = data.iter().map(|d| Series { data: d, col: txt(), fill: true, width: 1.0 }).collect();
        let hover = Some(pos2(rect.min.x + rect.width() * at, rect.center().y));
        let g = Graph { series: &series, max: 100.0, scroll: 0.5, grid: false, hover, interval: 1.0, label };
        let out = ctx.run(Default::default(), |ctx| {
            paint(&ctx.layer_painter(egui::LayerId::background()), rect, &g);
        });
        out.shapes.iter().find_map(|c| match &c.shape {
            Shape::Text(t) => {
                let r = t.galley.rect.translate(t.pos.to_vec2());
                // half a point of slack: text snaps to whole pixels
                assert!(rect.expand(0.51).contains_rect(r), "{:?} at {r:?} leaves {rect:?}", t.galley.text());
                Some(t.galley.text().to_string())
            }
            _ => None,
        })
    }

    #[test]
    fn hover_readout_survives_any_width() {
        let pct = |v: &[f32]| format!("{:.1}%  /  {:.1}%", v[0], v[1]);
        let rate = |v: &[f32]| format!("R {}  ·  W {}", crate::fmt::rate(v[0] as f64), crate::fmt::rate(v[1] as f64));
        for scale in [1.0_f32, 1.25, 1.5, 2.0] {
            let ctx = egui::Context::default();
            super::super::setup_style(&ctx, true);
            ctx.set_pixels_per_point(scale);
            let _ = ctx.run(Default::default(), |_| {});
            for w in 0..=720 {
                let rect = Rect::from_min_size(pos2(20.3, 20.0), vec2(w as f32 * 0.997, 150.0));
                for at in [0.0, 0.5, 0.999] {
                    let card = readout(&ctx, rect, at, &[100.0, 100.0], &pct);
                    // the dashboard's narrowest cards are about 130 across and still show both numbers
                    if w >= 125 {
                        assert!(card.is_some_and(|t| t.contains("100.0%  /  100.0%")), "no readout at width {w}");
                    }
                    readout(&ctx, rect, at, &[999_900_000.0, 999_900_000.0], &rate);
                }
            }
        }
    }
}
