//! Overview, CPU and Memory pages, and the placeholders for what's next.

use super::graph::{self, Graph, Series};
use super::widgets::{self, Icon};
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
fn caption_w(p: &Painter, x: f32, y: f32, text: &str, col: Color32, max_w: f32) -> f32 {
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
    mem: u64,
    vram: u64,
    disk_r: f64,
    disk_w: f64,
}

/// Processes summed by image name ("chrome.exe ×24")
fn grouped(s: &Snapshot) -> Vec<Group> {
    let mut m: HashMap<&str, Group> = HashMap::new();
    for p in &s.procs {
        let g = m.entry(p.name.as_str()).or_insert_with(|| Group { name: p.name.clone(), path: None, count: 0, cpu: 0.0, mem: 0, vram: 0, disk_r: 0.0, disk_w: 0.0 });
        g.count += 1;
        if g.path.is_none() {
            g.path = p.path.clone();
        }
        g.cpu += p.cpu;
        g.mem += p.private_ws;
        g.vram += p.vram;
        g.disk_r += p.disk_read;
        g.disk_w += p.disk_write;
    }
    m.into_values().collect()
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

pub fn overview(ui: &mut Ui, s: &Snapshot, scroll: f32) -> Option<Page> {
    let mut go = None;
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let w = ui.available_width();
        header(ui, "Overview", &format!("{}  ·  up {}", s.info.cpu_name, fmt::uptime(s.uptime)));
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
        let sub = format!("{} cores  ·  {} threads", s.info.physical.max(1), s.info.logical);
        widgets::fit(&p, pos2(r.min.x + 18.0, r.min.y + 92.0), Align2::LEFT_CENTER, &sub, reg(12.0), r.width() - 36.0, txt_faint());
        let gr = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 112.0), pos2(r.max.x - 1.0, r.max.y - 12.0));
        let series = [Series { data: &s.h_cpu, col: CPU_COL, fill: true, width: 1.6 }];
        graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: false, hover: hov(ui, gr), interval: s.interval, label: &pct_label });
        if resp.clicked() {
            go = Some(Page::Cpu);
        }

        // Memory
        let (resp, r) = widgets::card_button(ui, rect(1), egui::Id::new("ov_mem"));
        caption(&p, r.min.x + 18.0, r.min.y + 24.0, "Memory", MEM_COL);
        let used = glide(&ctx, egui::Id::new("ov_mem_v"), s.mem.used() as f32 / 1073741824.0, 10.0);
        big(&p, r.min.x + 18.0, r.min.y + 62.0, &format!("{used:.1}"), &format!("/ {} GB", fmt::gb(s.mem.total)), 32.0);
        let sub = format!("{:.0}% in use  ·  {} committed", s.mem.used_frac() * 100.0, fmt::bytes(s.mem.commit));
        widgets::fit(&p, pos2(r.min.x + 18.0, r.min.y + 92.0), Align2::LEFT_CENTER, &sub, reg(12.0), r.width() - 36.0, txt_faint());
        let gr = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 112.0), pos2(r.max.x - 1.0, r.max.y - 12.0));
        let series = [Series { data: &s.h_mem, col: MEM_COL, fill: true, width: 1.6 }];
        graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: false, hover: hov(ui, gr), interval: s.interval, label: &pct_label });
        if resp.clicked() {
            go = Some(Page::Memory);
        }

        // GPU (the main adapter)
        let (resp, r) = widgets::card_button(ui, rect(2), egui::Id::new("ov_gpu"));
        caption(&p, r.min.x + 18.0, r.min.y + 24.0, "GPU", GPU_COL);
        match main_gpu(s) {
            Some(i) => {
                let g = &s.gpus[i];
                let v = glide(&ctx, egui::Id::new("ov_gpu_v"), g.util, 10.0);
                let temp = g.sensors.temp.map(|t| format!("{t:.0}°C")).unwrap_or_default();
                big(&p, r.min.x + 18.0, r.min.y + 62.0, &format!("{v:.0}%"), &temp, 32.0);
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

pub fn soon(ui: &mut Ui, ic: Icon, title: &str, points: &[&str]) -> Option<Page> {
    header(ui, title, "Coming in the next update");
    let w = ui.available_width().min(620.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 90.0 + points.len() as f32 * 30.0), Sense::hover());
    let p = ui.painter();
    widgets::card(p, r, card());
    let c = pos2(r.min.x + 38.0, r.min.y + 38.0);
    p.circle_filled(c, 20.0, wa(10));
    widgets::icon(p, ic, c, GPU_COL);
    p.text(pos2(r.min.x + 72.0, r.min.y + 38.0), Align2::LEFT_CENTER, "What this page will show", sb(14.0), txt());
    for (i, t) in points.iter().enumerate() {
        let y = r.min.y + 80.0 + i as f32 * 30.0;
        p.circle_filled(pos2(r.min.x + 40.0, y), 2.5, txt_faint());
        p.text(pos2(r.min.x + 54.0, y), Align2::LEFT_CENTER, *t, reg(12.5), txt_dim());
    }
    None
}
