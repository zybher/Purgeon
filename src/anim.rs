//! Animation helpers. egui repaints only while something is moving, so a settled UI
//! costs nothing between data updates.

use eframe::egui::{self, Context, Id, Response};

/// Eased 0..1 value that follows `on`
pub fn ease(ctx: &Context, id: Id, on: bool) -> f32 {
    ctx.animate_bool_with_time_and_easing(id, on, 0.2, egui::emath::easing::cubic_out)
}

/// Critically damped spring toward 0/1 that keeps its velocity, so flicking the pointer
/// in and out never snaps
pub fn spring(ctx: &Context, id: Id, on: bool, omega: f32) -> f32 {
    let target = if on { 1.0 } else { 0.0 };
    let (mut x, mut v): (f32, f32) = ctx.data(|d| d.get_temp(id)).unwrap_or((target, 0.0));
    let dt = ctx.input(|i| i.stable_dt).min(0.05);
    let steps = 4;
    let h = dt / steps as f32;
    for _ in 0..steps {
        let acc = omega * omega * (target - x) - 2.0 * omega * v;
        v += acc * h;
        x += v * h;
    }
    if (target - x).abs() < 0.001 && v.abs() < 0.01 {
        x = target;
        v = 0.0;
    } else {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(id, (x, v)));
    x.clamp(0.0, 1.0)
}

/// A value that glides to its target (exponential approach)
pub fn glide(ctx: &Context, id: Id, target: f32, speed: f32) -> f32 {
    let cur: f32 = ctx.data(|d| d.get_temp(id)).unwrap_or(target);
    let dt = ctx.input(|i| i.stable_dt).min(0.05);
    let mut v = cur + (target - cur) * (1.0 - (-speed * dt).exp());
    if (target - v).abs() <= 0.002 * target.abs().max(1.0) {
        v = target;
    } else {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(id, v));
    v
}

/// Eased (hover, press) amounts for a custom-painted control
pub fn hover_fx(ctx: &Context, resp: &Response) -> (f32, f32) {
    (ease(ctx, resp.id.with("hov"), resp.hovered()), ease(ctx, resp.id.with("prs"), resp.is_pointer_button_down_on()))
}
