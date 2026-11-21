//! The process table: sortable columns, search, grouping by name, heat-tinted cells.

use super::widgets::{self, Icon};
use crate::anim::*;
use crate::fmt;
use crate::sys::Snapshot;
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Rect, ScrollArea, Sense, Stroke, Ui};
use std::collections::HashMap;
use std::time::Instant;

const ROW_H: f32 = 46.0;
const HEAD_H: f32 = 34.0;
/// How long rows take to slide to their new places after the sort changes
const REORDER_SECS: f32 = 0.38;
const RAM_TIP: &str = "How much of your RAM this app is holding right now, including memory it shares with other apps. This is what lowers your available RAM. Point at a number to see what it is made of.";
const COL_NAMES: [(Col, &str); 10] = [
    (Col::Name, "name"),
    (Col::Pid, "pid"),
    (Col::Cpu, "cpu"),
    (Col::Ram, "memory"),
    (Col::Io, "io"),
    (Col::Disk, "disk"),
    (Col::Gpu, "gpu"),
    (Col::Vram, "vram"),
    (Col::Threads, "threads"),
    (Col::Handles, "handles"),
];

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Col {
    Name,
    Pid,
    Cpu,
    Ram,
    Io,
    Disk,
    Gpu,
    Vram,
    Threads,
    Handles,
}

/// The RAM cell's hover text, in plain words
fn ram_breakdown(r: &Row) -> String {
    format!(
        "In RAM: {}\n   Only this app: {}  (Task Manager's \"Memory\")\n   Shared with other apps: {}\n\nReserved: {}\nMemory the app has asked Windows to set aside. It can be more than what is in RAM, and keeps growing if the app leaks memory.",
        fmt::bytes(r.ram()),
        fmt::bytes(r.private_ws),
        fmt::bytes(r.shared),
        fmt::bytes(r.commit),
    )
}

#[derive(Clone)]
struct Row {
    name: String,
    path: Option<std::sync::Arc<str>>,
    pid: Option<u32>,
    count: u32,
    cpu: f32,
    private_ws: u64,
    /// RAM shared with other apps. For a group, the largest member's, since processes of one
    /// program mostly share the same pages and adding them up would count those pages again.
    shared: u64,
    commit: u64,
    io: f64,
    disk: f64,
    gpu: f32,
    vram: u64,
    threads: u32,
    handles: u32,
}

impl Row {
    /// What the RAM column shows: memory only this app uses plus memory it shares
    fn ram(&self) -> u64 {
        self.private_ws + self.shared
    }
}

pub struct TableState {
    search: String,
    sort: Col,
    desc: bool,
    group: bool,
    selected: Option<(String, Option<u32>)>,
    rows: Vec<Row>,
    key: Option<(usize, Col, bool, bool, String)>,
    /// End task waiting for the user's confirmation: (name, pid)
    confirm: Option<(String, u32)>,
    /// Result of the last End task, shown for a few seconds
    note: Option<(String, bool, Instant)>,
    scroll_y: f32,
    view_h: f32,
    scroll_to: Option<f32>,
    /// Where each row was before the user changed the sort, and when: rows slide from there
    reorder: Option<(HashMap<(String, Option<u32>), usize>, Instant)>,
    /// The row the details panel shows (kept while it slides away)
    inspect: Option<Row>,
}

impl Default for TableState {
    fn default() -> Self {
        TableState {
            search: String::new(),
            sort: Col::Cpu,
            desc: true,
            group: false,
            selected: None,
            rows: Vec::new(),
            key: None,
            confirm: None,
            note: None,
            scroll_y: 0.0,
            view_h: 400.0,
            scroll_to: None,
            reorder: None,
            inspect: None,
        }
    }
}

impl TableState {
    /// Restores `--sort=cpu[:asc]` and `--group=on` (used when relaunching elevated)
    pub fn from_args() -> Self {
        let mut t = TableState::default();
        for a in std::env::args() {
            if let Some(v) = a.strip_prefix("--sort=") {
                let (name, asc) = v.split_once(':').map_or((v, false), |(n, d)| (n, d == "asc"));
                if let Some((c, _)) = COL_NAMES.iter().find(|(_, n)| *n == name) {
                    t.sort = *c;
                    t.desc = !asc;
                }
            } else if let Some(v) = a.strip_prefix("--group=") {
                t.group = v == "on";
            }
        }
        t
    }

    /// The arguments `from_args` reads back
    pub fn state_args(&self) -> Vec<String> {
        let name = COL_NAMES.iter().find(|(c, _)| *c == self.sort).map_or("cpu", |(_, n)| *n);
        vec![
            format!("--sort={name}{}", if self.desc { "" } else { ":asc" }),
            format!("--group={}", if self.group { "on" } else { "off" }),
        ]
    }

    fn rebuild(&mut self, s: &Snapshot) {
        let key = (s.seq as usize, self.sort, self.desc, self.group, self.search.clone());
        if self.key.as_ref() == Some(&key) {
            return;
        }
        // A new sort order (not a data update or a search) animates rows to their new places
        let resorted = self.key.as_ref().is_some_and(|k| (k.1, k.2, k.3) != (key.1, key.2, key.3) && k.4 == key.4);
        if resorted && !reduced() && self.group == self.key.as_ref().map_or(self.group, |k| k.3) {
            let old = self.rows.iter().enumerate().map(|(i, r)| ((r.name.clone(), r.pid), i)).collect();
            self.reorder = Some((old, Instant::now()));
        }
        self.key = Some(key);
        let q = self.search.trim().to_lowercase();
        let hit = |name: &str, pid: u32| q.is_empty() || name.to_lowercase().contains(&q) || pid.to_string() == q;
        let mut rows: Vec<Row> = if self.group {
            let mut m: HashMap<String, Row> = HashMap::new();
            for p in s.procs.iter().filter(|p| hit(&p.name, p.pid)) {
                let r = m.entry(p.name.to_lowercase()).or_insert_with(|| Row {
                    name: p.name.clone(),
                    path: None,
                    pid: None,
                    count: 0,
                    cpu: 0.0,
                    private_ws: 0,
                    shared: 0,
                    commit: 0,
                    io: 0.0,
                    disk: 0.0,
                    gpu: 0.0,
                    vram: 0,
                    threads: 0,
                    handles: 0,
                });
                r.count += 1;
                if r.path.is_none() {
                    r.path = p.path.clone();
                }
                r.cpu += p.cpu;
                r.private_ws += p.private_ws;
                r.shared = r.shared.max(p.shared());
                r.commit += p.commit;
                r.io += p.io_read + p.io_write;
                r.disk += p.disk_read + p.disk_write;
                r.gpu += p.gpu;
                r.vram += p.vram;
                r.threads += p.threads;
                r.handles += p.handles;
            }
            m.into_values().collect()
        } else {
            s.procs
                .iter()
                .filter(|p| hit(&p.name, p.pid))
                .map(|p| Row {
                    name: p.name.clone(),
                    path: p.path.clone(),
                    pid: Some(p.pid),
                    count: 1,
                    cpu: p.cpu,
                    private_ws: p.private_ws,
                    shared: p.shared(),
                    commit: p.commit,
                    io: p.io_read + p.io_write,
                    disk: p.disk_read + p.disk_write,
                    gpu: p.gpu,
                    vram: p.vram,
                    threads: p.threads,
                    handles: p.handles,
                })
                .collect()
        };
        let sort = self.sort;
        rows.sort_by(|a, b| {
            use std::cmp::Ordering;
            let o = match sort {
                Col::Name => b.name.to_lowercase().cmp(&a.name.to_lowercase()),
                Col::Pid => a.pid.unwrap_or(a.count).cmp(&b.pid.unwrap_or(b.count)),
                Col::Cpu => a.cpu.total_cmp(&b.cpu),
                Col::Ram => a.ram().cmp(&b.ram()),
                Col::Io => a.io.total_cmp(&b.io),
                Col::Disk => a.disk.total_cmp(&b.disk),
                Col::Gpu => a.gpu.total_cmp(&b.gpu),
                Col::Vram => a.vram.cmp(&b.vram),
                Col::Threads => a.threads.cmp(&b.threads),
                Col::Handles => a.handles.cmp(&b.handles),
            };
            // stable tie-break by name so equal rows don't shuffle every update
            let o = if o == Ordering::Equal { b.name.cmp(&a.name).then(b.pid.cmp(&a.pid)) } else { o };
            o.reverse()
        });
        if !self.desc {
            rows.reverse();
        }
        self.rows = rows;
    }

    pub fn show(&mut self, ui: &mut Ui, s: &Snapshot) {
        let ctx = ui.ctx().clone();
        let w = ui.available_width();

        // Header row: title, counts, search, grouping
        let (hr, _) = ui.allocate_exact_size(vec2(w, 66.0), Sense::hover());
        let p = ui.painter().clone();
        p.text(pos2(hr.min.x, hr.min.y + 14.0), Align2::LEFT_CENTER, "Processes", sb(24.0), txt());
        let sub = format!(
            "{} processes  ·  {} threads  ·  {} handles",
            fmt::thousands(s.nprocs as u64),
            fmt::thousands(s.threads as u64),
            fmt::thousands(s.handles as u64)
        );
        p.text(pos2(hr.min.x, hr.min.y + 40.0), Align2::LEFT_CENTER, sub, reg(12.5), txt_faint());

        let sw = Rect::from_min_size(pos2(hr.max.x - 36.0, hr.min.y + 9.0), vec2(36.0, 21.0));
        widgets::switch(ui, sw, &mut self.group).on_hover_text("Sum processes that share a name (all chrome.exe as one row)");
        p.text(pos2(sw.min.x - 10.0, sw.center().y), Align2::RIGHT_CENTER, "Group by name", reg(12.5), txt_dim());

        let sr = Rect::from_min_size(pos2(sw.min.x - 128.0 - 260.0, hr.min.y + 4.0), vec2(260.0, 31.0));
        let focus_id = egui::Id::new("proc_search");
        let focused = ctx.memory(|m| m.has_focus(focus_id));
        let fk = ease(&ctx, focus_id.with("fk"), focused);
        p.rect_filled(sr, 9.0, well());
        p.rect_stroke(sr, 9.0, Stroke::new(1.0_f32, mix(border(), ACCENT, fk)));
        if fk > 0.01 {
            p.rect_stroke(sr.expand(2.0), 11.0, Stroke::new(2.0_f32, ACCENT.gamma_multiply(0.25 * fk)));
        }
        widgets::icon(&p, Icon::Search, pos2(sr.min.x + 16.0, sr.center().y), mix(txt_faint(), ACCENT, fk));
        let te_r = Rect::from_min_max(pos2(sr.min.x + 30.0, sr.min.y + 7.0), pos2(sr.max.x - 26.0, sr.max.y - 6.0));
        ui.put(
            te_r,
            egui::TextEdit::singleline(&mut self.search).id(focus_id).frame(false).hint_text("Search name or PID").font(reg(13.0)),
        );
        if !self.search.is_empty() {
            let cr = Rect::from_center_size(pos2(sr.max.x - 14.0, sr.center().y), vec2(18.0, 18.0));
            let resp = ui.interact(cr, focus_id.with("clear"), Sense::click());
            let (h, _) = hover_fx(&ctx, &resp);
            p.circle_filled(cr.center(), 8.0, wa((14.0 + h * 20.0) as u8));
            let c = cr.center();
            let st = Stroke::new(1.2_f32, mix(txt_dim(), hi(), h));
            p.line_segment([c + vec2(-3.0, -3.0), c + vec2(3.0, 3.0)], st);
            p.line_segment([c + vec2(3.0, -3.0), c + vec2(-3.0, 3.0)], st);
            if resp.clicked() {
                self.search.clear();
            }
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::F)) {
            ctx.memory_mut(|m| m.request_focus(focus_id));
        }

        self.rebuild(s);
        // dev captures can start with the top row open in the details panel: PURGEON_PROC_SELECT=1
        if self.selected.is_none() && !self.rows.is_empty() && std::env::var_os("PURGEON_PROC_SELECT").is_some() && !ctx.data(|d| d.get_temp::<bool>(egui::Id::new("proc_test_sel")).unwrap_or(false)) {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("proc_test_sel"), true));
            self.selected = Some((self.rows[0].name.clone(), self.rows[0].pid));
        }
        let pitch = ROW_H + ui.spacing().item_spacing.y;
        if self.confirm.is_none() {
            self.keyboard(&ctx, focus_id, pitch);
        }

        // The table card fills the rest
        // (ui.put above moved the layout cursor back up to the search box, so anchor on the header)
        let avail = ui.available_rect_before_wrap();
        let card = Rect::from_min_max(pos2(avail.min.x, hr.max.y + 6.0), pos2(avail.max.x, avail.max.y - 18.0));
        widgets::card(&p, card, crate::theme::card());
        let inner = card.shrink2(vec2(8.0, 0.0));

        // The list on the left, and, while a row is selected, its details on the right
        let sel_row = self.selected.as_ref().and_then(|(n, pid)| self.rows.iter().find(|r| r.name == *n && r.pid == *pid)).cloned();
        if sel_row.is_some() {
            self.inspect = sel_row.clone();
        }
        let ik = ease(&ctx, egui::Id::new("proc_inspect"), sel_row.is_some());
        if ik < 0.01 {
            self.inspect = None;
        }
        let insp_w = (inner.width() * 0.34).clamp(250.0, 320.0) * ik;
        let list = Rect::from_min_max(inner.min, pos2(inner.max.x - insp_w, inner.max.y));

        // Four measures, the ones people sort by; the rest lives in the details
        let disk_col = if s.disk_trace {
            (Col::Disk, "Disk", "Reads + writes per second that reached the disk (no network, no cache hits).")
        } else {
            (Col::Io, "I/O", "Bytes read + written per second across file, network and device I/O.")
        };
        let mut cols: Vec<(Col, &str, &str)> = vec![
            (Col::Cpu, "CPU", "Share of the whole CPU over the last update (100% = every logical processor busy)."),
            (Col::Ram, "Memory", RAM_TIP),
            disk_col,
            (Col::Gpu, "GPU", "Busiest GPU engine this process uses (the way Task Manager counts it)."),
        ];
        let mut col_w = ((list.width() - 230.0) / cols.len() as f32).clamp(78.0, 112.0);
        while cols.len() > 2 && 230.0 + col_w * cols.len() as f32 > list.width() {
            cols.remove(if cols.len() == 4 { 2 } else { cols.len() - 1 });
            col_w = ((list.width() - 230.0) / cols.len() as f32).clamp(78.0, 112.0);
        }
        let name_w = list.width() - col_w * cols.len() as f32;
        let xs: Vec<f32> = (0..cols.len()).map(|i| list.min.x + name_w + col_w * (i + 1) as f32).collect();

        // Column headers
        let head = Rect::from_min_size(list.min, vec2(list.width(), HEAD_H));
        p.line_segment([pos2(card.min.x + 14.0, head.max.y), pos2(list.max.x - 6.0, head.max.y)], Stroke::new(1.0_f32, border()));
        let mut header_cell = |ui: &mut Ui, r: Rect, col: Col, title: &str, tip: &str, right: bool| {
            let resp = ui.interact(r, egui::Id::new(("col", col)), Sense::click());
            let (h, _) = hover_fx(&ctx, &resp);
            let on = self.sort == col;
            let colr = if on { hi() } else { mix(txt_faint(), txt(), h) };
            let (pos, al) = if right { (pos2(r.max.x - 14.0, r.center().y), Align2::RIGHT_CENTER) } else { (pos2(r.min.x + 14.0, r.center().y), Align2::LEFT_CENTER) };
            let tr = p.text(pos, al, title, sb(11.5), colr);
            if on {
                let ax = if right { tr.min.x - 9.0 } else { tr.max.x + 9.0 };
                let c = pos2(ax, r.center().y);
                let d = if self.desc { 1.0 } else { -1.0 };
                p.add(egui::Shape::convex_polygon(
                    vec![c + vec2(-3.5, -2.0 * d), c + vec2(3.5, -2.0 * d), c + vec2(0.0, 2.5 * d)],
                    ACCENT,
                    Stroke::NONE,
                ));
            }
            if resp.clicked() {
                if self.sort == col {
                    self.desc = !self.desc;
                } else {
                    self.sort = col;
                    self.desc = col != Col::Name;
                }
            }
            if !tip.is_empty() {
                resp.on_hover_text(tip);
            }
        };
        header_cell(ui, Rect::from_min_max(head.min, pos2(list.min.x + name_w, head.max.y)), Col::Name, "Name", "", false);
        for (i, c) in cols.iter().enumerate() {
            let r = Rect::from_min_max(pos2(xs[i] - col_w, head.min.y), pos2(xs[i], head.max.y));
            header_cell(ui, r, c.0, c.1, c.2, true);
        }

        // Rows
        let body = Rect::from_min_max(pos2(card.min.x, head.max.y + 1.0), pos2(list.max.x, card.max.y - 6.0));
        let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("proc_body"));
        bui.set_clip_rect(body.intersect(ui.clip_rect()));
        let rows = &self.rows;
        // The bars under each number compare a row with the busiest one, but never make a
        // near-idle list look busy: they fill against at least these amounts
        let top = |f: &dyn Fn(&Row) -> f32, floor: f32| rows.iter().map(f).fold(floor, f32::max);
        let scale_cpu = top(&|r| r.cpu, 10.0);
        let scale_ram = top(&|r| r.ram() as f32, 1024.0 * 1048576.0);
        let disk_trace = s.disk_trace;
        let scale_disk = top(&|r| (if disk_trace { r.disk } else { r.io }) as f32, 5.0 * 1048576.0);
        let scale_gpu = top(&|r| r.gpu, 10.0);
        // 0..1 progress of the sort animation, eased; None when settled
        let reorder = match &self.reorder {
            Some((old, at)) if at.elapsed().as_secs_f32() < REORDER_SECS => {
                ctx.request_repaint();
                let t = at.elapsed().as_secs_f32() / REORDER_SECS;
                Some((old, egui::emath::easing::cubic_in_out(t)))
            }
            _ => None,
        };
        let view_h = self.view_h;
        let mut clicked: Option<(String, Option<u32>)> = None;
        let mut end_req: Option<(String, Option<u32>)> = None;
        let group = self.group;
        let selected = self.selected.clone();
        let gpu_ok = s.gpu_proc;
        let sort = self.sort;
        let mut sa = ScrollArea::vertical().auto_shrink(false);
        if let Some(y) = self.scroll_to.take() {
            sa = sa.vertical_scroll_offset(y);
        }
        let out = sa.show_rows(&mut bui, ROW_H, rows.len(), |ui, range| {
            let first = range.start;
            for (j, row) in rows[range].iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let mut r = Rect::from_min_max(pos2(list.min.x, r.min.y), pos2(list.max.x - 4.0, r.max.y));
                // slide from where the row was before the sort; rows from far away start just off screen
                if let Some((old, k)) = &reorder {
                    let new_i = (first + j) as f32;
                    let from = old.get(&(row.name.clone(), row.pid)).map_or(new_i + 3.0, |&i| i as f32);
                    let off = ((from - new_i) * pitch).clamp(-view_h - ROW_H, view_h + ROW_H) * (1.0 - k);
                    r = r.translate(vec2(0.0, off));
                }
                let p = ui.painter();
                let id = egui::Id::new(("row", &row.name, row.pid));
                let h = ease(&ctx, id, resp.hovered());
                let is_sel = selected.as_ref().is_some_and(|(n, pid)| *n == row.name && *pid == row.pid);
                let selk = ease(&ctx, id.with("sel"), is_sel);
                let fill = mix(wa((h * 8.0) as u8), ACCENT.gamma_multiply(0.16), selk);
                p.rect_filled(r.shrink2(vec2(0.0, 2.0)), 9.0, fill);
                let cy = r.center().y;

                // name, and a quiet second line
                super::icons::paint(&ctx, p, Rect::from_center_size(pos2(r.min.x + 22.0, cy), vec2(22.0, 22.0)), &row.name, row.path.as_ref());
                let maxw = name_w - 64.0;
                let name = clip(p, &row.name, maxw);
                p.text(pos2(r.min.x + 44.0, cy - 7.5), Align2::LEFT_CENTER, name, reg(13.0), mix(txt(), hi(), selk));
                let sub = if group && row.count > 1 {
                    format!("{} processes", row.count)
                } else {
                    match row.pid {
                        Some(pid) => format!("PID {pid}"),
                        None => "1 process".into(),
                    }
                };
                p.text(pos2(r.min.x + 44.0, cy + 8.5), Align2::LEFT_CENTER, sub, reg(11.0), txt_faint());

                for (i, c) in cols.iter().enumerate() {
                    let cell = Rect::from_min_max(pos2(xs[i] - col_w, r.min.y), pos2(xs[i], r.max.y));
                    let (text, frac, col, idle) = match c.0 {
                        Col::Cpu => (fmt::pct(row.cpu), row.cpu / scale_cpu, CPU_COL, row.cpu < 0.05),
                        Col::Ram => {
                            if resp.hovered() && ctx.pointer_hover_pos().is_some_and(|q| cell.contains(q)) {
                                resp.clone().on_hover_text(ram_breakdown(row));
                            }
                            (fmt::bytes(row.ram()), row.ram() as f32 / scale_ram, MEM_COL, false)
                        }
                        Col::Io => (fmt::rate(row.io), row.io as f32 / scale_disk, IO_COL, row.io < 1.0),
                        Col::Disk => (fmt::rate(row.disk), row.disk as f32 / scale_disk, DISK_COL, row.disk < 1.0),
                        // GPU counters unavailable: unknown, not zero
                        Col::Gpu if !gpu_ok => ("—".into(), 0.0, GPU_COL, true),
                        Col::Gpu => (fmt::pct(row.gpu), row.gpu / scale_gpu, GPU_COL, row.gpu < 0.05),
                        _ => (String::new(), 0.0, CPU_COL, true),
                    };
                    metric(p, xs[i] - 14.0, cy, &text, frac, col, sort == c.0, idle);
                }

                if resp.clicked() {
                    clicked = Some((row.name.clone(), row.pid));
                }
                let (nm, pid) = (row.name.clone(), row.pid);
                resp.context_menu(|ui| {
                    if ui.button("Copy name").clicked() {
                        ui.output_mut(|o| o.copied_text = nm.clone());
                        ui.close_menu();
                    }
                    if let Some(pid) = pid {
                        if ui.button("Copy PID").clicked() {
                            ui.output_mut(|o| o.copied_text = pid.to_string());
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button("End task…").clicked() {
                        end_req = Some((nm.clone(), pid));
                        ui.close_menu();
                    }
                });
            }
        });
        if let Some(row) = self.inspect.clone() {
            let ir = Rect::from_min_max(pos2(list.max.x, card.min.y), card.max);
            if let Some(end) = self.inspector(ui, &p, ir, ik, &row, s) {
                end_req = Some(end);
            }
        }
        self.scroll_y = out.state.offset.y;
        self.view_h = out.inner_rect.height();
        if let Some(c) = clicked {
            self.selected = if self.selected.as_ref() == Some(&c) { None } else { Some(c) };
        }
        if let Some((name, pid)) = end_req {
            self.selected = Some((name.clone(), pid));
            self.ask_end(name, pid);
        }

        // Empty states
        if self.rows.is_empty() {
            let (t, sub) = if s.procs.is_empty() {
                ("Couldn't read the process list", "Windows refused the query. Try Elevate (bottom left).")
            } else {
                ("No processes match", "Search looks at names and PIDs. Press Esc to clear it.")
            };
            p.text(body.center() - vec2(0.0, 10.0), Align2::CENTER_CENTER, t, sb(13.5), txt_dim());
            p.text(body.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, sub, reg(12.0), txt_faint());
        }

        // Result of the last End task
        if let Some((msg, ok, at)) = &self.note {
            let age = at.elapsed().as_secs_f32();
            if age < 5.0 {
                let r = Rect::from_center_size(pos2(card.center().x, card.max.y - 26.0), vec2((card.width() - 40.0).min(520.0), 34.0));
                p.rect_filled(r, 10.0, Color32::from_gray(0x2a));
                p.rect_stroke(r, 10.0, Stroke::new(1.0_f32, border()));
                p.circle_filled(pos2(r.min.x + 16.0, r.center().y), 3.5, if *ok { GPU_COL } else { DANGER });
                widgets::fit(&p, pos2(r.min.x + 28.0, r.center().y), Align2::LEFT_CENTER, msg, reg(12.5), r.width() - 40.0, txt());
                ctx.request_repaint_after(std::time::Duration::from_secs_f32(5.0 - age));
            } else {
                self.note = None;
            }
        }

        self.confirm_dialog(&ctx);
    }

    /// Arrow keys move the selection, Delete ends it (after a confirm), typing filters,
    /// Esc clears the selection or the search
    fn keyboard(&mut self, ctx: &egui::Context, focus_id: egui::Id, pitch: f32) {
        use egui::{Event, Key};
        let search_focused = ctx.memory(|m| m.has_focus(focus_id));
        let other_focused = ctx.memory(|m| m.focused().is_some_and(|f| f != focus_id));
        if other_focused {
            return;
        }
        // Type to filter: the first key typed anywhere on the page goes into the search box
        if !search_focused {
            let typed: String = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        Event::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect()
            });
            if !typed.trim().is_empty() {
                self.search.push_str(&typed);
                ctx.memory_mut(|m| m.request_focus(focus_id));
                let mut st = egui::text_edit::TextEditState::load(ctx, focus_id).unwrap_or_default();
                let n = self.search.chars().count();
                st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(n))));
                st.store(ctx, focus_id);
                return;
            }
        }
        let (up, down, home, end, del, esc) = ctx.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::Home) && !search_focused,
                i.key_pressed(Key::End) && !search_focused,
                i.key_pressed(Key::Delete) && !search_focused,
                i.key_pressed(Key::Escape),
            )
        });
        if esc {
            if !self.search.is_empty() {
                self.search.clear();
            } else {
                self.selected = None;
            }
            ctx.memory_mut(|m| m.surrender_focus(focus_id));
        }
        let n = self.rows.len();
        if n > 0 && (up || down || home || end) {
            let cur = self.selected.as_ref().and_then(|(nm, pid)| self.rows.iter().position(|r| r.name == *nm && r.pid == *pid));
            let i = match (cur, up, down, home) {
                (_, _, _, true) => 0,
                _ if end => n - 1,
                (None, _, _, _) => 0,
                (Some(c), true, _, _) => c.saturating_sub(1),
                (Some(c), _, true, _) => (c + 1).min(n - 1),
                (Some(c), _, _, _) => c,
            };
            let r = &self.rows[i];
            self.selected = Some((r.name.clone(), r.pid));
            // keep the selected row in view
            let top = i as f32 * pitch;
            if top < self.scroll_y {
                self.scroll_to = Some(top);
            } else if top + pitch > self.scroll_y + self.view_h {
                self.scroll_to = Some(top + pitch - self.view_h);
            }
        }
        if del {
            if let Some((name, pid)) = self.selected.clone() {
                self.ask_end(name, pid);
            }
        }
    }

    fn ask_end(&mut self, name: String, pid: Option<u32>) {
        match pid {
            Some(pid) => self.confirm = Some((name, pid)),
            None => self.note = Some(("Turn off Group by name to end one process at a time.".into(), false, Instant::now())),
        }
    }

    /// "End chrome.exe?" with Cancel and End task. Enter confirms, Esc cancels.
    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some((name, pid)) = self.confirm.clone() else { return };
        let blocked = crate::platform::end_blocked(pid, &name);
        let (mut go, mut cancel) = ctx.input(|i| (i.key_pressed(egui::Key::Enter) && blocked.is_none(), i.key_pressed(egui::Key::Escape)));
        // dim everything behind the dialog
        let screen = ctx.screen_rect();
        ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("end_dim"))).rect_filled(screen, 0.0, Color32::from_black_alpha(110));
        egui::Window::new("end_task")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([380.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("End {name}?")).font(sb(16.0)).color(hi()));
                ui.add_space(2.0);
                ui.label(egui::RichText::new(format!("PID {pid}")).font(reg(12.0)).color(txt_faint()));
                ui.add_space(8.0);
                let body = blocked.unwrap_or("The program closes immediately. Anything unsaved in it is lost.");
                ui.label(egui::RichText::new(body).font(reg(12.5)).color(txt_dim()));
                ui.add_space(14.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if blocked.is_none() {
                        let b = egui::Button::new(egui::RichText::new("End task").font(sb(12.5)).color(Color32::WHITE)).fill(DANGER).min_size(vec2(92.0, 30.0));
                        if ui.add(b).clicked() {
                            go = true;
                        }
                    }
                    let label = if blocked.is_some() { "OK" } else { "Cancel" };
                    if ui.add(egui::Button::new(egui::RichText::new(label).font(sb(12.5))).min_size(vec2(80.0, 30.0))).clicked() {
                        cancel = true;
                    }
                });
            });
        if go {
            self.confirm = None;
            self.note = Some(match crate::platform::end_process(pid, &name) {
                Ok(()) => (format!("Ended {name} (PID {pid})."), true, Instant::now()),
                Err(e) => (format!("Couldn't end {name}: {e}"), false, Instant::now()),
            });
            self.key = None;
        } else if cancel {
            self.confirm = None;
        }
    }

    /// The details of the selected row, sliding in from the card's right side
    fn inspector(&mut self, ui: &mut Ui, p: &egui::Painter, r: Rect, k: f32, row: &Row, s: &Snapshot) -> Option<(String, Option<u32>)> {
        let ctx = ui.ctx().clone();
        let p = p.with_clip_rect(r.intersect(ui.clip_rect()));
        let a = k * k;
        let col = |c: Color32| c.gamma_multiply(a);
        p.line_segment([pos2(r.min.x, r.min.y + 14.0), pos2(r.min.x, r.max.y - 14.0)], Stroke::new(1.0_f32, col(border())));
        let x0 = r.min.x + 22.0 + (1.0 - k) * 30.0;
        let x1 = x0 + r.width().max(250.0) - 44.0;
        let w = x1 - x0;
        let mut y = r.min.y + 22.0;

        super::icons::paint(&ctx, &p, Rect::from_min_size(pos2(x0, y), vec2(32.0, 32.0)), &row.name, row.path.as_ref());
        widgets::fit(&p, pos2(x0 + 44.0, y + 8.0), Align2::LEFT_CENTER, &row.name, sb(15.0), w - 76.0, col(hi()));
        let who = if self.group && row.count > 1 { format!("{} processes", row.count) } else { row.pid.map_or("1 process".into(), |p| format!("PID {p}")) };
        p.text(pos2(x0 + 44.0, y + 26.0), Align2::LEFT_CENTER, who, reg(11.5), col(txt_faint()));
        // close
        let cr = Rect::from_center_size(pos2(x1 - 8.0, y + 10.0), vec2(22.0, 22.0));
        let resp = ui.interact(cr, egui::Id::new("inspect_close"), Sense::click());
        let (h, _) = hover_fx(&ctx, &resp);
        p.circle_filled(cr.center(), 11.0, col(wa((h * 22.0) as u8)));
        let c = cr.center();
        let st = Stroke::new(1.3_f32, col(mix(txt_dim(), hi(), h)));
        p.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], st);
        p.line_segment([c + vec2(3.5, -3.5), c + vec2(-3.5, 3.5)], st);
        if resp.clicked() {
            self.selected = None;
        }
        y += 50.0;

        let path = row.path.as_deref().unwrap_or("Windows keeps this program's location private");
        let lines = p.layout(path.to_string(), reg(11.0), col(txt_dim()), w);
        let ph = lines.size().y.min(44.0);
        p.with_clip_rect(Rect::from_min_size(pos2(x0, y), vec2(w, ph)).intersect(p.clip_rect())).galley(pos2(x0, y), lines, col(txt_dim()));
        y += ph + 18.0;

        let gpu_ok = s.gpu_proc;
        let stat = |label: &str, value: String, y: &mut f32, strong: bool| {
            p.text(pos2(x0, *y), Align2::LEFT_CENTER, label, reg(12.0), col(txt_faint()));
            p.text(pos2(x1, *y), Align2::RIGHT_CENTER, value, reg(12.5), col(if strong { hi() } else { txt() }));
            *y += 25.0;
        };
        let section = |t: &str, y: &mut f32| {
            p.text(pos2(x0, *y), Align2::LEFT_CENTER, t, sb(10.5), col(txt_faint()));
            *y += 22.0;
        };
        section("ACTIVITY", &mut y);
        stat("CPU", fmt::pct(row.cpu), &mut y, true);
        stat("GPU", if gpu_ok { fmt::pct(row.gpu) } else { "—".into() }, &mut y, false);
        if s.disk_trace {
            stat("Disk", fmt::rate(row.disk), &mut y, false);
        }
        stat("All I/O", fmt::rate(row.io), &mut y, false);
        y += 8.0;
        section("MEMORY", &mut y);
        stat("In RAM", fmt::bytes(row.ram()), &mut y, true);
        stat("Only this app", fmt::bytes(row.private_ws), &mut y, false);
        stat("Shared", fmt::bytes(row.shared), &mut y, false);
        stat("Reserved", fmt::bytes(row.commit), &mut y, false);
        stat("Video memory", if gpu_ok { fmt::bytes(row.vram) } else { "—".into() }, &mut y, false);
        y += 8.0;
        section("INSIDE", &mut y);
        stat("Threads", fmt::thousands(row.threads as u64), &mut y, false);
        stat("Handles", fmt::thousands(row.handles as u64), &mut y, false);

        // End task, at the foot of the panel
        let top = (r.max.y - 56.0).max(y + 6.0);
        let br = Rect::from_min_max(pos2(x0, top), pos2(x1, top + 34.0));
        let resp = ui.interact(br, egui::Id::new("inspect_end"), Sense::click());
        let (h, _) = hover_fx(&ctx, &resp);
        p.rect_filled(br, 9.0, col(DANGER.gamma_multiply(0.10 + 0.16 * h)));
        p.rect_stroke(br, 9.0, Stroke::new(1.0_f32, col(DANGER.gamma_multiply(0.45 + 0.3 * h))));
        p.text(br.center(), Align2::CENTER_CENTER, "End task…", sb(12.5), col(mix(txt(), hi(), h)));
        (resp.clicked() && k > 0.9).then(|| (row.name.clone(), row.pid))
    }
}

/// A number with a thin bar under it: how this row compares with the busiest one. The column
/// being sorted by stands out; idle values step back.
#[allow(clippy::too_many_arguments)]
fn metric(p: &egui::Painter, right: f32, cy: f32, text: &str, frac: f32, col: Color32, active: bool, idle: bool) {
    let tc = if idle { txt_faint() } else if active { hi() } else { txt() };
    p.text(pos2(right, cy - 5.5), Align2::RIGHT_CENTER, text, reg(12.5), tc);
    // an idle value gets no bar at all, so the eye only lands on rows doing something
    if idle {
        return;
    }
    let bw = 54.0;
    let track = Rect::from_min_max(pos2(right - bw, cy + 8.0), pos2(right, cy + 11.0));
    p.rect_filled(track, 1.5, wa(10));
    let f = frac.clamp(0.0, 1.0);
    if f > 0.004 {
        let fill = Rect::from_min_max(track.min, pos2(track.min.x + (bw * f).max(3.0), track.max.y));
        p.rect_filled(fill, 1.5, col.gamma_multiply(if active { 0.95 } else { 0.5 }));
    }
}

/// Shortens text with an ellipsis to fit `max` points
pub fn clip(p: &egui::Painter, s: &str, max: f32) -> String {
    let w = |t: &str| p.layout_no_wrap(t.to_string(), reg(12.5), Color32::WHITE).size().x;
    if w(s) <= max {
        return s.to_string();
    }
    let mut out: String = s.to_string();
    while !out.is_empty() && w(&format!("{out}…")) > max {
        out.pop();
    }
    format!("{out}…")
}
