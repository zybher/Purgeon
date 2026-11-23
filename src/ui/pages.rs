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
    /// GPU use, 0..100 (Windows' estimate)
    gpu: f32,
    /// All I/O (disk, network, devices) per second: known without administrator
    io: f64,
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
            gpu: 0.0,
            io: 0.0,
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
        g.gpu = (g.gpu + p.gpu).min(100.0);
        g.io += p.io_read + p.io_write;
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
        // a click opens the program's details
        let resp = ui.interact(rr, ui.id().with((title, i)), Sense::click());
        let h = ease(ui.ctx(), resp.id.with("h"), resp.hovered());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            super::detail::open_group(ui.ctx(), &g.name);
        }
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

/// "Who's using it": the programs using the most of a page's resource, at the top of the page.
/// Click one for its details.
#[allow(clippy::too_many_arguments)]
fn who_card(ui: &mut Ui, w: f32, title: &str, col: Color32, note: &str, rows: &[(&Group, f32, String)], value_w: f32, empty: &str) {
    let rows = &rows[..rows.len().min(6)];
    let (r, _) = ui.allocate_exact_size(vec2(w, 60.0 + rows.len().max(1) as f32 * 30.0), Sense::hover());
    top_list_w(ui, r, title, col, rows, value_w);
    let p = ui.painter();
    if !note.is_empty() {
        let tw = p.layout_no_wrap(title.to_string(), sb(12.5), txt()).size().x;
        widgets::fit(p, pos2(r.max.x - 18.0, r.min.y + 24.0), Align2::RIGHT_CENTER, note, reg(11.0), r.width() - tw - 80.0, txt_faint());
    }
    if rows.is_empty() {
        p.text(pos2(r.min.x + 18.0, r.min.y + 60.0), Align2::LEFT_CENTER, empty, reg(12.5), txt_faint());
    }
    ui.add_space(GAP);
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

    let (r, _) = ui.allocate_exact_size(vec2(w, if hogs.is_empty() { 84.0 } else { 140.0 }), Sense::hover());
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
        let resp = ui.interact(t, egui::Id::new(("top_app", i)), Sense::click()).on_hover_text(format!("{}  ·  {procs}", g.name));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            super::detail::open_group(ui.ctx(), &g.name);
        }
    }
}

/// The four cards at the top of the dashboard; one at a time can be opened for more detail
#[derive(Clone, Copy, PartialEq)]
enum Part {
    Cpu,
    Mem,
    Disk,
    Gpu,
}

/// The disk to show on the dashboard: the one that worked hardest over the graph's span (so
/// the card doesn't hop between disks from one second to the next)
fn busiest_disk(s: &Snapshot) -> Option<usize> {
    let load = |i: usize| s.h_disk.get(i).map_or(0.0, |h| h[0].iter().sum::<f32>());
    (0..s.disks.len()).max_by(|&a, &b| load(a).total_cmp(&load(b)))
}

/// A small chevron: pointing down when the card is open, right when it can be opened
fn chevron(p: &Painter, c: egui::Pos2, open: f32, col: Color32) {
    let a = open * std::f32::consts::FRAC_PI_2;
    let rot = |x: f32, y: f32| pos2(c.x + x * a.cos() - y * a.sin(), c.y + x * a.sin() + y * a.cos());
    let s = Stroke::new(1.6_f32, col);
    p.line_segment([rot(-2.5, -4.5), rot(2.0, 0.0)], s);
    p.line_segment([rot(2.0, 0.0), rot(-2.5, 4.5)], s);
}

/// A small chevron pointing right: "opens more"
pub(super) fn chevron_right(p: &Painter, c: egui::Pos2, col: Color32) {
    chevron(p, c, 0.0, col);
}

pub fn dashboard(ui: &mut Ui, s: &Snapshot, scroll: f32) -> Option<Page> {
    let mut go = None;
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        // the page reaches the window's edge so the scroll bar sits there; the content keeps its margin
        ui.set_max_width(ui.available_width() - super::PAGE_PAD);
        ui.add_space(super::PAGE_TOP);
        let w = ui.available_width();
        let ctx = ui.ctx().clone();
        header(ui, "Dashboard", &format!("{}  ·  up {}", s.info.cpu_name, fmt::uptime(s.uptime)));

        // ---- CPU, Memory, Disk, GPU: click one to open its details below the row ------------
        let open_id = egui::Id::new("dash_open");
        let saved = ctx.data(|d| d.get_temp::<Option<Part>>(open_id));
        // dev captures can start with a card open: PURGEON_DASH_OPEN=cpu|mem|disk|gpu
        let start = || match std::env::var("PURGEON_DASH_OPEN").ok()?.as_str() {
            "cpu" => Some(Part::Cpu),
            "mem" => Some(Part::Mem),
            "disk" => Some(Part::Disk),
            "gpu" => Some(Part::Gpu),
            _ => None,
        };
        let mut open: Option<Part> = saved.unwrap_or_else(start);
        let (row, _) = ui.allocate_exact_size(vec2(w, 200.0), Sense::hover());
        let cw = (w - GAP * 3.0) / 4.0;
        let rect = |i: usize| Rect::from_min_size(pos2(row.min.x + i as f32 * (cw + GAP), row.min.y), vec2(cw, row.height()));
        let p = ui.painter().clone();
        let disk = busiest_disk(s);
        let gpu = main_gpu(s);
        let parts = [(Part::Cpu, "CPU", CPU_COL, "ov_cpu"), (Part::Mem, "Memory", MEM_COL, "ov_mem"), (Part::Disk, "Disk", DISK_COL, "ov_disk"), (Part::Gpu, "GPU", GPU_COL, "ov_gpu")];
        for (i, &(part, title, col, key)) in parts.iter().enumerate() {
            let id = egui::Id::new(key);
            let (resp, r) = widgets::card_button(ui, rect(i), id);
            let is_open = open == Some(part);
            let o = ease(&ctx, id.with("open"), is_open);
            if o > 0.01 {
                p.rect_stroke(r, widgets::R_CARD, Stroke::new(1.5_f32, col.gamma_multiply(o)));
            }
            chevron(&p, pos2(r.max.x - 20.0, r.min.y + 24.0), o, txt_faint());
            caption(&p, r.min.x + 18.0, r.min.y + 24.0, title, col);
            let gr = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 104.0), pos2(r.max.x - 1.0, r.max.y - 10.0));
            let (x, vy, sy) = (r.min.x + 18.0, r.min.y + 58.0, r.min.y + 86.0);
            let sub_w = r.width() - 36.0;
            match part {
                Part::Cpu => {
                    let v = glide(&ctx, egui::Id::new("ov_cpu_v"), s.cpu, 10.0);
                    big(&p, x, vy, &format!("{v:.0}%"), "", 30.0);
                    // where the overlay's numbers fly to when it grows into this window
                    super::morph::mark("cpu", pos2(x, vy), 30.0);
                    let speed = if s.clocks.avg_mhz > 0.0 { format!("  ·  {:.2} GHz", s.clocks.avg_mhz / 1000.0) } else { String::new() };
                    let sub = format!("{} cores{speed}", s.info.physical.max(1));
                    widgets::fit(&p, pos2(x, sy), Align2::LEFT_CENTER, &sub, reg(12.0), sub_w, txt_faint());
                    let series = [Series { data: &s.h_cpu, col: CPU_COL, fill: true, width: 1.6 }];
                    graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: false, hover: hover_in(ui, gr), interval: s.interval, label: &pct_label });
                }
                Part::Mem => {
                    let used = glide(&ctx, egui::Id::new("ov_mem_v"), s.mem.used() as f32 / 1073741824.0, 10.0);
                    big(&p, x, vy, &format!("{used:.1}"), &format!("/ {} GB", fmt::gb(s.mem.total)), 30.0);
                    let sub = format!("{:.0}% in use", s.mem.used_frac() * 100.0);
                    super::morph::mark("ram", pos2(x, sy), 12.0);
                    widgets::fit(&p, pos2(x, sy), Align2::LEFT_CENTER, &sub, reg(12.0), sub_w, txt_faint());
                    let series = [Series { data: &s.h_mem, col: MEM_COL, fill: true, width: 1.6 }];
                    graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: false, hover: hover_in(ui, gr), interval: s.interval, label: &pct_label });
                }
                Part::Disk => match disk {
                    Some(di) => {
                        let d = &s.disks[di];
                        let v = glide(&ctx, egui::Id::new("ov_disk_v"), d.active, 10.0);
                        big(&p, x, vy, &format!("{v:.0}%"), "active", 30.0);
                        let name = if d.letters.is_empty() { format!("Disk {}", d.index) } else { d.letters.join(" ") };
                        let sub = format!("{name}  ·  {} read  ·  {} write", fmt::rate(d.read), fmt::rate(d.write));
                        widgets::fit(&p, pos2(x, sy), Align2::LEFT_CENTER, &sub, reg(12.0), sub_w, txt_faint());
                        let series = [Series { data: &s.h_disk[di][0], col: DISK_COL, fill: true, width: 1.6 }];
                        graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: false, hover: hover_in(ui, gr), interval: s.interval, label: &pct_label });
                    }
                    None => {
                        big(&p, x, vy, "—", "", 30.0);
                        p.text(pos2(x, sy), Align2::LEFT_CENTER, "No disk counters found", reg(12.0), txt_faint());
                    }
                },
                Part::Gpu => match gpu {
                    Some(gi) => {
                        let g = &s.gpus[gi];
                        let v = glide(&ctx, egui::Id::new("ov_gpu_v"), g.util, 10.0);
                        let temp = g.sensors.temp.map(|t| format!("{t:.0}°C")).unwrap_or_default();
                        big(&p, x, vy, &format!("{v:.0}%"), &temp, 30.0);
                        super::morph::mark("gpu", pos2(x, vy), 30.0);
                        let vw = p.layout_no_wrap(format!("{v:.0}%"), sb(30.0), txt()).size().x;
                        super::morph::mark("gpu_temp", pos2(x + vw + 4.0, vy + 30.0 * 0.12), 30.0 * 0.48);
                        let sub = format!("{} / {} VRAM", fmt::bytes(g.dedicated_used), fmt::bytes(g.dedicated_total));
                        widgets::fit(&p, pos2(x, sy), Align2::LEFT_CENTER, &sub, reg(12.0), sub_w, txt_faint());
                        let series = [
                            Series { data: &s.h_gpu[gi], col: GPU_COL, fill: true, width: 1.6 },
                            Series { data: &s.h_vram[gi], col: VRAM_COL, fill: false, width: 1.1 },
                        ];
                        graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: false, hover: hover_in(ui, gr), interval: s.interval, label: &pct_label });
                    }
                    None => {
                        big(&p, x, vy, "—", "", 30.0);
                        p.text(pos2(x, sy), Align2::LEFT_CENTER, "No GPU counters found", reg(12.0), txt_faint());
                    }
                },
            }
            if resp.clicked() {
                open = if is_open { None } else { Some(part) };
            }
        }
        ctx.data_mut(|d| d.insert_temp(open_id, open));

        // ---- the opened card's details --------------------------------------------------------
        if let Some(part) = open {
            ui.add_space(GAP);
            if let Some(page) = details(ui, s, part, w, scroll) {
                go = Some(page);
            }
        }

        // ---- what is using the PC ---------------------------------------------------------------
        ui.add_space(GAP);
        summary(ui, s, w);
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

/// The opened card's panel: a bigger graph with its scale, the numbers behind the card, and
/// a link to the full page
fn details(ui: &mut Ui, s: &Snapshot, part: Part, w: f32, scroll: f32) -> Option<Page> {
    let dash = || "—".to_string();
    let gb = |b: u64| format!("{:.1} GB", b as f64 / 1073741824.0);
    let (title, col, page, tiles): (String, Color32, Page, Vec<(String, String)>) = match part {
        Part::Cpu => {
            let speed = if s.clocks.avg_mhz > 0.0 { format!("{:.2} GHz", s.clocks.avg_mhz / 1000.0) } else { dash() };
            let base = if s.info.base_mhz > 0 { format!("{:.2} GHz", s.info.base_mhz as f32 / 1000.0) } else { dash() };
            (
                s.info.cpu_name.clone(),
                CPU_COL,
                Page::Cpu,
                vec![
                    ("Utilization".into(), format!("{:.1}%", s.cpu)),
                    ("Speed".into(), speed),
                    ("Base speed".into(), base),
                    ("Kernel time".into(), format!("{:.1}%", s.kernel)),
                    ("Cores / threads".into(), format!("{} / {}", s.info.physical.max(1), s.info.logical)),
                    ("Processes".into(), fmt::thousands(s.nprocs as u64)),
                    ("Threads".into(), fmt::thousands(s.threads as u64)),
                    ("Handles".into(), fmt::thousands(s.handles as u64)),
                ],
            )
        }
        Part::Mem => {
            let m = &s.mem;
            let mut t = vec![
                ("In use".into(), format!("{}  ({:.0}%)", gb(m.used()), m.used_frac() * 100.0)),
                ("Available".into(), gb(m.avail)),
                ("Committed".into(), format!("{} / {}", gb(m.commit), gb(m.commit_limit))),
                ("Cached".into(), gb(m.cache)),
                ("Paged pool".into(), fmt::bytes(m.paged)),
                ("Non-paged pool".into(), fmt::bytes(m.nonpaged)),
            ];
            if let Some(l) = &m.lists {
                t.push(("Standby".into(), gb(l.standby)));
                t.push(("Modified".into(), fmt::bytes(l.modified)));
            }
            (format!("{} GB installed", fmt::gb(m.total)), MEM_COL, Page::Memory, t)
        }
        Part::Disk => {
            let mut t = Vec::new();
            for d in &s.disks {
                let name = if d.letters.is_empty() { format!("Disk {}", d.index) } else { format!("Disk {} ({})", d.index, d.letters.join(" ")) };
                t.push((name, format!("{:.0}%  ·  {:.1} ms", d.active, d.resp_ms)));
                t.push(("Read / write".into(), format!("{}  /  {}", fmt::rate(d.read), fmt::rate(d.write))));
            }
            (if s.disks.len() == 1 { "1 physical disk".to_string() } else { format!("{} physical disks", s.disks.len()) }, DISK_COL, Page::Disk, t)
        }
        Part::Gpu => {
            let Some(g) = main_gpu(s).map(|i| &s.gpus[i]) else { return None };
            let sn = &g.sensors;
            (
                g.name.clone(),
                GPU_COL,
                Page::Gpu,
                vec![
                    ("Utilization".into(), format!("{:.0}%", g.util)),
                    ("Temperature".into(), sn.temp.map_or_else(dash, |t| format!("{t:.0}°C"))),
                    ("Dedicated memory".into(), format!("{} / {}", fmt::bytes(g.dedicated_used), fmt::bytes(g.dedicated_total))),
                    ("Shared memory".into(), format!("{} / {}", fmt::bytes(g.shared_used), fmt::bytes(g.shared_total))),
                    ("Core clock".into(), sn.core_mhz.map_or_else(dash, |m| format!("{m} MHz"))),
                    ("Memory clock".into(), sn.mem_mhz.map_or_else(dash, |m| format!("{m} MHz"))),
                    ("Power".into(), sn.power_w.map_or_else(dash, |p| format!("{p:.0} W"))),
                    ("Fan".into(), sn.fan_pct.map_or_else(dash, |f| format!("{f}%"))),
                ],
            )
        }
    };

    let pad = 18.0;
    let graph_h = 170.0;
    let (_, tiles_h) = widgets::tile_grid(pos2(0.0, 0.0), w - pad * 2.0, tiles.len(), 150.0, 64.0, 12.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 44.0 + graph_h + 14.0 + tiles_h + pad), Sense::hover());
    let p = ui.painter().clone();
    widgets::card(&p, r, card());
    caption_w(&p, r.min.x + pad, r.min.y + 24.0, &title, col, r.width() - 200.0);

    // "Open the full page"
    let link = p.layout_no_wrap("Open full page".to_string(), sb(12.0), ACCENT);
    let lr = Rect::from_min_size(pos2(r.max.x - pad - link.size().x - 14.0, r.min.y + 14.0), vec2(link.size().x + 14.0, 22.0));
    let resp = ui.interact(lr, egui::Id::new("dash_more"), Sense::click());
    if resp.hovered() {
        p.rect_filled(lr, 6.0, wa(10));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    p.galley(pos2(lr.min.x + 7.0, lr.center().y - link.size().y / 2.0), link, ACCENT);
    let go = resp.clicked().then_some(page);

    // the graph, with its scale this time
    let gr = Rect::from_min_size(pos2(r.min.x + pad, r.min.y + 44.0), vec2(w - pad * 2.0, graph_h));
    p.rect_filled(gr, 8.0, well());
    let hover = hover_in(ui, gr);
    match part {
        Part::Cpu => {
            let series = [
                Series { data: &s.h_cpu, col: CPU_COL, fill: true, width: 1.8 },
                Series { data: &s.h_kernel, col: KERNEL_COL, fill: false, width: 1.2 },
            ];
            graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: true, hover, interval: s.interval, label: &pct_label });
        }
        Part::Mem => {
            let series = [
                Series { data: &s.h_mem, col: MEM_COL, fill: true, width: 1.8 },
                Series { data: &s.h_commit, col: txt_faint(), fill: false, width: 1.1 },
            ];
            graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: true, hover, interval: s.interval, label: &pct_label });
        }
        Part::Disk => {
            let series: Vec<Series> = s
                .h_disk
                .iter()
                .enumerate()
                .map(|(i, h)| Series { data: &h[0], col: if Some(i) == busiest_disk(s) { DISK_COL } else { DISK_COL.gamma_multiply(0.45) }, fill: Some(i) == busiest_disk(s), width: 1.4 })
                .collect();
            graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: true, hover, interval: s.interval, label: &pct_label });
        }
        Part::Gpu => {
            if let Some(i) = main_gpu(s) {
                let series = [
                    Series { data: &s.h_gpu[i], col: GPU_COL, fill: true, width: 1.8 },
                    Series { data: &s.h_vram[i], col: VRAM_COL, fill: false, width: 1.2 },
                ];
                graph::paint(&p, gr, &Graph { series: &series, max: 100.0, scroll, grid: true, hover, interval: s.interval, label: &pct_label });
            }
        }
    }

    let (rects, _) = widgets::tile_grid(pos2(r.min.x + pad, gr.max.y + 14.0), w - pad * 2.0, tiles.len(), 150.0, 64.0, 12.0);
    for (tr, (c, v)) in rects.iter().zip(&tiles) {
        widgets::tile(&p, *tr, c, v, None);
    }
    go
}

pub fn cpu(ui: &mut Ui, s: &Snapshot, scroll: f32) -> Option<Page> {
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        // the page reaches the window's edge so the scroll bar sits there; the content keeps its margin
        ui.set_max_width(ui.available_width() - super::PAGE_PAD);
        ui.add_space(super::PAGE_TOP);
        let w = ui.available_width();
        let ctx = ui.ctx().clone();
        header(ui, "CPU", &s.info.cpu_name);

        // who's using it, before the graphs
        let groups = grouped(s);
        let mut by: Vec<&Group> = groups.iter().filter(|g| g.cpu >= 0.05).collect();
        by.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        let threads = s.info.logical.max(1) as f32;
        let rows: Vec<(&Group, f32, String)> = by.iter().map(|g| (*g, g.cpu, format!("{}  ·  {:.1} threads", fmt::pct(g.cpu), g.cpu / 100.0 * threads))).collect();
        who_card(ui, w, "Who's using the CPU", CPU_COL, "Click a program to see what it's doing", &rows, 170.0, "Nothing is using the CPU right now");

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
        // the page reaches the window's edge so the scroll bar sits there; the content keeps its margin
        ui.set_max_width(ui.available_width() - super::PAGE_PAD);
        ui.add_space(super::PAGE_TOP);
        let w = ui.available_width();
        let ctx = ui.ctx().clone();
        let m = &s.mem;
        header(ui, "Memory", &format!("{} installed  ·  {} committed of {}", fmt::bytes(m.total), fmt::bytes(m.commit), fmt::bytes(m.commit_limit)));

        let groups = grouped(s);
        let mut by_mem: Vec<&Group> = groups.iter().collect();
        by_mem.sort_by(|a, b| b.mem.cmp(&a.mem));
        let total = m.total.max(1) as f32;
        let rows: Vec<(&Group, f32, String)> = by_mem.iter().map(|g| (*g, g.mem as f32, format!("{}  ·  {:.0}%", fmt::bytes(g.mem), g.mem as f32 / total * 100.0))).collect();
        who_card(ui, w, "Who's using memory", MEM_COL, "Click a program to see what it's doing", &rows, 150.0, "");

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
        let segs: Vec<(&str, u64, Color32)> = match m.lists {
            Some(l) => vec![
                ("In use", m.total.saturating_sub(m.avail).saturating_sub(l.modified), MEM_COL),
                ("Modified", l.modified, WARN_COL),
                ("Standby", l.standby, KERNEL_COL),
                ("Free", l.free + l.zero, Color32::from_gray(0x55)),
            ],
            None => vec![
                ("In use", m.used(), MEM_COL),
                ("Available", m.avail, Color32::from_gray(0x55)),
            ],
        };
        let bar = Rect::from_min_max(pos2(r.min.x + 18.0, r.min.y + 18.0), pos2(r.max.x - 18.0, r.min.y + 46.0));
        p.rect_filled(bar, 7.0, well());
        let total: u64 = segs.iter().map(|x| x.1).sum::<u64>().max(1);
        let mut x = bar.min.x;
        let mut lx = r.min.x + 18.0;
        for (i, (name, b, col)) in segs.iter().enumerate() {
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
        // the page reaches the window's edge so the scroll bar sits there; the content keeps its margin
        ui.set_max_width(ui.available_width() - super::PAGE_PAD);
        ui.add_space(super::PAGE_TOP);
        let w = ui.available_width();
        let ctx = ui.ctx().clone();
        let total_r: f64 = s.disks.iter().map(|d| d.read).sum();
        let total_w: f64 = s.disks.iter().map(|d| d.write).sum();
        header(ui, "Disk", &format!("{} physical disks  ·  reading {}  ·  writing {}", s.disks.len(), fmt::rate(total_r), fmt::rate(total_w)));
        if s.disks.is_empty() {
            return;
        }

        // who's using it, before the per-disk graphs
        let groups = grouped(s);
        if s.disk_trace {
            let mut by: Vec<&Group> = groups.iter().filter(|g| g.disk_r + g.disk_w > 0.0).collect();
            by.sort_by(|a, b| (b.disk_r + b.disk_w).total_cmp(&(a.disk_r + a.disk_w)));
            let rows: Vec<(&Group, f32, String)> =
                by.iter().map(|g| (*g, (g.disk_r + g.disk_w) as f32, format!("R {}  ·  W {}", fmt::rate(g.disk_r), fmt::rate(g.disk_w)))).collect();
            let (or, ow) = s.disk_orphan;
            // I/O whose thread exited before we could name its process: shown so the sum adds up
            let note = if or + ow > 0.0 { format!("+ {} from programs that already exited", fmt::rate(or + ow)) } else { String::new() };
            who_card(ui, w, "Who's using the disk", DISK_COL, &note, &rows, 170.0, "No disk activity in the last update");
        } else {
            // without administrator: all I/O (disk, network and devices together), still useful
            let mut by: Vec<&Group> = groups.iter().filter(|g| g.io >= 1.0).collect();
            by.sort_by(|a, b| b.io.total_cmp(&a.io));
            let rows: Vec<(&Group, f32, String)> = by.iter().map(|g| (*g, g.io as f32, fmt::rate(g.io))).collect();
            who_card(ui, w, "Who's moving data", DISK_COL, "Disk, network and devices together. Elevate to see disk alone", &rows, 110.0, "Nothing is moving data right now");
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
        // the page reaches the window's edge so the scroll bar sits there; the content keeps its margin
        ui.set_max_width(ui.available_width() - super::PAGE_PAD);
        ui.add_space(super::PAGE_TOP);
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

        // who's using it, before the adapters
        let groups = grouped(s);
        if s.net_trace {
            let mut by: Vec<&Group> = groups.iter().filter(|g| g.net_rx + g.net_tx > 0.0).collect();
            by.sort_by(|a, b| (b.net_rx + b.net_tx).total_cmp(&(a.net_rx + a.net_tx)));
            let rows: Vec<(&Group, f32, String)> = by.iter().map(|g| (*g, (g.net_rx + g.net_tx) as f32, format!("↓ {}  ·  ↑ {}", fmt::bits(g.net_rx), fmt::bits(g.net_tx)))).collect();
            who_card(ui, w, "Who's using the network", NET_COL, "≈ Windows' estimate per program", &rows, 190.0, "No network traffic in the last update");
        } else {
            let mut by: Vec<&Group> = groups.iter().filter(|g| g.conns.tcp > 0).collect();
            by.sort_by(|a, b| b.conns.tcp.cmp(&a.conns.tcp));
            let rows: Vec<(&Group, f32, String)> = by.iter().map(|g| (*g, g.conns.tcp as f32, format!("{} connected", g.conns.tcp))).collect();
            who_card(ui, w, "Who's connected", NET_COL, "Elevate to see download and upload speed per program", &rows, 130.0, "No open connections");
        }
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
        let (r, _) = ui.allocate_exact_size(vec2(w, 60.0 + rows.len().max(1) as f32 * 30.0), Sense::hover());
        top_list_w(ui, r, "Open connections", NET_UP_COL, &rows, 250.0);
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
        // the page reaches the window's edge so the scroll bar sits there; the content keeps its margin
        ui.set_max_width(ui.available_width() - super::PAGE_PAD);
        ui.add_space(super::PAGE_TOP);
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

        // who's using it, before the graphs (per-program GPU figures are Windows' estimates)
        let groups = grouped(s);
        if s.gpu_proc {
            let mut by: Vec<&Group> = groups.iter().filter(|g| g.gpu >= 0.1 || g.vram >= 64 << 20).collect();
            by.sort_by(|a, b| b.gpu.total_cmp(&a.gpu).then(b.vram.cmp(&a.vram)));
            let rows: Vec<(&Group, f32, String)> = by.iter().map(|g| (*g, g.gpu, format!("≈ {:.0}%  ·  {} VRAM", g.gpu, fmt::bytes(g.vram)))).collect();
            who_card(ui, w, "Who's using the GPU", GPU_COL, "≈ Windows' estimate per program", &rows, 170.0, "Nothing is using the GPU right now");
        }

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

/// A row of tiles, wrapping as needed
fn tile_row(ui: &mut Ui, w: f32, tiles: &[(String, String, Option<Color32>)]) {
    let p = ui.painter().clone();
    let o = ui.cursor().min;
    let (rects, th) = widgets::tile_grid(o, w, tiles.len(), 170.0, 64.0, 12.0);
    ui.allocate_exact_size(vec2(w, th), Sense::hover());
    for (tr, (c, v, dcol)) in rects.iter().zip(tiles) {
        widgets::tile(&p, *tr, c, v, *dcol);
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


pub fn sensors(ui: &mut Ui, s: &Snapshot, scroll: f32) -> Option<Page> {
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        // the page reaches the window's edge so the scroll bar sits there; the content keeps its margin
        ui.set_max_width(ui.available_width() - super::PAGE_PAD);
        ui.add_space(super::PAGE_TOP);
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
            let mut tiles: Vec<(String, String, Option<Color32>)> = vec![
                ("Temperature".into(), n.temp.map_or_else(na, |t| format!("{t:.0} °C")), n.temp.map(temp_col)),
                (
                    "Slows down at".into(),
                    n.slowdown_temp.map_or_else(na, |t| format!("{t:.0} °C")),
                    None,
                ),
                (
                    "Power".into(),
                    match (n.power_w, n.power_limit_w) {
                        (Some(p), Some(l)) => format!("{p:.0} / {l:.0} W"),
                        (Some(p), None) => format!("{p:.0} W"),
                        _ => na(),
                    },
                    Some(WARN_COL),
                ),
                (
                    "Fan".into(),
                    n.fan_pct.map_or_else(na, |f| if f == 0 { "Stopped (0 %)".into() } else { format!("{f} %") }),
                    None,
                ),
                ("Core clock".into(), with_max(n.core_mhz, n.core_max_mhz), None),
                ("Memory clock".into(), with_max(n.mem_mhz, n.mem_max_mhz), None),
                ("Video clock".into(), n.video_mhz.map_or_else(na, |c| format!("{c} MHz")), None),
                (
                    "Performance state".into(),
                    n.pstate.map_or_else(na, |ps| match ps {
                        0 => "P0 (full speed)".into(),
                        1..=4 => format!("P{ps} (fast)"),
                        5..=7 => format!("P{ps} (light load)"),
                        _ => format!("P{ps} (idle)"),
                    }),
                    None,
                ),
                (
                    "PCIe link".into(),
                    match (n.pcie, n.pcie_max) {
                        (Some((g, l)), Some((mg, ml))) if (g, l) != (mg, ml) => format!("Gen {g} x{l} (max Gen {mg} x{ml})"),
                        (Some((g, l)), _) => format!("Gen {g} x{l}"),
                        _ => na(),
                    },
                    None,
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
                ));
            }
            tile_row(ui, w, &tiles);
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
        let mut tiles: Vec<(String, String, Option<Color32>)> = vec![
            ("Average clock".into(), ghz(s.clocks.avg_mhz), Some(CPU_COL)),
            ("Fastest thread".into(), if fastest > 0.0 { ghz(fastest) } else { "Unavailable".into() }, None),
            ("Slowest thread".into(), if slowest < f32::MAX { ghz(slowest) } else { "Unavailable".into() }, None),
            ("Base clock".into(), ghz(base), None),
        ];
        if s.zones.is_empty() {
            tiles.push(("CPU temperature".into(), "Needs a driver".into(), None));
        }
        for (name, c) in &s.zones {
            tiles.push((
                format!("Thermal zone {name}"),
                format!("{c:.0} °C"),
                Some(temp_col(*c)),
            ));
        }
        tile_row(ui, w, &tiles);
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
