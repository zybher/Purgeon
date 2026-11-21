//! The window: title bar, sidebar, and the page area.

mod graph;
pub mod overlay;
mod pages;
mod icons;
mod morph;
mod table;
mod tools;
mod widgets;

use crate::anim::*;
use crate::sys::{self, Snapshot};
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, CursorIcon, PointerButton, Rect, ResizeDirection, Rounding, Sense, Shape, Stroke, ViewportCommand};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use widgets::Icon;

const TITLE_H: f32 = 40.0;
const SIDE_W: f32 = 212.0;
/// Update speeds; 0 ms = paused
const INTERVALS: [(&str, u32); 4] = [("0.5s", 500), ("1s", 1000), ("2s", 2000), ("Pause", 0)];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    Processes,
    Cpu,
    Memory,
    Disk,
    Network,
    Gpu,
    Sensors,
}

const NAV: [(Page, Icon, &str, bool); 8] = [
    (Page::Overview, Icon::Overview, "Overview", true),
    (Page::Processes, Icon::Processes, "Processes", true),
    (Page::Cpu, Icon::Cpu, "CPU", true),
    (Page::Memory, Icon::Memory, "Memory", true),
    (Page::Disk, Icon::Disk, "Disk", true),
    (Page::Network, Icon::Network, "Network", true),
    (Page::Gpu, Icon::Gpu, "GPU", true),
    (Page::Sensors, Icon::Sensors, "Sensors", true),
];

pub struct App {
    shared: Arc<sys::Shared>,
    snap: Option<Arc<Snapshot>>,
    page: Page,
    page_at: Instant,
    interval: usize,
    dwm_done: bool,
    table: table::TableState,
    shot: Option<crate::shot::Shot>,
    trimmed: bool,
    was_focused: bool,
    was_minimized: bool,
    /// Closed to the tray; the window hides on the next frame
    hide_next: bool,
    /// The UAC prompt was declined: show a dismissible note
    uac_note: bool,
    /// Snapshots seen, to evict dead programs' icons now and then
    snaps: u64,
    /// The overlay growing into the window, while it plays
    morph: Option<morph::Run>,
    /// Screenshot mode: the morph check was started
    morph_tested: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        let reduced = crate::platform::reduce_motion();
        crate::anim::set_reduced(reduced);
        setup_style(&cc.egui_ctx, reduced);
        // Alerts and the tray's hover text follow every sample, also while hidden in the tray
        let mut watch = crate::alerts::Watch::default();
        let mut tip = String::new();
        let shared = sys::start(cc.egui_ctx.clone(), move |s| {
            crate::boost::step(s);
            crate::apps::step(s);
            if let Some(a) = watch.check(s) {
                crate::tray::notify(&a.title, &a.body, true);
            }
            let t = format!("{} · CPU {:.0}% · RAM {:.0}%", crate::TITLE, s.cpu, s.mem.used_frac() * 100.0);
            if t != tip {
                crate::tray::set_tip(&t);
                tip = t;
            }
        });
        crate::tray::start(shared.clone(), cc.egui_ctx.clone());
        App {
            shared,
            snap: None,
            page: start_page(),
            page_at: Instant::now(),
            interval: 1,
            dwm_done: false,
            table: table::TableState::from_args(),
            shot: crate::shot::Shot::from_env(),
            trimmed: false,
            was_focused: true,
            was_minimized: false,
            hide_next: false,
            uac_note: false,
            snaps: 0,
            morph: None,
            morph_tested: false,
        }
    }

    fn go(&mut self, p: Page) {
        if self.page != p {
            self.page = p;
            self.page_at = Instant::now();
        }
    }

    fn titlebar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, rect: Rect) {
        let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let p = ui.painter().clone();
        let cy = rect.center().y;

        // The whole bar drags the window; buttons allocated after it sit on top
        let drag = ui.interact(rect, egui::Id::new("title_drag"), Sense::click_and_drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_max));
        }

        widgets::logo(&p, pos2(rect.min.x + 26.0, cy), 19.0);
        p.text(pos2(rect.min.x + 44.0, cy), Align2::LEFT_CENTER, "Purgeon", sb(13.0), txt());

        // Live readout in the middle of the bar
        if let Some(s) = &self.snap {
            let paused = self.shared.interval_ms.load(Ordering::Relaxed) == 0;
            let t = format!(
                "CPU {:.0}%   ·   RAM {:.0}%   ·   {} processes{}",
                s.cpu,
                s.mem.used_frac() * 100.0,
                s.nprocs,
                if paused { "   ·   Paused" } else { "" }
            );
            p.text(pos2(rect.center().x, cy), Align2::CENTER_CENTER, t, reg(12.0), txt_faint());
        }

        let (bw, bh) = (46.0, rect.height());
        let x0 = rect.max.x - bw * 3.0;
        for k in 0..3 {
            let r = Rect::from_min_size(pos2(x0 + bw * k as f32, rect.min.y), vec2(bw, bh));
            let resp = ui.interact(r, egui::Id::new(("winbtn", k)), Sense::click());
            let (h, pr) = hover_fx(ctx, &resp);
            let ic = Stroke::new(1.1_f32, mix(txt_dim(), hi(), h));
            let c = r.center();
            match k {
                0 => {
                    p.rect_filled(r, 0.0, wa((h * 18.0 + pr * 16.0) as u8));
                    p.line_segment([pos2(c.x - 5.0, c.y), pos2(c.x + 5.0, c.y)], ic);
                    if resp.clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                    }
                }
                1 => {
                    p.rect_filled(r, 0.0, wa((h * 18.0 + pr * 16.0) as u8));
                    if is_max {
                        p.rect_stroke(Rect::from_center_size(c + vec2(1.5, -1.5), vec2(8.0, 8.0)), 1.0, ic);
                        p.rect_filled(Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(8.0, 8.0)), 1.0, mix(chrome(), Color32::from_gray(0x2d), h));
                        p.rect_stroke(Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(8.0, 8.0)), 1.0, ic);
                    } else {
                        p.rect_stroke(Rect::from_center_size(c, vec2(10.0, 10.0)), 1.5, ic);
                    }
                    if resp.clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_max));
                    }
                }
                _ => {
                    p.rect_filled(r, 0.0, DANGER.gamma_multiply(h * (1.0 - pr * 0.25)));
                    p.line_segment([c + vec2(-4.5, -4.5), c + vec2(4.5, 4.5)], ic);
                    p.line_segment([c + vec2(4.5, -4.5), c + vec2(-4.5, 4.5)], ic);
                    if resp.clicked() {
                        // to the tray when there is one (see `update`), else quits
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                }
            }
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, rect: Rect) {
        let p = ui.painter().clone();
        p.text(pos2(rect.min.x + 24.0, rect.min.y + 18.0), Align2::LEFT_CENTER, "MONITOR", sb(10.5), txt_faint());
        let (item_h, pitch) = (34.0, 38.0);
        let top = rect.min.y + 32.0;
        let sel_i = NAV.iter().position(|n| n.0 == self.page).unwrap_or(0);
        // The selection pill glides between items
        let gy = glide(ctx, egui::Id::new("nav_pill"), top + sel_i as f32 * pitch, 18.0);
        let pill = Rect::from_min_size(pos2(rect.min.x + 12.0, gy), vec2(rect.width() - 24.0, item_h));
        p.rect_filled(pill, 9.0, Color32::from_gray(0x23));
        p.rect_stroke(pill, 9.0, Stroke::new(1.0_f32, Color32::from_gray(0x2c)));

        let mut go = None;
        for (i, (page, ic, label, ready)) in NAV.iter().enumerate() {
            let r = Rect::from_min_size(pos2(rect.min.x + 12.0, top + i as f32 * pitch), vec2(rect.width() - 24.0, item_h));
            let resp = ui.interact(r, egui::Id::new(("nav", i)), Sense::click());
            let (h, pr) = hover_fx(ctx, &resp);
            let on = *page == self.page;
            let onk = ease(ctx, resp.id.with("on"), on);
            if !on {
                p.rect_filled(r.shrink(pr * 1.5), 9.0, wa((h * 10.0) as u8));
            }
            let base = if *ready { txt_dim() } else { txt_faint() };
            let col = mix(mix(base, txt(), h), hi(), onk);
            let icol = mix(mix(base, txt(), h), ACCENT, onk);
            // the icon nudges right a touch on hover
            widgets::icon(&p, *ic, pos2(r.min.x + 22.0 + h * 1.5, r.center().y), icol);
            p.text(pos2(r.min.x + 42.0 + h * 1.5, r.center().y), Align2::LEFT_CENTER, *label, sb(13.0), col);
            if !*ready {
                let br = Rect::from_min_size(pos2(r.max.x - 46.0, r.center().y - 9.0), vec2(36.0, 18.0));
                p.rect_filled(br, 9.0, wa(10));
                p.text(br.center(), Align2::CENTER_CENTER, "Soon", sb(10.0), txt_faint());
            }
            if resp.clicked() {
                go = Some(*page);
            }
            if resp.hovered() {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
            }
        }
        if let Some(pg) = go {
            self.go(pg);
        }

        // Update speed
        let by = rect.max.y - 96.0;
        p.text(pos2(rect.min.x + 24.0, by), Align2::LEFT_CENTER, "UPDATE SPEED", sb(10.5), txt_faint());
        let sr = Rect::from_min_size(pos2(rect.min.x + 16.0, by + 14.0), vec2(rect.width() - 32.0, 28.0));
        let names: Vec<&str> = INTERVALS.iter().map(|i| i.0).collect();
        if widgets::segmented(ui, sr, &names, &mut self.interval, egui::Id::new("speed")) {
            self.shared.interval_ms.store(INTERVALS[self.interval].1, Ordering::Relaxed);
        }

        // Elevation status
        let admin = self.snap.as_ref().is_some_and(|s| s.info.admin);
        let ay = rect.max.y - 26.0;
        p.circle_filled(pos2(rect.min.x + 27.0, ay), 3.5, if admin { GPU_COL } else { WARN_COL });
        let (t, tip) = if admin {
            ("Administrator", "Running elevated: every reading is available.")
        } else {
            ("Standard user", "Some readings (memory lists, protected processes, sensors) need Purgeon to run as administrator.")
        };
        if admin {
            let lr = p.text(pos2(rect.min.x + 37.0, ay), Align2::LEFT_CENTER, t, reg(11.5), txt_faint());
            ui.interact(lr.expand(4.0), egui::Id::new("admin_tip"), Sense::hover()).on_hover_text(tip);
        } else {
            // "Standard user · Elevate": restarts through the UAC prompt
            let br = Rect::from_min_max(pos2(rect.min.x + 16.0, ay - 12.0), pos2(rect.max.x - 16.0, ay + 12.0));
            let resp = ui.interact(br, egui::Id::new("admin_btn"), Sense::click());
            let (h, pr) = hover_fx(ctx, &resp);
            p.rect_filled(br.shrink(pr * 1.5), 8.0, wa((h * 12.0) as u8));
            p.text(pos2(rect.min.x + 37.0, ay), Align2::LEFT_CENTER, t, reg(11.5), txt_faint());
            p.text(pos2(br.max.x - 10.0, ay), Align2::RIGHT_CENTER, "Elevate", sb(11.5), mix(ACCENT, hi(), h * 0.3));
            if resp.hovered() {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
            }
            let resp = resp.on_hover_text("Restart Purgeon as administrator to unlock per-process physical disk activity, memory lists and protected processes.");
            if resp.clicked() {
                // the elevated copy opens on the same page with the same table sort
                let mut state = vec![format!("--page={}", page_name(self.page))];
                state.extend(self.table.state_args());
                if crate::platform::relaunch_as_admin(&state) {
                    crate::quit();
                }
                self.uac_note = true;
            }
        }
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        crate::boost::on_quit();
        // kernel trace sessions outlive the process unless stopped
        crate::sys::etw::stop();
    }

    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        chrome().to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(s) = self.shared.take() {
            self.snaps += 1;
            if self.snaps % 30 == 0 {
                let live: std::collections::HashSet<&str> = s.procs.iter().filter_map(|p| p.path.as_deref()).collect();
                icons::retain(ctx, &live);
            }
            self.snap = Some(s);
        }
        overlay::show(ctx, self.snap.as_deref());
        if self.shared.hidden.load(Ordering::Relaxed) {
            // In the tray: draw nothing and ask for nothing, until the tray shows the window
            return;
        }
        // Closing (the X, Alt+F4, the taskbar) hides Purgeon to the tray; it keeps watching
        // without drawing. The tray icon's Quit ends it.
        if ctx.input(|i| i.viewport().close_requested()) && crate::tray::active() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            // Put away on the next frame, once the close is cancelled
            self.hide_next = true;
            ctx.request_repaint();
            return;
        }
        if self.hide_next {
            self.hide_next = false;
            crate::platform::hide_window();
            self.shared.hidden.store(true, Ordering::Relaxed);
            self.shared.visible.store(false, Ordering::Relaxed);
            self.was_minimized = true;
            crate::platform::trim_memory();
            if !crate::settings::flag("tray_note", false) {
                crate::settings::set_flag("tray_note", true);
                crate::tray::notify("Purgeon is still running", "It's keeping an eye on your PC from the tray and will tell you if something's wrong. Right-click its icon to quit.", false);
            }
            // Draw nothing this frame, so nothing asks for another one
            return;
        }
        let hidden = self.shared.hidden.load(Ordering::Relaxed);
        // Minimized or in the tray: the sampler keeps going but stops waking the UI
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false)) || hidden;
        self.shared.visible.store(!minimized, Ordering::Relaxed);
        // Give back memory only needed while starting (the GPU sensor library alone takes ~19 MB
        // to start), and again whenever the window loses focus or is minimized. The startup trim
        // runs even if focus was lost first, e.g. when Windows opens the app behind other windows.
        let focused = ctx.input(|i| i.focused);
        let startup = !self.trimmed && self.page_at.elapsed().as_secs_f32() > 2.0 && self.snap.is_some();
        if startup || (self.was_focused && !focused) || (minimized && !self.was_minimized) {
            self.trimmed |= startup;
            crate::platform::trim_memory();
        }
        self.was_focused = focused;
        self.was_minimized = minimized;
        if !self.dwm_done {
            self.dwm_done = crate::platform::style_window();
            if !self.dwm_done {
                ctx.request_repaint();
            }
        }
        // How far the newest sample has slid into the graphs
        let scroll = if crate::anim::reduced() { 1.0 } else { self.snap.as_ref().map_or(1.0, |s| (s.at.elapsed().as_secs_f32() / 0.35).min(1.0)) };
        if scroll < 1.0 {
            ctx.request_repaint();
        }
        let scroll = egui::emath::easing::cubic_out(scroll);

        // The overlay's expand button: the bar grows into this window, on the Overview page
        if self.morph.is_none() {
            if self.shot.is_some() && !self.morph_tested && self.snap.is_some() {
                self.morph_tested = true;
                morph::test_begin(ctx, self.snap.as_deref());
            }
            if let Some(m) = morph::take(ctx) {
                self.morph = Some(m);
                self.page = Page::Overview;
                // the page rises in once the bar has nearly filled the window
                self.page_at = Instant::now() + std::time::Duration::from_millis(230);
            }
        }
        if self.morph.as_ref().is_some_and(|m| m.done()) {
            // drops the copy of the screen
            self.morph = None;
            crate::platform::trim_memory();
        }
        if self.morph.is_some() {
            ctx.request_repaint();
        }

        let fill = if self.morph.is_some() { Color32::TRANSPARENT } else { chrome() };
        egui::CentralPanel::default().frame(egui::Frame::none().fill(fill)).show(ctx, |ui| {
            let full = ui.max_rect();
            if let Some(m) = &self.morph {
                m.paint_back(ui.painter(), full);
                ui.set_clip_rect(m.shape(full));
                ui.multiply_opacity(m.content());
            }
            let title = Rect::from_min_max(full.min, pos2(full.max.x, full.min.y + TITLE_H));
            let side = Rect::from_min_max(pos2(full.min.x, title.max.y), pos2(full.min.x + SIDE_W, full.max.y));
            let content = Rect::from_min_max(pos2(side.max.x, title.max.y), full.max);
            self.titlebar(ui, ctx, title);
            self.sidebar(ui, ctx, side);

            ui.painter().rect_filled(content, Rounding { nw: 14.0, ..Default::default() }, bg());
            ui.painter().add(Shape::line(
                vec![pos2(content.min.x, content.max.y), pos2(content.min.x, content.min.y + 14.0)],
                Stroke::new(1.0_f32, Color32::from_gray(0x1d)),
            ));

            // Page entrance: fade in and rise a few points
            let t = if crate::anim::reduced() { 1.0 } else { egui::emath::easing::cubic_out((self.page_at.elapsed().as_secs_f32() / 0.28).min(1.0)) };
            if t < 1.0 {
                ctx.request_repaint();
            }
            let inner = Rect::from_min_max(content.min + vec2(28.0, 22.0 + 10.0 * (1.0 - t)), content.max - vec2(28.0, 0.0));
            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(inner).id_salt(("page", self.page as u8)));
            cui.set_clip_rect(content.shrink(1.0).intersect(ui.clip_rect()));
            cui.multiply_opacity(t);
            match &self.snap {
                None => {
                    cui.painter().text(content.center(), Align2::CENTER_CENTER, "Reading the system…", reg(14.0), txt_faint());
                }
                Some(s) => {
                    let s = s.clone();
                    let page = self.page;
                    let nav = match page {
                        Page::Overview => pages::overview(&mut cui, &s, scroll),
                        Page::Processes => {
                            self.table.show(&mut cui, &s);
                            None
                        }
                        Page::Cpu => pages::cpu(&mut cui, &s, scroll),
                        Page::Memory => pages::memory(&mut cui, &s, scroll),
                        Page::Disk => pages::disk(&mut cui, &s, scroll),
                        Page::Network => pages::network(&mut cui, &s, scroll),
                        Page::Gpu => pages::gpu(&mut cui, &s, scroll),
                        Page::Sensors => pages::sensors(&mut cui, &s, scroll),
                    };
                    if let Some(pg) = nav {
                        self.go(pg);
                    }
                }
            }
            if self.uac_note {
                self.uac_note = uac_note(ctx, content);
            }
        });
        if let Some(m) = &self.morph {
            m.paint_front(ctx, ctx.screen_rect());
        }
        resize_edges(ctx);
        if let Some(sh) = self.shot.as_mut().filter(|_| !hidden) {
            sh.tick(ctx);
        }
    }
}

/// Borderless window: resize from the edges and corners
fn resize_edges(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let r = ctx.screen_rect();
    let (e, c) = (5.0, 12.0);
    let (l, rt, t, b) = (pos.x <= r.min.x + e, pos.x >= r.max.x - e, pos.y <= r.min.y + e, pos.y >= r.max.y - e);
    let (lc, rc, tc, bc) = (pos.x <= r.min.x + c, pos.x >= r.max.x - c, pos.y <= r.min.y + c, pos.y >= r.max.y - c);
    use ResizeDirection as D;
    let dir = if (t && lc) || (l && tc) {
        Some((D::NorthWest, CursorIcon::ResizeNorthWest))
    } else if (t && rc) || (rt && tc) {
        Some((D::NorthEast, CursorIcon::ResizeNorthEast))
    } else if (b && lc) || (l && bc) {
        Some((D::SouthWest, CursorIcon::ResizeSouthWest))
    } else if (b && rc) || (rt && bc) {
        Some((D::SouthEast, CursorIcon::ResizeSouthEast))
    } else if t {
        Some((D::North, CursorIcon::ResizeVertical))
    } else if b {
        Some((D::South, CursorIcon::ResizeVertical))
    } else if l {
        Some((D::West, CursorIcon::ResizeHorizontal))
    } else if rt {
        Some((D::East, CursorIcon::ResizeHorizontal))
    } else {
        None
    };
    if let Some((d, cur)) = dir {
        ctx.set_cursor_icon(cur);
        if ctx.input(|i| i.pointer.button_pressed(PointerButton::Primary)) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(d));
        }
    }
}

/// Shown after the UAC prompt was declined. Returns false once dismissed.
fn uac_note(ctx: &egui::Context, content: Rect) -> bool {
    let mut keep = true;
    let w = (content.width() - 56.0).min(520.0);
    let r = Rect::from_center_size(pos2(content.center().x, content.max.y - 34.0), vec2(w, 40.0));
    egui::Area::new(egui::Id::new("uac_note")).order(egui::Order::Foreground).fixed_pos(r.min).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(r.size(), Sense::hover());
        let p = ui.painter();
        p.rect_filled(r.translate(vec2(0.0, 2.0)), 11.0, Color32::from_black_alpha(70));
        p.rect_filled(r, 11.0, Color32::from_gray(0x26));
        p.rect_stroke(r, 11.0, Stroke::new(1.0_f32, border()));
        p.circle_filled(pos2(r.min.x + 18.0, r.center().y), 3.5, WARN_COL);
        widgets::fit(
            p,
            pos2(r.min.x + 30.0, r.center().y),
            Align2::LEFT_CENTER,
            "Administrator mode wasn't started. Purgeon keeps running as a standard user.",
            reg(12.5),
            r.width() - 72.0,
            txt(),
        );
        let cr = Rect::from_center_size(pos2(r.max.x - 20.0, r.center().y), vec2(24.0, 24.0));
        let resp = ui.interact(cr, egui::Id::new("uac_note_x"), Sense::click());
        let (h, _) = hover_fx(ctx, &resp);
        p.circle_filled(cr.center(), 11.0, wa((h * 20.0) as u8));
        let c = cr.center();
        let st = Stroke::new(1.2_f32, mix(txt_dim(), hi(), h));
        p.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], st);
        p.line_segment([c + vec2(3.5, -3.5), c + vec2(-3.5, 3.5)], st);
        if resp.clicked() {
            keep = false;
        }
    });
    keep
}

fn setup_style(ctx: &egui::Context, reduced: bool) {
    let mut fonts = egui::FontDefinitions::default();
    // Segoe UI Variable (Windows 11) where present, else Segoe UI. Both draw digits with equal
    // widths by default, so numbers don't jiggle as they change.
    let regular = std::fs::read("C:\\Windows\\Fonts\\SegUIVar.ttf").or_else(|_| std::fs::read("C:\\Windows\\Fonts\\segoeui.ttf"));
    if let Ok(bytes) = regular {
        fonts.font_data.insert("segoe".into(), egui::FontData::from_owned(bytes));
        if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            f.insert(0, "segoe".into());
        }
    }
    let mut sb_list = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    if let Ok(bytes) = std::fs::read("C:\\Windows\\Fonts\\seguisb.ttf") {
        fonts.font_data.insert("segoe_sb".into(), egui::FontData::from_owned(bytes));
        sb_list.insert(0, "segoe_sb".into());
    }
    fonts.families.insert(egui::FontFamily::Name("sb".into()), sb_list);
    ctx.set_fonts(fonts);

    let mut st = (*ctx.style()).clone();
    st.visuals = egui::Visuals::dark();
    let v = &mut st.visuals;
    v.panel_fill = chrome();
    v.window_fill = Color32::from_gray(0x1e);
    v.window_stroke = Stroke::new(1.0_f32, border());
    v.window_rounding = Rounding::same(10.0);
    v.menu_rounding = Rounding::same(10.0);
    let shadow = egui::epaint::Shadow { offset: vec2(0.0, 3.0), blur: 8.0, spread: 1.0, color: Color32::from_black_alpha(90) };
    v.window_shadow = shadow;
    v.popup_shadow = shadow;
    v.extreme_bg_color = well();
    v.selection.bg_fill = ACCENT.gamma_multiply(0.6);
    v.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(147, 197, 253));
    v.override_text_color = Some(txt());
    v.text_cursor.stroke = Stroke::new(1.5_f32, ACCENT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, border());
    v.widgets.inactive.bg_fill = Color32::from_gray(0x2a);
    v.widgets.inactive.weak_bg_fill = Color32::from_gray(0x2a);
    v.widgets.hovered.bg_fill = Color32::from_gray(0x36);
    v.widgets.hovered.weak_bg_fill = Color32::from_gray(0x36);
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.active.bg_fill = Color32::from_gray(0x40);
    v.widgets.active.weak_bg_fill = Color32::from_gray(0x40);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.rounding = Rounding::same(7.0);
    }
    st.spacing.button_padding = vec2(10.0, 5.0);
    st.spacing.item_spacing = vec2(8.0, 4.0);
    st.spacing.menu_margin = egui::Margin::same(6.0);
    st.spacing.window_margin = egui::Margin::same(12.0);
    st.spacing.scroll = egui::style::ScrollStyle::thin();
    for (style, size) in [(egui::TextStyle::Body, 13.0), (egui::TextStyle::Button, 13.0), (egui::TextStyle::Small, 11.0), (egui::TextStyle::Heading, 20.0)] {
        st.text_styles.insert(style, egui::FontId::proportional(size));
    }
    // Reduce motion (Windows "Animation effects" off): no animated transitions at all
    st.animation_time = if reduced { 0.0 } else { 0.14 };
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|s| *s = st.clone());
}

fn page_name(p: Page) -> &'static str {
    match p {
        Page::Overview => "overview",
        Page::Processes => "processes",
        Page::Cpu => "cpu",
        Page::Memory => "memory",
        Page::Disk => "disk",
        Page::Network => "network",
        Page::Gpu => "gpu",
        Page::Sensors => "sensors",
    }
}

/// `--page=cpu` (overview, processes, cpu, memory, disk, gpu, sensors) opens on that page
fn start_page() -> Page {
    let arg = std::env::args().find_map(|a| a.strip_prefix("--page=").map(str::to_lowercase));
    match arg.as_deref() {
        Some("processes") => Page::Processes,
        Some("cpu") => Page::Cpu,
        Some("memory") => Page::Memory,
        Some("disk") => Page::Disk,
        Some("network") => Page::Network,
        Some("gpu") => Page::Gpu,
        Some("sensors") => Page::Sensors,
        _ => Page::Overview,
    }
}
