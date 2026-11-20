//! The window: title bar, sidebar, and the page area.

mod graph;
mod pages;
mod icons;
mod table;
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
const INTERVALS: [(&str, u32); 3] = [("0.5s", 500), ("1s", 1000), ("2s", 2000)];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    Processes,
    Cpu,
    Memory,
    Disk,
    Gpu,
    Sensors,
}

const NAV: [(Page, Icon, &str, bool); 7] = [
    (Page::Overview, Icon::Overview, "Overview", true),
    (Page::Processes, Icon::Processes, "Processes", true),
    (Page::Cpu, Icon::Cpu, "CPU", true),
    (Page::Memory, Icon::Memory, "Memory", true),
    (Page::Disk, Icon::Disk, "Disk", true),
    (Page::Gpu, Icon::Gpu, "GPU", true),
    (Page::Sensors, Icon::Sensors, "Sensors", false),
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
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        setup_style(&cc.egui_ctx);
        App {
            shared: sys::start(cc.egui_ctx.clone()),
            snap: None,
            page: start_page(),
            page_at: Instant::now(),
            interval: 1,
            dwm_done: false,
            table: table::TableState::default(),
            shot: crate::shot::Shot::from_env(),
            trimmed: false,
            was_focused: true,
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

        widgets::logo(&p, pos2(rect.min.x + 26.0, cy), 18.0);
        p.text(pos2(rect.min.x + 44.0, cy), Align2::LEFT_CENTER, "Purgeon", sb(13.0), txt());

        // Live readout in the middle of the bar
        if let Some(s) = &self.snap {
            let t = format!("CPU {:.0}%   ·   RAM {:.0}%   ·   {} processes", s.cpu, s.mem.used_frac() * 100.0, s.nprocs);
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
        p.rect_filled(Rect::from_min_size(pos2(pill.min.x + 1.0, pill.center().y - 8.0), vec2(3.0, 16.0)), 1.5, ACCENT);

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
            if resp.clicked() && crate::platform::relaunch_as_admin() {
                crate::sys::etw::stop();
                std::process::exit(0);
            }
        }
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // kernel trace sessions outlive the process unless stopped
        crate::sys::etw::stop();
    }

    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        chrome().to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(s) = self.shared.take() {
            self.snap = Some(s);
        }
        // Trim once startup is over, and again whenever the window loses focus (Tangra's trick)
        let focused = ctx.input(|i| i.focused);
        if (!self.trimmed && self.page_at.elapsed().as_secs_f32() > 2.0 && self.snap.is_some()) || (self.was_focused && !focused) {
            self.trimmed = true;
            crate::platform::trim_memory();
        }
        self.was_focused = focused;
        if !self.dwm_done {
            self.dwm_done = crate::platform::style_window();
            if !self.dwm_done {
                ctx.request_repaint();
            }
        }
        // How far the newest sample has slid into the graphs
        let scroll = self.snap.as_ref().map_or(1.0, |s| (s.at.elapsed().as_secs_f32() / 0.35).min(1.0));
        if scroll < 1.0 {
            ctx.request_repaint();
        }
        let scroll = egui::emath::easing::cubic_out(scroll);

        egui::CentralPanel::default().frame(egui::Frame::none().fill(chrome())).show(ctx, |ui| {
            let full = ui.max_rect();
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
            let t = egui::emath::easing::cubic_out((self.page_at.elapsed().as_secs_f32() / 0.28).min(1.0));
            if t < 1.0 {
                ctx.request_repaint();
            }
            let inner = Rect::from_min_max(content.min + vec2(28.0, 22.0 + 10.0 * (1.0 - t)), content.max - vec2(28.0, 0.0));
            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(inner).id_salt(("page", self.page as u8)));
            cui.set_clip_rect(content.shrink(1.0));
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
                        Page::Gpu => pages::gpu(&mut cui, &s, scroll),
                        Page::Sensors => pages::soon(&mut cui, Icon::Sensors, "Sensors", &[
                            "GPU temperature, clocks, fan and power (NVML on NVIDIA)",
                            "CPU package and per-core temperatures",
                            "Some sensors need Purgeon to run as administrator",
                        ]),
                    };
                    if let Some(pg) = nav {
                        self.go(pg);
                    }
                }
            }
        });
        resize_edges(ctx);
        if let Some(sh) = &mut self.shot {
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

fn setup_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(bytes) = std::fs::read("C:\\Windows\\Fonts\\segoeui.ttf") {
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
    st.animation_time = 0.14;
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|s| *s = st.clone());
}

/// `--page=cpu` (overview, processes, cpu, memory, gpu, sensors) opens on that page
fn start_page() -> Page {
    let arg = std::env::args().find_map(|a| a.strip_prefix("--page=").map(str::to_lowercase));
    match arg.as_deref() {
        Some("processes") => Page::Processes,
        Some("cpu") => Page::Cpu,
        Some("memory") => Page::Memory,
        Some("disk") => Page::Disk,
        Some("gpu") => Page::Gpu,
        Some("sensors") => Page::Sensors,
        _ => Page::Overview,
    }
}
