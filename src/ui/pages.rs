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
    p.text(pos2(r.min.x, r.min.y + 40.0), Align2::LEFT_CENTER, sub, reg(12.5), txt_faint());
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    ui.painter().text(pos2(r.min.x + 2.0, r.center().y), Align2::LEFT_CENTER, title, sb(13.5), txt_dim());
}

/// Caption with a coloured dot
fn caption(p: &Painter, x: f32, y: f32, text: &str, col: Color32) -> f32 {
    p.circle_filled(pos2(x + 4.0, y), 4.0, col);
    let r = p.text(pos2(x + 14.0, y), Align2::LEFT_CENTER, text, sb(12.5), txt_dim());
    r.max.x
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
    count: u32,
    cpu: f32,
    mem: u64,
}

/// Processes summed by image name ("chrome.exe ×24")
fn grouped(s: &Snapshot) -> Vec<Group> {
    let mut m: HashMap<&str, Group> = HashMap::new();
    for p in &s.procs {
        let g = m.entry(p.name.as_str()).or_insert_with(|| Group { name: p.name.clone(), count: 0, cpu: 0.0, mem: 0 });
        g.count += 1;
        g.cpu += p.cpu;
        g.mem += p.private_ws;
    }
    m.into_values().collect()
}

/// A card listing the top groups with a bar each
fn top_list(ui: &mut Ui, r: Rect, title: &str, col: Color32, rows: &[(&Group, f32, String)]) {
    let p = ui.painter().clone();
    widgets::card(&p, r, card());
    caption(&p, r.min.x + 18.0, r.min.y + 24.0, title, col);
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
        widgets::avatar(&p, Rect::from_center_size(pos2(rr.min.x + 20.0, rr.center().y), vec2(18.0, 18.0)), &g.name);
        p.text(pos2(rr.max.x - 10.0, rr.center().y), Align2::RIGHT_CENTER, txtv, sb(12.5), txt());
        let bw = (rr.width() * 0.28).min(140.0);
        let br = Rect::from_min_size(pos2(rr.max.x - 86.0 - bw, rr.center().y - 2.5), vec2(bw, 5.0));
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
        p.text(pos2(r.min.x + 18.0, r.min.y + 92.0), Align2::LEFT_CENTER, sub, reg(12.0), txt_faint());
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
        p.text(pos2(r.min.x + 18.0, r.min.y + 92.0), Align2::LEFT_CENTER, sub, reg(12.0), txt_faint());
        let gr = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 112.0), pos2(r.max.x - 1.0, r.max.y - 12.0));
        let series = [Series { data: &s.h_mem, col: MEM_COL, fill: true, width: 1.6 }];
        graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: false, hover: hov(ui, gr), interval: s.interval, label: &pct_label });
        if resp.clicked() {
            go = Some(Page::Memory);
        }

        // GPU (next)
        let (resp, r) = widgets::card_button(ui, rect(2), egui::Id::new("ov_gpu"));
        caption(&p, r.min.x + 18.0, r.min.y + 24.0, "GPU", GPU_COL);
        big(&p, r.min.x + 18.0, r.min.y + 62.0, "—", "", 32.0);
        p.text(pos2(r.min.x + 18.0, r.min.y + 92.0), Align2::LEFT_CENTER, "VRAM per process is coming next", reg(12.0), txt_faint());
        let gr = Rect::from_min_max(pos2(r.min.x + 18.0, r.min.y + 120.0), pos2(r.max.x - 18.0, r.max.y - 22.0));
        for k in 0..4 {
            let y = gr.min.y + gr.height() * k as f32 / 3.0;
            p.line_segment([pos2(gr.min.x, y), pos2(gr.max.x, y)], Stroke::new(1.0_f32, wa(7)));
        }
        widgets::icon(&p, Icon::Gpu, gr.center(), txt_faint());
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
        let tiles: [(&str, String); 8] = [
            ("Utilization", format!("{:.1}%", s.cpu)),
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
            p.text(pos2(r.min.x + 12.0, r.min.y + 16.0), Align2::LEFT_CENTER, format!("CPU {i}"), reg(11.5), txt_faint());
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
