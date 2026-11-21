//! Dashboard, CPU and Memory pages, and the placeholders for what's next.

use super::graph::{self, Graph, Series};
use super::widgets;
use super::Page;
use crate::anim::*;
use crate::fmt;
use crate::sys::Snapshot;
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Painter, Rect, ScrollArea, Sense, Stroke, Ui};
use std::collections::HashMap;

const GAP: f32 = 16.0;

fn header(ui: &mut Ui, title: &str, sub: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::hover());
    let p = ui.painter();
    p.text(pos2(r.min.x, r.min.y + 14.0), Align2::LEFT_CENTER, title, sb(24.0), txt());
    widgets::fit(p, pos2(r.min.x, r.min.y + 40.0), Align2::LEFT_CENTER, sub, reg(12.5), r.width(), txt_faint());
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    ui.painter().text(pos2(r.min.x + 2.0, r.center().y), Align2::LEFT_CENTER, title, sb(13.5), txt_dim());
}

/// Caption with a coloured dot
fn caption(p: &Painter, x: f32, y: f32, text: &str, col: Color32) -> f32 {
    caption_w(p, x, y, text, col, f32::INFINITY)
}

/// Caption that stays within `max_w` (shrinks, then ellipsizes)
pub(super) fn caption_w(p: &Painter, x: f32, y: f32, text: &str, col: Color32, max_w: f32) -> f32 {
    p.circle_filled(pos2(x + 4.0, y), 4.0, col);
    widgets::fit(p, pos2(x + 14.0, y), Align2::LEFT_CENTER, text, sb(12.5), max_w - 14.0, txt_dim()).max.x
}

/// A big number with an optional dim suffix
fn big(p: &Painter, x: f32, y: f32, value: &str, suffix: &str, size: f32) {
    let r = p.text(pos2(x, y), Align2::LEFT_CENTER, value, sb(size), txt());
    if !suffix.is_empty() {
        p.text(pos2(r.max.x + 4.0, y + size * 0.12), Align2::LEFT_CENTER, suffix, reg(size * 0.48), txt_faint());
    }
}

fn hover_in(ui: &Ui, r: Rect) -> Option<egui::Pos2> {
    ui.ctx().pointer_hover_pos().filter(|q| r.contains(*q) && ui.clip_rect().contains(*q))
}

fn pct_label(v: &[f32]) -> String {
    match v {
        [a] => format!("{a:.1}%"),
        [a, b, ..] => format!("{a:.1}%  /  {b:.1}%"),
        _ => String::new(),
    }
}

struct Group {
    name: String,
    path: Option<std::sync::Arc<str>>,
    count: u32,
    cpu: f32,
    /// RAM in use: private memory of every member plus the largest member's shared memory
    mem: u64,
    shared: u64,
    vram: u64,
    disk_r: f64,
    disk_w: f64,
    net_rx: f64,
    net_tx: f64,
    conns: crate::sys::net::Conns,
}

/// Processes summed by image name ("chrome.exe ×24")
fn grouped(s: &Snapshot) -> Vec<Group> {
    let mut m: HashMap<&str, Group> = HashMap::new();
    for p in &s.procs {
        let g = m.entry(p.name.as_str()).or_insert_with(|| Group {
            name: p.name.clone(),
            path: None,
            count: 0,
            cpu: 0.0,
            mem: 0,
            shared: 0,
            vram: 0,
            disk_r: 0.0,
            disk_w: 0.0,
            net_rx: 0.0,
            net_tx: 0.0,
            conns: Default::default(),
        });
        g.count += 1;
        if g.path.is_none() {
            g.path = p.path.clone();
        }
        g.cpu += p.cpu;
        g.mem += p.private_ws;
        g.shared = g.shared.max(p.shared());
        g.vram += p.vram;
        g.disk_r += p.disk_read;
        g.disk_w += p.disk_write;
        g.net_rx += p.net_rx;
        g.net_tx += p.net_tx;
        g.conns.tcp += p.conns.tcp;
        g.conns.listen += p.conns.listen;
        g.conns.udp += p.conns.udp;
    }
    let mut v: Vec<Group> = m.into_values().collect();
    for g in &mut v {
        g.mem += g.shared;
    }
    v
}

/// A card listing the top groups with a bar each
fn top_list(ui: &mut Ui, r: Rect, title: &str, col: Color32, rows: &[(&Group, f32, String)]) {
    top_list_w(ui, r, title, col, rows, 86.0)
}

/// `top_list` with room for a wider value text on the right
fn top_list_w(ui: &mut Ui, r: Rect, title: &str, col: Color32, rows: &[(&Group, f32, String)], value_w: f32) {
    let p = ui.painter().clone();
    widgets::card(&p, r, card());
    caption_w(&p, r.min.x + 18.0, r.min.y + 24.0, title, col, r.width() - 36.0);
    let max = rows.iter().map(|x| x.1).fold(0.0001_f32, f32::max);
    let row_h = 30.0;
    for (i, (g, v, txtv)) in rows.iter().enumerate() {
        let rr = Rect::from_min_size(pos2(r.min.x + 8.0, r.min.y + 46.0 + i as f32 * row_h), vec2(r.width() - 16.0, row_h));
        if rr.max.y > r.max.y - 6.0 {
            break;
        }
        let resp = ui.interact(rr, ui.id().with((title, i)), Sense::hover());
        let h = ease(ui.ctx(), resp.id.with("h"), resp.hovered());
        p.rect_filled(rr, 7.0, wa((h * 9.0) as u8));
        super::icons::paint(ui.ctx(), &p, Rect::from_center_size(pos2(rr.min.x + 20.0, rr.center().y), vec2(18.0, 18.0)), &g.name, g.path.as_ref());
        p.text(pos2(rr.max.x - 10.0, rr.center().y), Align2::RIGHT_CENTER, txtv, sb(12.5), txt());
        let bw = (rr.width() * 0.28).min(140.0);
        let br = Rect::from_min_size(pos2(rr.max.x - value_w - bw, rr.center().y - 2.5), vec2(bw, 5.0));
        // Name (and ×count) must end before the bar starts
        let room = br.min.x - 14.0 - (rr.min.x + 36.0) - if g.count > 1 { 34.0 } else { 0.0 };
        let name = super::table::clip(&p, &g.name, room.max(20.0));
        let nr = p.text(pos2(rr.min.x + 36.0, rr.center().y), Align2::LEFT_CENTER, name, reg(12.5), txt());
        if g.count > 1 {
            p.text(pos2(nr.max.x + 5.0, rr.center().y), Align2::LEFT_CENTER, format!("×{}", g.count), reg(11.5), txt_faint());
        }
        widgets::meter(&p, br, glide(ui.ctx(), resp.id.with("bar"), v / max, 10.0), col);
    }
}

/// What a CPU reading means, for tooltips
fn cpu_note(v: f32) -> String {
    if v < 50.0 {
        format!("{v:.0}% busy. Normal: below 50% the CPU has plenty of room.")
    } else if v < 85.0 {
        format!("{v:.0}% busy. High but fine for games, builds and exports. Above 85% for long, programs start waiting for the CPU.")
    } else {
        format!("{v:.0}% busy. The CPU is close to full and programs may stutter. Top CPU below shows what is using it.")
    }
}

/// What a memory reading means, for tooltips
fn mem_note(frac: f32) -> String {
    let v = frac * 100.0;
    if frac < 0.75 {
        format!("{v:.0}% in use. Normal: Windows uses spare memory as cache and gives it back when needed.")
    } else if frac < 0.9 {
        format!("{v:.0}% in use. Getting full: Windows starts compressing memory, which costs some CPU.")
    } else {
        format!("{v:.0}% in use. Nearly full: Windows is moving memory to the page file on disk, which is much slower. Closing the biggest users in Top memory helps.")
    }
}

/// What a GPU reading means, for tooltips
fn gpu_note(util: f32, vram_frac: f32) -> String {
    let u = if util < 90.0 {
        format!("GPU {util:.0}% busy. Normal.")
    } else {
        format!("GPU {util:.0}% busy. Fully used, which is expected while gaming: the game is limited by the GPU, not the CPU.")
    };
    let v = if vram_frac < 0.9 {
        format!("VRAM {:.0}% full. Fine.", vram_frac * 100.0)
    } else {
        format!("VRAM {:.0}% full. Above 90% games can stutter as textures spill into slower system memory.", vram_frac * 100.0)
    };
    format!("{u}\n{v}")
}

/// "Your PC has plenty of headroom", then up to five programs in plain words
fn summary(ui: &mut Ui, s: &Snapshot, w: f32) {
    let groups = grouped(s);
    let total = s.mem.total.max(1) as f32;
    // the programs holding the most memory, biggest first, all in the same unit
    let mut hogs: Vec<&Group> = groups.iter().filter(|g| g.mem as f32 / total >= 0.01).collect();
    hogs.sort_by(|a, b| b.mem.cmp(&a.mem));
    hogs.truncate(5);
    let biggest = hogs.first().map_or(1, |g| g.mem.max(1)) as f32;

    let mf = s.mem.used_frac();
    let (head, col) = if s.cpu > 85.0 {
        ("CPU nearly maxed out", DANGER)
    } else if mf > 0.9 {
        ("Memory nearly full", DANGER)
    } else if s.cpu > 50.0 || mf > 0.75 {
        ("Busy, with headroom left", WARN_COL)
    } else {
        ("Plenty of headroom", GPU_COL)
    };

    let (r, resp) = ui.allocate_exact_size(vec2(w, if hogs.is_empty() { 84.0 } else { 140.0 }), Sense::hover());
    let p = ui.painter().clone();
    widgets::card(&p, r, card());
    p.circle_filled(pos2(r.min.x + 22.0, r.min.y + 26.0), 4.0, col);
    let hr = p.text(pos2(r.min.x + 34.0, r.min.y + 26.0), Align2::LEFT_CENTER, head, sb(14.5), txt());
    if hogs.is_empty() {
        p.text(pos2(r.min.x + 34.0, r.min.y + 56.0), Align2::LEFT_CENTER, "Nothing is using much right now", reg(12.5), txt_faint());
    } else {
        p.text(pos2(hr.max.x + 10.0, r.min.y + 26.5), Align2::LEFT_CENTER, "Using the most memory", reg(12.0), txt_faint());
    }
    // one tile per program; the bars are sized against the biggest, so the ranking reads at a glance
    let (gap, slots) = (10.0, 5.0);
    let tw = (r.width() - 36.0 - gap * (slots - 1.0)) / slots;
    for (i, g) in hogs.iter().enumerate() {
        let value = format!("{:.2} GB", g.mem as f64 / 1073741824.0);
        let of = format!("{:.0}% of RAM", g.mem as f32 / total * 100.0);
        let share = g.mem as f32 / biggest;
        let t = Rect::from_min_size(pos2(r.min.x + 18.0 + i as f32 * (tw + gap), r.min.y + 48.0), vec2(tw, 76.0));
        p.rect_filled(t, 10.0, well());
        super::icons::paint(ui.ctx(), &p, Rect::from_center_size(pos2(t.min.x + 21.0, t.min.y + 20.0), vec2(18.0, 18.0)), &g.name, g.path.as_ref());
        let base = g.name.trim_end_matches(".exe").trim_end_matches(".EXE");
        let name = crate::platform::friendly_name(base, g.path.as_deref());
        widgets::fit(&p, pos2(t.min.x + 36.0, t.min.y + 20.0), Align2::LEFT_CENTER, &name, sb(12.5), tw - 46.0, txt());
        let vr = p.text(pos2(t.min.x + 12.0, t.min.y + 46.0), Align2::LEFT_CENTER, &value, sb(17.0), txt());
        widgets::fit(&p, pos2(vr.max.x + 6.0, t.min.y + 47.5), Align2::LEFT_CENTER, &of, reg(11.0), t.max.x - 10.0 - (vr.max.x + 6.0), txt_faint());
        widgets::meter(&p, Rect::from_min_size(pos2(t.min.x + 12.0, t.min.y + 63.0), vec2(tw - 24.0, 3.0)), share.clamp(0.0, 1.0), MEM_COL);
        let procs = if g.count > 1 { format!("{} processes", g.count) } else { "1 process".into() };
        ui.interact(t, egui::Id::new(("top_app", i)), Sense::hover()).on_hover_text(format!("{}  ·  {procs}", g.name));
    }
    resp.on_hover_text("The five programs holding the most memory, all their processes counted together. The Processes page shows everything.");
}

pub fn dashboard(ui: &mut Ui, s: &Snapshot, scroll: f32) -> Option<Page> {
    let mut go = None;
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let w = ui.available_width();
        header(ui, "Dashboard", &format!("{}  ·  up {}", s.info.cpu_name, fmt::uptime(s.uptime)));
        summary(ui, s, w);
        ui.add_space(GAP);
        let (row, _) = ui.allocate_exact_size(vec2(w, 236.0), Sense::hover());
        let cw = (w - GAP * 2.0) / 3.0;
        let rect = |i: usize| Rect::from_min_size(pos2(row.min.x + i as f32 * (cw + GAP), row.min.y), vec2(cw, row.height()));
        let ctx = ui.ctx().clone();
        let hov = |ui: &Ui, r: Rect| hover_in(ui, r);

        // CPU
        let (resp, r) = widgets::card_button(ui, rect(0), egui::Id::new("ov_cpu"));
        let p = ui.painter().clone();
        caption(&p, r.min.x + 18.0, r.min.y + 24.0, "CPU", CPU_COL);
        let v = glide(&ctx, egui::Id::new("ov_cpu_v"), s.cpu, 10.0);
        big(&p, r.min.x + 18.0, r.min.y + 62.0, &format!("{v:.0}%"), "", 32.0);
        // where the overlay's numbers fly to when it grows into this window
        super::morph::mark("cpu", pos2(r.min.x + 18.0, r.min.y + 62.0), 32.0);
        let sub = format!("{} cores  ·  {} threads", s.info.physical.max(1), s.info.logical);
        widgets::fit(&p, pos2(r.min.x + 18.0, r.min.y + 92.0), Align2::LEFT_CENTER, &sub, reg(12.0), r.width() - 36.0, txt_faint());
        let gr = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 112.0), pos2(r.max.x - 1.0, r.max.y - 12.0));
        let series = [Series { data: &s.h_cpu, col: CPU_COL, fill: true, width: 1.6 }];
        graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: false, hover: hov(ui, gr), interval: s.interval, label: &pct_label });
        if resp.clicked() {
            go = Some(Page::Cpu);
        }
        resp.on_hover_text(cpu_note(s.cpu));

        // Memory
        let (resp, r) = widgets::card_button(ui, rect(1), egui::Id::new("ov_mem"));
        caption(&p, r.min.x + 18.0, r.min.y + 24.0, "Memory", MEM_COL);
        let used = glide(&ctx, egui::Id::new("ov_mem_v"), s.mem.used() as f32 / 1073741824.0, 10.0);
        big(&p, r.min.x + 18.0, r.min.y + 62.0, &format!("{used:.1}"), &format!("/ {} GB", fmt::gb(s.mem.total)), 32.0);
        let sub = format!("{:.0}% in use  ·  {} committed", s.mem.used_frac() * 100.0, fmt::bytes(s.mem.commit));
        super::morph::mark("ram", pos2(r.min.x + 18.0, r.min.y + 92.0), 12.0);
        widgets::fit(&p, pos2(r.min.x + 18.0, r.min.y + 92.0), Align2::LEFT_CENTER, &sub, reg(12.0), r.width() - 36.0, txt_faint());
        let gr = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 112.0), pos2(r.max.x - 1.0, r.max.y - 12.0));
        let series = [Series { data: &s.h_mem, col: MEM_COL, fill: true, width: 1.6 }];
        graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: false, hover: hov(ui, gr), interval: s.interval, label: &pct_label });
        if resp.clicked() {
            go = Some(Page::Memory);
        }
        resp.on_hover_text(mem_note(s.mem.used_frac()));

        // GPU (the main adapter)
        let (resp, r) = widgets::card_button(ui, rect(2), egui::Id::new("ov_gpu"));
        caption(&p, r.min.x + 18.0, r.min.y + 24.0, "GPU", GPU_COL);
        match main_gpu(s) {
            Some(i) => {
                let g = &s.gpus[i];
                let v = glide(&ctx, egui::Id::new("ov_gpu_v"), g.util, 10.0);
                let temp = g.sensors.temp.map(|t| format!("{t:.0}°C")).unwrap_or_default();
                big(&p, r.min.x + 18.0, r.min.y + 62.0, &format!("{v:.0}%"), &temp, 32.0);
                super::morph::mark("gpu", pos2(r.min.x + 18.0, r.min.y + 62.0), 32.0);
                let vw = p.layout_no_wrap(format!("{v:.0}%"), sb(32.0), txt()).size().x;
                super::morph::mark("gpu_temp", pos2(r.min.x + 18.0 + vw + 4.0, r.min.y + 62.0 + 32.0 * 0.12), 32.0 * 0.48);
                let sub = format!("{} / {} VRAM", fmt::bytes(g.dedicated_used), fmt::bytes(g.dedicated_total));
                widgets::fit(&p, pos2(r.min.x + 18.0, r.min.y + 92.0), Align2::LEFT_CENTER, &sub, reg(12.0), r.width() - 36.0, txt_faint());
                let gr = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 112.0), pos2(r.max.x - 1.0, r.max.y - 12.0));
                let series = [
                    Series { data: &s.h_gpu[i], col: GPU_COL, fill: true, width: 1.6 },
                    Series { data: &s.h_vram[i], col: VRAM_COL, fill: false, width: 1.1 },
                ];
                graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: false, hover: hov(ui, gr), interval: s.interval, label: &pct_label });
            }
            None => {
                big(&p, r.min.x + 18.0, r.min.y + 62.0, "—", "", 32.0);
                p.text(pos2(r.min.x + 18.0, r.min.y + 92.0), Align2::LEFT_CENTER, "No GPU counters found", reg(12.0), txt_faint());
            }
        }
        if resp.clicked() {
            go = Some(Page::Gpu);
        }
        if let Some(g) = main_gpu(s).map(|i| &s.gpus[i]) {
            resp.on_hover_text(gpu_note(g.util, g.dedicated_used as f32 / g.dedicated_total.max(1) as f32));
        }

        ui.add_space(GAP);
        super::tools::cards(ui, w, GAP);
        ui.add_space(GAP);
        let groups = grouped(s);
        let (row, _) = ui.allocate_exact_size(vec2(w, 300.0), Sense::hover());
        let hw = (w - GAP) / 2.0;
        let mut by_cpu: Vec<&Group> = groups.iter().collect();
        by_cpu.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        let rows: Vec<(&Group, f32, String)> = by_cpu.iter().take(8).map(|g| (*g, g.cpu, fmt::pct(g.cpu))).collect();
        top_list(ui, Rect::from_min_size(row.min, vec2(hw, row.height())), "Top CPU", CPU_COL, &rows);
        let mut by_mem: Vec<&Group> = groups.iter().collect();
        by_mem.sort_by(|a, b| b.mem.cmp(&a.mem));
        let rows: Vec<(&Group, f32, String)> = by_mem.iter().take(8).map(|g| (*g, g.mem as f32, fmt::bytes(g.mem))).collect();
        top_list(ui, Rect::from_min_size(pos2(row.min.x + hw + GAP, row.min.y), vec2(hw, row.height())), "Top memory", MEM_COL, &rows);
        ui.add_space(GAP);
    });
    go
}

pub fn cpu(ui: &mut Ui, s: &Snapshot, scroll: f32) -> Option<Page> {
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let w = ui.available_width();
        let ctx = ui.ctx().clone();
        header(ui, "CPU", &s.info.cpu_name);

        let (r, _) = ui.allocate_exact_size(vec2(w, 300.0), Sense::hover());
        let p = ui.painter().clone();
        widgets::card(&p, r, card());
        caption(&p, r.min.x + 18.0, r.min.y + 24.0, "Utilization", CPU_COL);
        let x = caption(&p, r.max.x - 170.0, r.min.y + 24.0, "Total", CPU_COL);
        caption(&p, x + 14.0, r.min.y + 24.0, "Kernel", KERNEL_COL);
        let v = glide(&ctx, egui::Id::new("cpu_v"), s.cpu, 10.0);
        big(&p, r.min.x + 18.0, r.min.y + 60.0, &format!("{v:.1}%"), &format!("kernel {:.1}%", s.kernel), 30.0);
        let gr = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 90.0), pos2(r.max.x - 1.0, r.max.y - 12.0));
        let series = [
            Series { data: &s.h_cpu, col: CPU_COL, fill: true, width: 1.8 },
            Series { data: &s.h_kernel, col: KERNEL_COL, fill: false, width: 1.2 },
        ];
        graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: true, hover: hover_in(ui, gr), interval: s.interval, label: &pct_label });

        ui.add_space(GAP);
        let speed = if s.clocks.avg_mhz > 0.0 { format!("{:.2} GHz", s.clocks.avg_mhz / 1000.0) } else { "—".into() };
        let tm = s.clocks.utility.map(|u| format!("{u:.1}%")).unwrap_or_else(|| "—".into());
        let tiles: [(&str, String); 10] = [
            ("Utilization", format!("{:.1}%", s.cpu)),
            ("Task Manager figure", tm),
            ("Speed", speed),
            ("Kernel time", format!("{:.1}%", s.kernel)),
            ("Processes", fmt::thousands(s.nprocs as u64)),
            ("Threads", fmt::thousands(s.threads as u64)),
            ("Handles", fmt::thousands(s.handles as u64)),
            ("Up time", fmt::uptime(s.uptime)),
            ("Base speed", if s.info.base_mhz > 0 { format!("{:.2} GHz", s.info.base_mhz as f32 / 1000.0) } else { "—".into() }),
            ("Cores / threads", format!("{} / {}", s.info.physical.max(1), s.info.logical)),
        ];
        let o = ui.cursor().min;
        let (rects, h) = widgets::tile_grid(o, w, tiles.len(), 150.0, 64.0, 12.0);
        ui.allocate_exact_size(vec2(w, h), Sense::hover());
        for (r, (c, v)) in rects.iter().zip(&tiles) {
            widgets::tile(&p, *r, c, v, None);
            if *c == "Task Manager figure" {
                ui.interact(*r, egui::Id::new("tm_tip"), Sense::hover()).on_hover_text(
                    "Task Manager's \"% Processor Utility\": busy time multiplied by how fast the clock ran versus base speed. \
                     It reads high when the CPU turbo-boosts and low when it parks. \"Utilization\" is the plain share of time the cores were busy.",
                );
            }
        }

        ui.add_space(GAP);
        section(ui, &format!("Logical processors  ·  {}", s.cores.len()));
        let o = ui.cursor().min;
        let n = s.cores.len();
        let (rects, h) = widgets::tile_grid(o, w, n, 170.0, 92.0, 10.0);
        ui.allocate_exact_size(vec2(w, h), Sense::hover());
        for (i, r) in rects.iter().enumerate() {
            p.rect_filled(*r, 10.0, card());
            p.rect_stroke(*r, 10.0, Stroke::new(1.0_f32, border()));
            let v = s.cores[i];
            let lr = p.text(pos2(r.min.x + 12.0, r.min.y + 16.0), Align2::LEFT_CENTER, format!("CPU {i}"), reg(11.5), txt_faint());
            if let Some(m) = s.clocks.mhz.get(i).filter(|m| **m > 0.0) {
                p.text(pos2(lr.max.x + 6.0, r.min.y + 16.0), Align2::LEFT_CENTER, format!("{:.2} GHz", m / 1000.0), reg(10.5), txt_faint());
            }
            p.text(pos2(r.max.x - 12.0, r.min.y + 16.0), Align2::RIGHT_CENTER, format!("{v:.0}%"), sb(12.5), mix(txt_dim(), hi(), v / 100.0));
            let gr = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 30.0), pos2(r.max.x - 1.0, r.max.y - 6.0));
            let data = s.h_cores.get(i).map(|v| v.as_slice()).unwrap_or(&[]);
            let series = [Series { data, col: CPU_COL, fill: true, width: 1.3 }];
            graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: false, hover: hover_in(ui, gr), interval: s.interval, label: &pct_label });
        }
        ui.add_space(GAP);
    });
    None
}

pub fn memory(ui: &mut Ui, s: &Snapshot, scroll: f32) -> Option<Page> {
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let w = ui.available_width();
        let ctx = ui.ctx().clone();
        let m = &s.mem;
        header(ui, "Memory", &format!("{} installed  ·  {} committed of {}", fmt::bytes(m.total), fmt::bytes(m.commit), fmt::bytes(m.commit_limit)));

        let (r, _) = ui.allocate_exact_size(vec2(w, 280.0), Sense::hover());
        let p = ui.painter().clone();
        widgets::card(&p, r, card());
        caption(&p, r.min.x + 18.0, r.min.y + 24.0, "In use", MEM_COL);
        let x = caption(&p, r.max.x - 210.0, r.min.y + 24.0, "In use", MEM_COL);
        caption(&p, x + 14.0, r.min.y + 24.0, "Committed", COMMIT_COL);
        let used = glide(&ctx, egui::Id::new("mem_v"), m.used() as f32 / 1073741824.0, 10.0);
        big(&p, r.min.x + 18.0, r.min.y + 60.0, &format!("{used:.1} GB"), &format!("{:.0}% of {} GB", m.used_frac() * 100.0, fmt::gb(m.total)), 30.0);
        let gr = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 90.0), pos2(r.max.x - 1.0, r.max.y - 12.0));
        let series = [
            Series { data: &s.h_mem, col: MEM_COL, fill: true, width: 1.8 },
            Series { data: &s.h_commit, col: COMMIT_COL, fill: false, width: 1.2 },
        ];
        graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: true, hover: hover_in(ui, gr), interval: s.interval, label: &pct_label });

        // Composition bar
        ui.add_space(GAP);
        let (r, _) = ui.allocate_exact_size(vec2(w, 104.0), Sense::hover());
        widgets::card(&p, r, card());
        let segs: Vec<(&str, u64, Color32, &str)> = match m.lists {
            Some(l) => vec![
                ("In use", m.total.saturating_sub(m.avail).saturating_sub(l.modified), MEM_COL, "Memory used by processes, drivers and the system"),
                ("Modified", l.modified, WARN_COL, "Changed pages that must be written to disk before they can be reused"),
                ("Standby", l.standby, KERNEL_COL, "Cached data that is not in use, freed instantly when needed"),
                ("Free", l.free + l.zero, Color32::from_gray(0x55), "Memory that holds nothing"),
            ],
            None => vec![
                ("In use", m.used(), MEM_COL, "Memory used by processes, drivers and the system"),
                ("Available", m.avail, Color32::from_gray(0x55), "Standby cache plus free memory, ready for programs"),
            ],
        };
        let bar = Rect::from_min_max(pos2(r.min.x + 18.0, r.min.y + 18.0), pos2(r.max.x - 18.0, r.min.y + 46.0));
        p.rect_filled(bar, 7.0, well());
        let total: u64 = segs.iter().map(|x| x.1).sum::<u64>().max(1);
        let mut x = bar.min.x;
        let mut lx = r.min.x + 18.0;
        for (i, (name, b, col, tip)) in segs.iter().enumerate() {
            let fw = glide(&ctx, egui::Id::new(("seg", i)), *b as f32 / total as f32, 8.0) * bar.width();
            let sr = Rect::from_min_size(pos2(x, bar.min.y), vec2(fw.max(0.0), bar.height()));
            let resp = ui.interact(sr, egui::Id::new(("segh", i)), Sense::hover());
            let h = ease(&ctx, resp.id.with("h"), resp.hovered());
            let rounding = egui::Rounding {
                nw: if i == 0 { 7.0 } else { 0.0 },
                sw: if i == 0 { 7.0 } else { 0.0 },
                ne: if i == segs.len() - 1 { 7.0 } else { 0.0 },
                se: if i == segs.len() - 1 { 7.0 } else { 0.0 },
            };
            p.rect_filled(sr.shrink2(vec2(0.5, 0.0)), rounding, mix(*col, hi(), h * 0.25));
            resp.on_hover_text(format!("{name}: {}\n{tip}", fmt::bytes(*b)));
            x += fw;
            // legend
            let ly = r.max.y - 30.0;
            p.circle_filled(pos2(lx + 4.0, ly), 4.0, *col);
            let tr = p.text(pos2(lx + 14.0, ly), Align2::LEFT_CENTER, *name, reg(12.0), txt_dim());
            let vr = p.text(pos2(tr.max.x + 6.0, ly), Align2::LEFT_CENTER, fmt::bytes(*b), sb(12.0), txt());
            lx = vr.max.x + 22.0;
        }
        if m.lists.is_none() {
            p.text(pos2(r.max.x - 18.0, r.max.y - 30.0), Align2::RIGHT_CENTER, "Run as administrator for standby / modified / free", reg(11.5), txt_faint());
        }

        ui.add_space(GAP);
        let mut tiles: Vec<(&str, String, Option<Color32>)> = vec![
            ("In use", fmt::bytes(m.used()), Some(MEM_COL)),
            ("Available", fmt::bytes(m.avail), None),
            ("Committed", format!("{} / {}", fmt::gb(m.commit), fmt::gb(m.commit_limit)) + " GB", Some(COMMIT_COL)),
            ("Commit peak", fmt::bytes(m.commit_peak), None),
            ("Cached (system)", fmt::bytes(m.cache), None),
            ("Paged pool", fmt::bytes(m.paged), None),
            ("Non-paged pool", fmt::bytes(m.nonpaged), None),
        ];
        if let Some(l) = m.lists {
            tiles.push(("Standby", fmt::bytes(l.standby), Some(KERNEL_COL)));
            tiles.push(("Modified", fmt::bytes(l.modified), Some(WARN_COL)));
            tiles.push(("Free", fmt::bytes(l.free + l.zero), None));
        }
        let o = ui.cursor().min;
        let (rects, h) = widgets::tile_grid(o, w, tiles.len(), 150.0, 64.0, 12.0);
        ui.allocate_exact_size(vec2(w, h), Sense::hover());
        for (r, (c, v, d)) in rects.iter().zip(&tiles) {
            widgets::tile(&p, *r, c, v, *d);
        }

        ui.add_space(GAP);
        let groups = grouped(s);
        let mut by_mem: Vec<&Group> = groups.iter().collect();
        by_mem.sort_by(|a, b| b.mem.cmp(&a.mem));
        let rows: Vec<(&Group, f32, String)> = by_mem.iter().take(10).map(|g| (*g, g.mem as f32, fmt::bytes(g.mem))).collect();
        let (r, _) = ui.allocate_exact_size(vec2(w, 60.0 + rows.len() as f32 * 30.0), Sense::hover());
        top_list(ui, r, "Biggest memory users (private working set, grouped by name)", MEM_COL, &rows);
        ui.add_space(GAP);
    });
    None
}

fn rate_label(v: &[f32]) -> String {
    match v {
        [r, w, ..] => format!("R {}  ·  W {}", fmt::rate(*r as f64), fmt::rate(*w as f64)),
        [a] => fmt::rate(*a as f64),
        _ => String::new(),
    }
}

fn disk_title(d: &crate::sys::disk::Disk) -> String {
    if d.letters.is_empty() {
        format!("Disk {}", d.index)
    } else {
        format!("Disk {}  ({})", d.index, d.letters.join(" "))
    }
}

pub fn disk(ui: &mut Ui, s: &Snapshot, scroll: f32) -> Option<Page> {
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let w = ui.available_width();
        let ctx = ui.ctx().clone();
        let total_r: f64 = s.disks.iter().map(|d| d.read).sum();
        let total_w: f64 = s.disks.iter().map(|d| d.write).sum();
        header(ui, "Disk", &format!("{} physical disks  ·  reading {}  ·  writing {}", s.disks.len(), fmt::rate(total_r), fmt::rate(total_w)));
        if s.disks.is_empty() {
            return;
        }
        let p = ui.painter().clone();
        for (i, d) in s.disks.iter().enumerate() {
            let h = &s.h_disk[i];
            let (r, _) = ui.allocate_exact_size(vec2(w, 316.0), Sense::hover());
            widgets::card(&p, r, card());
            caption_w(&p, r.min.x + 18.0, r.min.y + 24.0, &disk_title(d), DISK_COL, r.width() - 36.0);
            let v = glide(&ctx, egui::Id::new(("disk_v", i)), d.active, 10.0);
            big(&p, r.min.x + 18.0, r.min.y + 60.0, &format!("{v:.0}%"), &format!("active  ·  {:.1} ms response", d.resp_ms), 30.0);

            // Two labelled panels: active time (0-100%) and throughput (scaled to its own peak)
            let half = (r.width() - 36.0 - GAP) / 2.0;
            let pa = Rect::from_min_size(pos2(r.min.x + 18.0, r.min.y + 92.0), vec2(half, r.height() - 92.0 - 16.0));
            let pb = Rect::from_min_size(pos2(pa.max.x + GAP, pa.min.y), pa.size());
            for pr in [pa, pb] {
                p.rect_filled(pr, 8.0, well());
                p.rect_stroke(pr, 8.0, Stroke::new(1.0_f32, border()));
            }
            p.text(pos2(pa.min.x + 12.0, pa.min.y + 16.0), Align2::LEFT_CENTER, "Active time", sb(11.5), txt_dim());
            p.text(pos2(pa.max.x - 12.0, pa.min.y + 16.0), Align2::RIGHT_CENTER, "100%", reg(10.5), txt_faint());
            let lx = caption(&p, pb.min.x + 8.0, pb.min.y + 16.0, "Read", IO_COL);
            caption(&p, lx + 10.0, pb.min.y + 16.0, "Write", WARN_COL);
            let ga = Rect::from_min_max(pos2(pa.min.x + 1.0, pa.min.y + 30.0), pos2(pa.max.x - 1.0, pa.max.y - 6.0));
            let gb = Rect::from_min_max(pos2(pb.min.x + 1.0, pb.min.y + 30.0), pos2(pb.max.x - 1.0, pb.max.y - 6.0));
            let series = [Series { data: &h[0], col: DISK_COL, fill: true, width: 1.6 }];
            graph::paint(&p, ga, &Graph { series: &series, max: 100.0, scroll, grid: true, hover: hover_in(ui, ga), interval: s.interval, label: &pct_label });
            let peak = h[1].iter().chain(h[2].iter()).fold(1048576.0_f32, |a, b| a.max(*b)) * 1.15;
            let peak = glide(&ctx, egui::Id::new(("disk_peak", i)), peak, 4.0);
            let series = [
                Series { data: &h[1], col: IO_COL, fill: true, width: 1.5 },
                Series { data: &h[2], col: WARN_COL, fill: false, width: 1.3 },
            ];
            graph::paint(&p, gb, &Graph { series: &series, max: peak, scroll, grid: true, hover: hover_in(ui, gb), interval: s.interval, label: &rate_label });
            p.text(pos2(pb.max.x - 12.0, pb.min.y + 16.0), Align2::RIGHT_CENTER, fmt::rate(peak as f64), reg(10.5), txt_faint());

            ui.add_space(12.0);
            let mut tiles: Vec<(String, String, Option<Color32>)> = vec![
                ("Read speed".into(), fmt::rate(d.read), Some(IO_COL)),
                ("Write speed".into(), fmt::rate(d.write), Some(WARN_COL)),
                ("Active time".into(), format!("{:.1}%", d.active), Some(DISK_COL)),
                ("Response time".into(), format!("{:.2} ms", d.resp_ms), None),
                ("Queue length".into(), format!("{:.0}", d.queue), None),
            ];
            for (l, total, free) in &d.volumes {
                tiles.push((format!("{l} used"), format!("{} / {}", fmt::bytes(total - free), fmt::bytes(*total)), None));
            }
            let o = ui.cursor().min;
            let (rects, th) = widgets::tile_grid(o, w, tiles.len(), 170.0, 64.0, 12.0);
            ui.allocate_exact_size(vec2(w, th), Sense::hover());
            for (tr, (c, v, dcol)) in rects.iter().zip(&tiles) {
                widgets::tile(&p, *tr, c, v, *dcol);
            }
            ui.add_space(GAP + 8.0);
        }

        // Which programs are using the disk
        if s.disk_trace {
            let groups = grouped(s);
            let mut by: Vec<&Group> = groups.iter().filter(|g| g.disk_r + g.disk_w > 0.0).collect();
            by.sort_by(|a, b| (b.disk_r + b.disk_w).total_cmp(&(a.disk_r + a.disk_w)));
            let rows: Vec<(&Group, f32, String)> = by
                .iter()
                .take(10)
                .map(|g| (*g, (g.disk_r + g.disk_w) as f32, format!("R {}  ·  W {}", fmt::rate(g.disk_r), fmt::rate(g.disk_w))))
                .collect();
            let (r, _) = ui.allocate_exact_size(vec2(w, 60.0 + rows.len().max(1) as f32 * 30.0), Sense::hover());
            top_list_w(ui, r, "Programs using the disk (physical I/O)", DISK_COL, &rows, 170.0);
            let (or, ow) = s.disk_orphan;
            if or + ow > 0.0 {
                // I/O whose thread exited before we could name its process: shown so the sum adds up
                let t = format!("+ {} from programs that already exited", fmt::rate(or + ow));
                widgets::fit(&p, pos2(r.max.x - 18.0, r.min.y + 24.0), Align2::RIGHT_CENTER, &t, reg(11.0), r.width() * 0.4, txt_faint());
            }
            if rows.is_empty() {
                p.text(pos2(r.min.x + 18.0, r.min.y + 60.0), Align2::LEFT_CENTER, "No disk activity in the last update", reg(12.5), txt_faint());
            }
        } else {
            let (r, _) = ui.allocate_exact_size(vec2(w, 74.0), Sense::hover());
            widgets::card(&p, r, card());
            caption_w(&p, r.min.x + 18.0, r.min.y + 24.0, "Which programs are using the disk", DISK_COL, r.width() - 36.0);
            widgets::fit(
                &p,
                pos2(r.min.x + 18.0, r.min.y + 50.0),
                Align2::LEFT_CENTER,
                "Click Elevate (bottom left) to see per-program physical reads, writes and response time.",
                reg(12.0),
                r.width() - 36.0,
                txt_faint(),
            );
        }
        ui.add_space(GAP);
    });
    None
}

fn net_label(v: &[f32]) -> String {
    match v {
        [d, u, ..] => format!("↓ {}  ·  ↑ {}", fmt::bits(*d as f64), fmt::bits(*u as f64)),
        [a] => fmt::bits(*a as f64),
        _ => String::new(),
    }
}

pub fn network(ui: &mut Ui, s: &Snapshot, scroll: f32) -> Option<Page> {
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let w = ui.available_width();
        let ctx = ui.ctx().clone();
        let down: f64 = s.nets.iter().map(|a| a.rx).sum();
        let up: f64 = s.nets.iter().map(|a| a.tx).sum();
        let n = s.nets.len();
        header(
            ui,
            "Network",
            &format!("{n} connected adapter{}  ·  down {}  ·  up {}", if n == 1 { "" } else { "s" }, fmt::bits(down), fmt::bits(up)),
        );
        let p = ui.painter().clone();
        if s.nets.is_empty() {
            let (r, _) = ui.allocate_exact_size(vec2(w, 74.0), Sense::hover());
            widgets::card(&p, r, card());
            caption_w(&p, r.min.x + 18.0, r.min.y + 24.0, "No connected network adapters", NET_COL, r.width() - 36.0);
            widgets::fit(
                &p,
                pos2(r.min.x + 18.0, r.min.y + 50.0),
                Align2::LEFT_CENTER,
                "Plug in a cable or join a Wi-Fi network and it shows up here.",
                reg(12.0),
                r.width() - 36.0,
                txt_faint(),
            );
            ui.add_space(GAP);
        }
        for (i, a) in s.nets.iter().enumerate() {
            let h = &s.h_net[i];
            let (r, _) = ui.allocate_exact_size(vec2(w, 290.0), Sense::hover());
            widgets::card(&p, r, card());
            // "Ethernet 2 · Realtek ...", with the adapter type added when the name doesn't say it
            let mut title = a.name.clone();
            if !a.name.to_lowercase().contains(&a.kind.to_lowercase()) && a.kind != "Network" {
                title += &format!(" ({})", a.kind);
            }
            if !a.desc.is_empty() && a.desc != a.name {
                title += &format!("  ·  {}", a.desc);
            }
            caption_w(&p, r.min.x + 18.0, r.min.y + 24.0, &title, NET_COL, r.width() - 36.0);
            let v = glide(&ctx, egui::Id::new(("net_v", a.luid)), a.rx as f32, 10.0);
            big(&p, r.min.x + 18.0, r.min.y + 60.0, &fmt::bits(v as f64), &format!("down  ·  {} up", fmt::bits(a.tx)), 30.0);

            // Download and upload on one graph, scaled to the recent peak (at least 1 Mbps)
            let pr = Rect::from_min_size(pos2(r.min.x + 18.0, r.min.y + 92.0), vec2(r.width() - 36.0, r.height() - 92.0 - 16.0));
            p.rect_filled(pr, 8.0, well());
            p.rect_stroke(pr, 8.0, Stroke::new(1.0_f32, border()));
            let lx = caption(&p, pr.min.x + 8.0, pr.min.y + 16.0, "Download", NET_COL);
            caption(&p, lx + 10.0, pr.min.y + 16.0, "Upload", NET_UP_COL);
            let g = Rect::from_min_max(pos2(pr.min.x + 1.0, pr.min.y + 30.0), pos2(pr.max.x - 1.0, pr.max.y - 6.0));
            let peak = h[0].iter().chain(h[1].iter()).fold(125_000.0_f32, |m, b| m.max(*b)) * 1.15;
            let peak = glide(&ctx, egui::Id::new(("net_peak", a.luid)), peak, 4.0);
            let series = [
                Series { data: &h[0], col: NET_COL, fill: true, width: 1.5 },
                Series { data: &h[1], col: NET_UP_COL, fill: false, width: 1.3 },
            ];
            graph::paint(&p, g, &Graph { series: &series, max: peak, scroll, grid: true, hover: hover_in(ui, g), interval: s.interval, label: &net_label });
            p.text(pos2(pr.max.x - 12.0, pr.min.y + 16.0), Align2::RIGHT_CENTER, fmt::bits(peak as f64), reg(10.5), txt_faint());

            ui.add_space(12.0);
            let link = if a.link_rx == a.link_tx || a.link_tx == 0 {
                fmt::link(a.link_rx)
            } else {
                format!("{} ↓  ·  {} ↑", fmt::link(a.link_rx), fmt::link(a.link_tx))
            };
            let tiles: [(&str, String, Option<Color32>); 5] = [
                ("Download", fmt::bits(a.rx), Some(NET_COL)),
                ("Upload", fmt::bits(a.tx), Some(NET_UP_COL)),
                ("Received since start", fmt::bytes(a.rx_total), None),
                ("Sent since start", fmt::bytes(a.tx_total), None),
                ("Link speed", link, None),
            ];
            let o = ui.cursor().min;
            let (rects, th) = widgets::tile_grid(o, w, tiles.len(), 170.0, 64.0, 12.0);
            ui.allocate_exact_size(vec2(w, th), Sense::hover());
            for (tr, (c, v, dcol)) in rects.iter().zip(&tiles) {
                widgets::tile(&p, *tr, c, v, *dcol);
            }
            ui.add_space(GAP + 8.0);
        }

        let groups = grouped(s);
        // Which programs are using the network (kernel trace, administrator only)
        if s.net_trace {
            let mut by: Vec<&Group> = groups.iter().filter(|g| g.net_rx + g.net_tx > 0.0).collect();
            by.sort_by(|a, b| (b.net_rx + b.net_tx).total_cmp(&(a.net_rx + a.net_tx)));
            let rows: Vec<(&Group, f32, String)> = by
                .iter()
                .take(10)
                .map(|g| (*g, (g.net_rx + g.net_tx) as f32, format!("↓ {}  ·  ↑ {}", fmt::bits(g.net_rx), fmt::bits(g.net_tx))))
                .collect();
            let (r, _) = ui.allocate_exact_size(vec2(w, 60.0 + rows.len().max(1) as f32 * 30.0), Sense::hover());
            top_list_w(ui, r, "Programs using the network", NET_COL, &rows, 190.0);
            if rows.is_empty() {
                p.text(pos2(r.min.x + 18.0, r.min.y + 60.0), Align2::LEFT_CENTER, "No network traffic in the last update", reg(12.5), txt_faint());
            }
        } else {
            let (r, _) = ui.allocate_exact_size(vec2(w, 74.0), Sense::hover());
            widgets::card(&p, r, card());
            caption_w(&p, r.min.x + 18.0, r.min.y + 24.0, "Which programs are using the network", NET_COL, r.width() - 36.0);
            widgets::fit(
                &p,
                pos2(r.min.x + 18.0, r.min.y + 50.0),
                Align2::LEFT_CENTER,
                "Click Elevate (bottom left) to see download and upload speed per program.",
                reg(12.0),
                r.width() - 36.0,
                txt_faint(),
            );
        }
        ui.add_space(GAP);

        // Open connections: available without administrator
        let mut by: Vec<&Group> = groups.iter().filter(|g| g.conns.total() > 0).collect();
        by.sort_by(|a, b| b.conns.tcp.cmp(&a.conns.tcp).then(b.conns.total().cmp(&a.conns.total())));
        let rows: Vec<(&Group, f32, String)> = by
            .iter()
            .take(10)
            .map(|g| {
                let c = g.conns;
                let mut t = format!("{} connected", c.tcp);
                if c.listen > 0 {
                    t += &format!("  ·  {} listening", c.listen);
                }
                if c.udp > 0 {
                    t += &format!("  ·  {} UDP", c.udp);
                }
                (*g, c.total() as f32, t)
            })
            .collect();
        let (r, resp) = ui.allocate_exact_size(vec2(w, 60.0 + rows.len().max(1) as f32 * 30.0), Sense::hover());
        top_list_w(ui, r, "Open connections", NET_UP_COL, &rows, 250.0);
        resp.on_hover_text(
            "Connected: TCP connections to another computer or server. Listening: ports the program waits on for others to connect. UDP: sockets used by games, voice chat and DNS, which have no connection state.",
        );
        if rows.is_empty() {
            p.text(pos2(r.min.x + 18.0, r.min.y + 60.0), Align2::LEFT_CENTER, "No open connections", reg(12.5), txt_faint());
        }
        ui.add_space(GAP);
    });
    None
}

/// The adapter people care about: the one with the most VRAM
fn main_gpu(s: &Snapshot) -> Option<usize> {
    (0..s.gpus.len()).max_by_key(|&i| s.gpus[i].dedicated_total)
}

fn engine_name(k: &str) -> String {
    match k {
        "VideoDecode" => "Video decode".into(),
        "VideoEncode" => "Video encode".into(),
        "LegacyOverlay" => "Overlay".into(),
        _ => k.replace('_', " "),
    }
}

/// Green when cool, orange when warm, red when hot
pub fn temp_col(t: f32) -> Color32 {
    if t < 60.0 {
        GPU_COL
    } else if t < 80.0 {
        WARN_COL
    } else {
        DANGER
    }
}

pub fn gpu(ui: &mut Ui, s: &Snapshot, scroll: f32) -> Option<Page> {
    let Some(main) = main_gpu(s) else {
        header(ui, "GPU", "No GPU performance counters were found on this system");
        return None;
    };
    let ctx = ui.ctx().clone();
    let sel_id = egui::Id::new("gpu_sel");
    let mut sel = ctx.data(|d| d.get_temp::<usize>(sel_id)).unwrap_or(main).min(s.gpus.len() - 1);
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let w = ui.available_width();
        let top = ui.cursor().min;
        let g = &s.gpus[sel];
        let vendor = g.vendor_name();
        let sub = if vendor.is_empty() {
            format!("{} dedicated memory", fmt::bytes(g.dedicated_total))
        } else {
            format!("{vendor}  ·  {} dedicated memory", fmt::bytes(g.dedicated_total))
        };
        header(ui, &g.name, &sub);
        if s.gpus.len() > 1 {
            // Adapter picker in the header's right corner
            let names: Vec<String> = (0..s.gpus.len()).map(|i| format!("GPU {i}")).collect();
            let refs: Vec<&str> = names.iter().map(|x| x.as_str()).collect();
            let sw = 76.0 * refs.len() as f32;
            let r = Rect::from_min_size(pos2(top.x + w - sw, top.y + 4.0), vec2(sw, 30.0));
            if widgets::segmented(ui, r, &refs, &mut sel, sel_id.with("seg")) {
                ctx.data_mut(|d| d.insert_temp(sel_id, sel));
            }
        }
        let g = &s.gpus[sel];

        let (r, _) = ui.allocate_exact_size(vec2(w, 280.0), Sense::hover());
        let p = ui.painter().clone();
        widgets::card(&p, r, card());
        caption(&p, r.min.x + 18.0, r.min.y + 24.0, "Utilization", GPU_COL);
        let x = caption(&p, r.max.x - 230.0, r.min.y + 24.0, "Usage", GPU_COL);
        caption(&p, x + 14.0, r.min.y + 24.0, "Dedicated memory", VRAM_COL);
        let v = glide(&ctx, egui::Id::new(("gpu_v", sel)), g.util, 10.0);
        let busiest = g
            .engines
            .iter()
            .max_by(|a, b| a.util.total_cmp(&b.util))
            .filter(|e| e.util >= 0.5)
            .map(|e| format!("busiest: {}", engine_name(&e.kind)))
            .unwrap_or_default();
        big(&p, r.min.x + 18.0, r.min.y + 60.0, &format!("{v:.1}%"), &busiest, 30.0);
        let gr = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 90.0), pos2(r.max.x - 1.0, r.max.y - 12.0));
        let series = [
            Series { data: &s.h_gpu[sel], col: GPU_COL, fill: true, width: 1.8 },
            Series { data: &s.h_vram[sel], col: VRAM_COL, fill: false, width: 1.2 },
        ];
        graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: true, hover: hover_in(ui, gr), interval: s.interval, label: &pct_label });

        // Engines
        ui.add_space(GAP);
        section(ui, "Engines");
        let engines: Vec<&crate::sys::gpu::Engine> = g
            .engines
            .iter()
            .filter(|e| matches!(e.kind.as_str(), "3D" | "Copy" | "VideoDecode" | "VideoEncode") || e.kind.starts_with("Compute") || e.util >= 0.5)
            .collect();
        let o = ui.cursor().min;
        let (rects, h) = widgets::tile_grid(o, w, engines.len(), 170.0, 64.0, 12.0);
        ui.allocate_exact_size(vec2(w, h), Sense::hover());
        for (r, e) in rects.iter().zip(&engines) {
            p.rect_filled(*r, 10.0, card());
            p.rect_stroke(*r, 10.0, Stroke::new(1.0_f32, border()));
            p.text(pos2(r.min.x + 14.0, r.min.y + 20.0), Align2::LEFT_CENTER, engine_name(&e.kind), reg(12.0), txt_faint());
            p.text(pos2(r.max.x - 14.0, r.min.y + 20.0), Align2::RIGHT_CENTER, format!("{:.0}%", e.util), sb(13.0), mix(txt_dim(), hi(), e.util / 100.0));
            let br = Rect::from_min_max(pos2(r.min.x + 14.0, r.max.y - 20.0), pos2(r.max.x - 14.0, r.max.y - 15.0));
            let f = glide(&ctx, egui::Id::new(("eng", sel, e.kind.as_str())), e.util / 100.0, 10.0);
            widgets::meter(&p, br, f, GPU_COL);
        }

        // Memory and sensors
        ui.add_space(GAP);
        section(ui, "Memory and sensors");
        let n = &g.sensors;
        let mut tiles: Vec<(&str, String, Option<Color32>)> = vec![
            ("Dedicated memory", format!("{} / {}", fmt::bytes(g.dedicated_used), fmt::bytes(g.dedicated_total)), Some(VRAM_COL)),
            ("Shared memory", format!("{} / {}", fmt::bytes(g.shared_used), fmt::bytes(g.shared_total)), None),
        ];
        if let Some(t) = n.temp {
            tiles.push(("Temperature", format!("{t:.0} °C"), Some(temp_col(t))));
        }
        if let Some(c) = n.core_mhz {
            tiles.push(("Core clock", format!("{c} MHz"), None));
        }
        if let Some(c) = n.mem_mhz {
            tiles.push(("Memory clock", format!("{c} MHz"), None));
        }
        if let Some(pw) = n.power_w {
            let lim = n.power_limit_w.map(|l| format!(" / {l:.0} W")).unwrap_or_default();
            tiles.push(("Power", format!("{pw:.0} W{lim}"), None));
        }
        if let Some(f) = n.fan_pct {
            tiles.push(("Fan", if f == 0 { "Stopped (0 %)".into() } else { format!("{f} %") }, None));
        }
        let o = ui.cursor().min;
        let (rects, h) = widgets::tile_grid(o, w, tiles.len(), 170.0, 64.0, 12.0);
        ui.allocate_exact_size(vec2(w, h), Sense::hover());
        for (r, (c, v, d)) in rects.iter().zip(&tiles) {
            widgets::tile(&p, *r, c, v, *d);
        }
        if n.temp.is_none() && g.vendor == 0x10de {
            section(ui, "Temperature, clocks and power need the NVIDIA driver's NVML library");
        }

        ui.add_space(GAP);
        let groups = grouped(s);
        let mut by: Vec<&Group> = groups.iter().filter(|g| g.vram > 0).collect();
        by.sort_by(|a, b| b.vram.cmp(&a.vram));
        let rows: Vec<(&Group, f32, String)> = by.iter().take(10).map(|g| (*g, g.vram as f32, fmt::bytes(g.vram))).collect();
        let (r, _) = ui.allocate_exact_size(vec2(w, 60.0 + rows.len() as f32 * 30.0), Sense::hover());
        top_list(ui, r, "Biggest VRAM users (dedicated GPU memory, grouped by name)", VRAM_COL, &rows);
        ui.add_space(GAP);
    });
    None
}

fn temp_label(v: &[f32]) -> String {
    v.first().map(|t| format!("{t:.0} °C")).unwrap_or_default()
}

fn watt_label(v: &[f32]) -> String {
    v.first().map(|w| format!("{w:.0} W")).unwrap_or_default()
}

fn ghz(mhz: f32) -> String {
    format!("{:.2} GHz", mhz / 1000.0)
}

fn mhz_label(v: &[f32]) -> String {
    v.first().map(|m| ghz(*m)).unwrap_or_default()
}

/// A labelled graph well (like the Disk page's): returns the area to draw the graph in
fn well_panel(p: &Painter, r: Rect, title: &str, col: Color32, right: &str) -> Rect {
    p.rect_filled(r, 8.0, well());
    p.rect_stroke(r, 8.0, Stroke::new(1.0_f32, border()));
    caption(p, r.min.x + 8.0, r.min.y + 16.0, title, col);
    p.text(pos2(r.max.x - 12.0, r.min.y + 16.0), Align2::RIGHT_CENTER, right, reg(10.5), txt_faint());
    Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 30.0), pos2(r.max.x - 1.0, r.max.y - 6.0))
}

/// Tiles with an optional tooltip each
fn tiles_tip(ui: &mut Ui, w: f32, tiles: &[(String, String, Option<Color32>, Option<&str>)]) {
    let p = ui.painter().clone();
    let o = ui.cursor().min;
    let (rects, th) = widgets::tile_grid(o, w, tiles.len(), 170.0, 64.0, 12.0);
    ui.allocate_exact_size(vec2(w, th), Sense::hover());
    for (i, (tr, (c, v, dcol, tip))) in rects.iter().zip(tiles).enumerate() {
        widgets::tile(&p, *tr, c, v, *dcol);
        if let Some(t) = tip {
            ui.interact(*tr, ui.id().with(("tile_tip", i, c.as_str())), Sense::hover()).on_hover_text(*t);
        }
    }
}

/// A card with a title and a paragraph that wraps
fn note_card(ui: &mut Ui, w: f32, title: &str, col: Color32, body: &str) {
    let p = ui.painter().clone();
    let g = p.layout(body.to_string(), reg(12.5), txt_dim(), w - 36.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 58.0 + g.size().y), Sense::hover());
    widgets::card(&p, r, card());
    caption_w(&p, r.min.x + 18.0, r.min.y + 24.0, title, col, r.width() - 36.0);
    p.galley(pos2(r.min.x + 18.0, r.min.y + 42.0), g, txt_dim());
}

const NO_CPU_TEMP: &str = "Windows has no built-in way to read the CPU's own temperature sensor. Tools that show it load a kernel driver, which anti-cheat can flag. Purgeon never loads one, so there is nothing extra to install and nothing for anti-cheat to object to.";

pub fn sensors(ui: &mut Ui, s: &Snapshot, scroll: f32) -> Option<Page> {
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let w = ui.available_width();
        let ctx = ui.ctx().clone();
        header(ui, "Sensors", "Read through Windows and the graphics driver. Purgeon installs no drivers.");
        let p = ui.painter().clone();

        // ---- GPUs (NVIDIA through NVML, the driver's own monitoring library)
        for (i, g) in s.gpus.iter().enumerate() {
            if g.vendor == 0x1414 {
                continue; // Microsoft Basic Render Driver
            }
            let n = &g.sensors;
            if n.temp.is_none() && n.power_w.is_none() {
                let why = if g.vendor == 0x10de {
                    "The NVIDIA driver's monitoring library (nvml.dll) could not be opened, so temperature, power and clocks are unavailable. Reinstalling or updating the NVIDIA driver usually fixes this."
                } else {
                    "Temperature, power and clocks are unavailable for this GPU. Windows only reports its usage and memory, which are on the GPU page. NVIDIA cards are supported today."
                };
                note_card(ui, w, &g.name, GPU_COL, why);
                ui.add_space(GAP);
                continue;
            }
            let h = &s.h_sens[i];
            let (r, _) = ui.allocate_exact_size(vec2(w, 300.0), Sense::hover());
            widgets::card(&p, r, card());
            caption_w(&p, r.min.x + 18.0, r.min.y + 24.0, &g.name, GPU_COL, r.width() - 36.0);
            let t = n.temp.unwrap_or(0.0);
            let tv = glide(&ctx, egui::Id::new(("sens_t", g.luid)), t, 10.0);
            let sub = match (n.throttle_text(), n.slowdown_temp) {
                (Some(why), _) if !why.starts_with("Idle") => why.to_string(),
                (_, Some(sd)) => format!("slows itself down at {sd:.0} °C"),
                _ => String::new(),
            };
            big(&p, r.min.x + 18.0, r.min.y + 60.0, &format!("{tv:.0} °C"), &sub, 30.0);

            let half = (r.width() - 36.0 - GAP) / 2.0;
            let pa = Rect::from_min_size(pos2(r.min.x + 18.0, r.min.y + 92.0), vec2(half, r.height() - 92.0 - 16.0));
            let pb = Rect::from_min_size(pos2(pa.max.x + GAP, pa.min.y), pa.size());
            let tmax = n.slowdown_temp.map_or(100.0, |sd| (sd + 10.0).max(100.0));
            let ga = well_panel(&p, pa, "Temperature", temp_col(t), &format!("{tmax:.0} °C"));
            let series = [Series { data: &h[0], col: temp_col(t), fill: true, width: 1.6 }];
            graph::paint(&p, ga, &Graph { series: &series, max: tmax, scroll, grid: true, hover: hover_in(ui, ga), interval: s.interval, label: &temp_label });
            let pmax = n.power_limit_w.unwrap_or_else(|| h[1].iter().fold(50.0_f32, |m, v| m.max(*v)) * 1.15);
            let gb = well_panel(&p, pb, "Power", WARN_COL, &format!("{pmax:.0} W"));
            let series = [Series { data: &h[1], col: WARN_COL, fill: true, width: 1.6 }];
            graph::paint(&p, gb, &Graph { series: &series, max: pmax, scroll, grid: true, hover: hover_in(ui, gb), interval: s.interval, label: &watt_label });

            ui.add_space(12.0);
            let na = || "Unavailable".to_string();
            let with_max = |c: Option<u32>, m: Option<u32>| match (c, m) {
                (Some(c), Some(m)) if m > 0 => format!("{c} / {m} MHz"),
                (Some(c), _) => format!("{c} MHz"),
                _ => na(),
            };
            let mut tiles: Vec<(String, String, Option<Color32>, Option<&str>)> = vec![
                ("Temperature".into(), n.temp.map_or_else(na, |t| format!("{t:.0} °C")), n.temp.map(temp_col), None),
                (
                    "Slows down at".into(),
                    n.slowdown_temp.map_or_else(na, |t| format!("{t:.0} °C")),
                    None,
                    Some("Above this temperature the GPU lowers its clocks to protect itself, and games lose frames."),
                ),
                (
                    "Power".into(),
                    match (n.power_w, n.power_limit_w) {
                        (Some(p), Some(l)) => format!("{p:.0} / {l:.0} W"),
                        (Some(p), None) => format!("{p:.0} W"),
                        _ => na(),
                    },
                    Some(WARN_COL),
                    Some("Power the card draws now, and the most it is allowed to draw."),
                ),
                (
                    "Fan".into(),
                    n.fan_pct.map_or_else(na, |f| if f == 0 { "Stopped (0 %)".into() } else { format!("{f} %") }),
                    None,
                    Some("Most cards stop their fans when cool, so 0 % at idle is normal."),
                ),
                ("Core clock".into(), with_max(n.core_mhz, n.core_max_mhz), None, Some("Current graphics clock / the highest it can boost to.")),
                ("Memory clock".into(), with_max(n.mem_mhz, n.mem_max_mhz), None, None),
                ("Video clock".into(), n.video_mhz.map_or_else(na, |c| format!("{c} MHz")), None, Some("Clock of the video encoder and decoder.")),
                (
                    "Performance state".into(),
                    n.pstate.map_or_else(na, |ps| match ps {
                        0 => "P0 (full speed)".into(),
                        1..=4 => format!("P{ps} (fast)"),
                        5..=7 => format!("P{ps} (light load)"),
                        _ => format!("P{ps} (idle)"),
                    }),
                    None,
                    Some("P0 is full speed. Higher numbers are lower-power states the driver uses when there is little to do."),
                ),
                (
                    "PCIe link".into(),
                    match (n.pcie, n.pcie_max) {
                        (Some((g, l)), Some((mg, ml))) if (g, l) != (mg, ml) => format!("Gen {g} x{l} (max Gen {mg} x{ml})"),
                        (Some((g, l)), _) => format!("Gen {g} x{l}"),
                        _ => na(),
                    },
                    None,
                    Some("The card drops to a slower link to save power when idle. Under load it should reach its maximum."),
                ),
            ];
            if n.throttle.is_some() {
                let why = n.throttle_text();
                let col = match why {
                    Some(w) if w.starts_with("Too hot") || w.starts_with("Hardware") => Some(DANGER),
                    Some(w) if w.starts_with("At its power") => Some(WARN_COL),
                    _ => None,
                };
                let v = match why {
                    None => "Nothing (full boost)".into(),
                    Some(w) if w.starts_with("Idle") => "Idle".into(),
                    Some(w) if w.starts_with("At its power") => "Power limit".into(),
                    Some(w) if w.starts_with("Too hot") => "Temperature".into(),
                    Some(_) => "Hardware slowdown".into(),
                };
                tiles.push((
                    "Clock held back by".into(),
                    v,
                    col,
                    Some("What is keeping the GPU below its top clock right now, as the NVIDIA driver reports it. Power limit under full load is normal."),
                ));
            }
            tiles_tip(ui, w, &tiles);
            ui.add_space(GAP + 8.0);
        }

        // ---- CPU: clocks from Windows; temperature only where the PC exposes a thermal zone
        let (r, _) = ui.allocate_exact_size(vec2(w, 260.0), Sense::hover());
        widgets::card(&p, r, card());
        caption_w(&p, r.min.x + 18.0, r.min.y + 24.0, &s.info.cpu_name, CPU_COL, r.width() - 36.0);
        let avg = glide(&ctx, egui::Id::new("sens_mhz"), s.clocks.avg_mhz, 10.0);
        let base = s.info.base_mhz as f32;
        big(&p, r.min.x + 18.0, r.min.y + 60.0, &ghz(avg), &format!("average clock  ·  base {}", ghz(base)), 30.0);
        let pr = Rect::from_min_size(pos2(r.min.x + 18.0, r.min.y + 92.0), vec2(r.width() - 36.0, r.height() - 92.0 - 16.0));
        let peak = s.h_mhz.iter().fold(base * 1.3, |m, v| m.max(*v)) * 1.1;
        let peak = glide(&ctx, egui::Id::new("sens_mhz_peak"), peak, 4.0);
        let gr = well_panel(&p, pr, "Average clock", CPU_COL, &ghz(peak));
        let series = [Series { data: &s.h_mhz, col: CPU_COL, fill: true, width: 1.6 }];
        graph::paint(&p, gr, &Graph { series: &series, max: peak, scroll, grid: true, hover: hover_in(ui, gr), interval: s.interval, label: &mhz_label });
        ui.add_space(12.0);
        let fastest = s.clocks.mhz.iter().copied().fold(0.0_f32, f32::max);
        let slowest = s.clocks.mhz.iter().copied().fold(f32::MAX, f32::min);
        let mut tiles: Vec<(String, String, Option<Color32>, Option<&str>)> = vec![
            ("Average clock".into(), ghz(s.clocks.avg_mhz), Some(CPU_COL), None),
            ("Fastest thread".into(), if fastest > 0.0 { ghz(fastest) } else { "Unavailable".into() }, None, None),
            ("Slowest thread".into(), if slowest < f32::MAX { ghz(slowest) } else { "Unavailable".into() }, None, Some("Idle cores run slower or sleep to save power.")),
            ("Base clock".into(), ghz(base), None, Some("The rated speed. Boost clocks above it are normal when the CPU is cool and has power to spare.")),
        ];
        if s.zones.is_empty() {
            tiles.push(("CPU temperature".into(), "Needs a driver".into(), None, Some(NO_CPU_TEMP)));
        }
        for (name, c) in &s.zones {
            tiles.push((
                format!("Thermal zone {name}"),
                format!("{c:.0} °C"),
                Some(temp_col(*c)),
                Some("A motherboard temperature reported through ACPI. The PC maker decides what it measures; it is often near the CPU but is not the CPU's own sensor, and on some PCs it never changes."),
            ));
        }
        tiles_tip(ui, w, &tiles);
        ui.add_space(GAP + 8.0);

        note_card(
            ui,
            w,
            "Not shown on purpose: CPU temperature, motherboard sensors and case fans",
            txt_faint(),
            "Reading these needs a kernel driver, which anti-cheat can flag. Purgeon never loads one.",
        );
        ui.add_space(GAP);
    });
    None
}
