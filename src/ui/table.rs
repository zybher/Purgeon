//! The process table: sortable columns, search, grouping by name, heat-tinted cells.

use super::widgets::{self, Icon};
use crate::anim::*;
use crate::fmt;
use crate::sys::Snapshot;
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Rect, ScrollArea, Sense, Stroke, Ui};
use std::collections::HashMap;
use std::time::Instant;

const ROW_H: f32 = 28.0;
const HEAD_H: f32 = 34.0;
/// What the Memory column shows
const MEM_MODES: [&str; 3] = ["Private", "Working set", "Commit"];
const MEM_TIPS: [&str; 3] = [
    "Memory = private working set: physical memory only this process uses. Task Manager's \"Memory\" column.",
    "Memory = working set: all physical memory mapped by the process, including pages shared with other processes. Adding these up counts shared pages more than once.",
    "Memory = commit: private memory the process has reserved, in RAM or the page file. Keeps growing when a program leaks memory.",
];
const COL_NAMES: [(Col, &str); 12] = [
    (Col::Name, "name"),
    (Col::Pid, "pid"),
    (Col::Cpu, "cpu"),
    (Col::Private, "memory"),
    (Col::Ws, "workingset"),
    (Col::Commit, "commit"),
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
    Private,
    Ws,
    Commit,
    Io,
    Disk,
    Gpu,
    Vram,
    Threads,
    Handles,
}

/// Columns right of the name: (column, title, width, explanation, priority).
/// Lower priority columns are hidden first when the window is too narrow.
const COLS: [(Col, &str, f32, &str, u8); 11] = [
    (Col::Pid, "PID", 62.0, "Process ID. Grouped rows show how many processes share the name.", 9),
    (Col::Cpu, "CPU", 68.0, "Share of the whole CPU over the last update (100% = every logical processor busy).", 9),
    (Col::Private, "Memory", 88.0, "Private working set: physical memory only this process uses. Task Manager's \"Memory\" column.", 9),
    (Col::Ws, "Working set", 96.0, "All physical memory mapped by the process, including pages shared with others (DLLs, mapped files).", 3),
    (Col::Commit, "Commit", 86.0, "Private bytes the process has committed (RAM or page file).", 2),
    (Col::Io, "I/O", 86.0, "Bytes read + written per second across file, network and device I/O.", 8),
    (Col::Disk, "Disk", 86.0, "Physical disk reads + writes per second: only I/O that reached the disk (no network, no cache hits). Needs administrator.", 8),
    (Col::Gpu, "GPU", 62.0, "Busiest GPU engine this process uses (the way Task Manager counts it).", 8),
    (Col::Vram, "VRAM", 84.0, "Dedicated GPU memory the process holds, summed over all GPUs.", 8),
    (Col::Threads, "Threads", 64.0, "Number of threads.", 1),
    (Col::Handles, "Handles", 70.0, "Number of open kernel object handles.", 4),
];

#[derive(Clone)]
struct Row {
    name: String,
    path: Option<std::sync::Arc<str>>,
    pid: Option<u32>,
    count: u32,
    cpu: f32,
    private_ws: u64,
    ws: u64,
    commit: u64,
    io: f64,
    disk: f64,
    gpu: f32,
    vram: u64,
    threads: u32,
    handles: u32,
}

pub struct TableState {
    search: String,
    sort: Col,
    desc: bool,
    group: bool,
    selected: Option<(String, Option<u32>)>,
    rows: Vec<Row>,
    key: Option<(usize, Col, bool, bool, String, usize)>,
    /// 0 private working set, 1 working set, 2 commit
    mem_mode: usize,
    /// End task waiting for the user's confirmation: (name, pid)
    confirm: Option<(String, u32)>,
    /// Result of the last End task, shown for a few seconds
    note: Option<(String, bool, Instant)>,
    scroll_y: f32,
    view_h: f32,
    scroll_to: Option<f32>,
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
            mem_mode: 0,
            confirm: None,
            note: None,
            scroll_y: 0.0,
            view_h: 400.0,
            scroll_to: None,
        }
    }
}

impl TableState {
    /// Restores `--sort=cpu[:asc]`, `--group=on` and `--mem=1` (used when relaunching elevated)
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
            } else if let Some(v) = a.strip_prefix("--mem=") {
                t.mem_mode = v.parse::<usize>().unwrap_or(0).min(2);
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
            format!("--mem={}", self.mem_mode),
        ]
    }

    fn rebuild(&mut self, s: &Snapshot) {
        let key = (s.seq as usize, self.sort, self.desc, self.group, self.search.clone(), self.mem_mode);
        if self.key.as_ref() == Some(&key) {
            return;
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
                    ws: 0,
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
                r.ws += p.working_set;
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
                    ws: p.working_set,
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
        let mem_mode = self.mem_mode;
        let mem = |r: &Row| match mem_mode {
            1 => r.ws,
            2 => r.commit,
            _ => r.private_ws,
        };
        rows.sort_by(|a, b| {
            use std::cmp::Ordering;
            let o = match sort {
                Col::Name => b.name.to_lowercase().cmp(&a.name.to_lowercase()),
                Col::Pid => a.pid.unwrap_or(a.count).cmp(&b.pid.unwrap_or(b.count)),
                Col::Cpu => a.cpu.total_cmp(&b.cpu),
                Col::Private => mem(a).cmp(&mem(b)),
                Col::Ws => a.ws.cmp(&b.ws),
                Col::Commit => a.commit.cmp(&b.commit),
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
        let sub_r = p.text(pos2(hr.min.x, hr.min.y + 40.0), Align2::LEFT_CENTER, sub, reg(12.5), txt_faint());

        // Which memory figure the Memory column shows
        let mr = Rect::from_min_size(pos2(hr.max.x - 258.0, hr.min.y + 40.0), vec2(258.0, 24.0));
        if mr.min.x > sub_r.max.x + 12.0 {
            if widgets::segmented(ui, mr, &MEM_MODES, &mut self.mem_mode, egui::Id::new("mem_mode")) {
                self.key = None;
            }
            ui.interact(mr, egui::Id::new("mem_mode_tip"), Sense::hover()).on_hover_text(MEM_TIPS[self.mem_mode]);
            if mr.min.x - 110.0 > sub_r.max.x + 12.0 {
                p.text(pos2(mr.min.x - 10.0, mr.center().y), Align2::RIGHT_CENTER, "Memory column", reg(12.0), txt_faint());
            }
        }

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

        // Visible columns: drop the least important ones until the name column has room
        let mm = self.mem_mode;
        let mut cols: Vec<(Col, &str, f32, &str, u8)> = COLS
            .iter()
            .copied()
            .filter(|c| c.0 != Col::Disk || s.disk_trace)
            .filter(|c| !(mm == 1 && c.0 == Col::Ws) && !(mm == 2 && c.0 == Col::Commit))
            .map(|mut c| {
                if c.0 == Col::Private {
                    c.3 = MEM_TIPS[mm];
                }
                c
            })
            .collect();
        while cols.iter().map(|c| c.2).sum::<f32>() + 230.0 > inner.width() {
            match cols.iter().enumerate().filter(|(_, c)| c.4 < 9).min_by_key(|(_, c)| c.4) {
                Some((i, _)) => {
                    cols.remove(i);
                }
                None => break,
            }
        }
        let fixed: f32 = cols.iter().map(|c| c.2).sum();
        let name_w = (inner.width() - fixed - 10.0).max(160.0);
        let mut xs = Vec::with_capacity(cols.len());
        let mut x = inner.min.x + name_w;
        for c in &cols {
            x += c.2;
            xs.push(x);
        }

        // Column headers
        let head = Rect::from_min_size(inner.min, vec2(inner.width(), HEAD_H));
        p.line_segment([pos2(card.min.x, head.max.y), pos2(card.max.x, head.max.y)], Stroke::new(1.0_f32, border()));
        let mut header_cell = |ui: &mut Ui, r: Rect, col: Col, title: &str, tip: &str, right: bool| {
            let resp = ui.interact(r, egui::Id::new(("col", col)), Sense::click());
            let (h, _) = hover_fx(&ctx, &resp);
            let on = self.sort == col;
            p.rect_filled(r.shrink2(vec2(1.0, 5.0)), 6.0, wa((h * 10.0) as u8));
            let colr = if on { hi() } else { mix(txt_faint(), txt(), h) };
            let (pos, al) = if right { (pos2(r.max.x - 10.0, r.center().y), Align2::RIGHT_CENTER) } else { (pos2(r.min.x + 12.0, r.center().y), Align2::LEFT_CENTER) };
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
        header_cell(ui, Rect::from_min_max(head.min, pos2(inner.min.x + name_w, head.max.y)), Col::Name, "Name", "", false);
        for (i, c) in cols.iter().enumerate() {
            let r = Rect::from_min_max(pos2(xs[i] - c.2, head.min.y), pos2(xs[i], head.max.y));
            header_cell(ui, r, c.0, c.1, c.3, true);
        }

        // Rows
        let body = Rect::from_min_max(pos2(card.min.x, head.max.y + 1.0), pos2(card.max.x, card.max.y - 6.0));
        let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("proc_body"));
        bui.set_clip_rect(body.intersect(ui.clip_rect()));
        let total_mem = s.mem.total.max(1) as f32;
        let total_vram = s.gpus.iter().map(|g| g.dedicated_total).max().unwrap_or(0).max(1) as f32;
        let rows = &self.rows;
        let mut clicked: Option<(String, Option<u32>)> = None;
        let mut end_req: Option<(String, Option<u32>)> = None;
        let group = self.group;
        let selected = self.selected.clone();
        let gpu_ok = s.gpu_proc;
        let mut sa = ScrollArea::vertical().auto_shrink(false);
        if let Some(y) = self.scroll_to.take() {
            sa = sa.vertical_scroll_offset(y);
        }
        let out = sa.show_rows(&mut bui, ROW_H, rows.len(), |ui, range| {
            for row in &rows[range] {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let r = Rect::from_min_max(pos2(inner.min.x, r.min.y), pos2(inner.max.x, r.max.y));
                let p = ui.painter();
                let id = egui::Id::new(("row", &row.name, row.pid));
                let h = ease(&ctx, id, resp.hovered());
                let is_sel = selected.as_ref().is_some_and(|(n, pid)| *n == row.name && *pid == row.pid);
                let selk = ease(&ctx, id.with("sel"), is_sel);
                let fill = mix(wa((h * 9.0) as u8), ACCENT.gamma_multiply(0.22), selk);
                p.rect_filled(r.shrink2(vec2(0.0, 1.0)), 7.0, fill);
                let cy = r.center().y;

                // name
                super::icons::paint(&ctx, p, Rect::from_center_size(pos2(r.min.x + 18.0, cy), vec2(18.0, 18.0)), &row.name, row.path.as_ref());
                let maxw = name_w - 50.0;
                let name = clip(p, &row.name, maxw);
                let nr = p.text(pos2(r.min.x + 34.0, cy), Align2::LEFT_CENTER, name, reg(12.5), txt());
                if group && row.count > 1 {
                    p.text(pos2(nr.max.x + 6.0, cy), Align2::LEFT_CENTER, format!("×{}", row.count), reg(11.5), txt_faint());
                }

                for (i, c) in cols.iter().enumerate() {
                    let cell = Rect::from_min_max(pos2(xs[i] - c.2, r.min.y + 3.0), pos2(xs[i], r.max.y - 3.0));
                    let (text, heat, hcol, dim) = match c.0 {
                        Col::Pid => (row.pid.map_or_else(|| row.count.to_string(), |p| p.to_string()), 0.0, CPU_COL, true),
                        Col::Cpu => (fmt::pct(row.cpu), row.cpu / 30.0, CPU_COL, false),
                        Col::Private => {
                            let m = match mm {
                                1 => row.ws,
                                2 => row.commit,
                                _ => row.private_ws,
                            };
                            (fmt::bytes(m), m as f32 / total_mem * 6.0, MEM_COL, false)
                        }
                        Col::Ws => (fmt::bytes(row.ws), 0.0, MEM_COL, false),
                        Col::Commit => (fmt::bytes(row.commit), 0.0, MEM_COL, false),
                        Col::Io => (fmt::rate(row.io), (row.io / (50.0 * 1048576.0)) as f32, IO_COL, false),
                        Col::Disk => (fmt::rate(row.disk), (row.disk / (50.0 * 1048576.0)) as f32, DISK_COL, row.disk == 0.0),
                        // GPU counters unavailable: unknown, not zero
                        Col::Gpu | Col::Vram if !gpu_ok => ("—".into(), 0.0, GPU_COL, true),
                        Col::Gpu => (fmt::pct(row.gpu), row.gpu / 40.0, GPU_COL, false),
                        Col::Vram => (if row.vram == 0 { "0".into() } else { fmt::bytes(row.vram) }, row.vram as f32 / total_vram * 4.0, VRAM_COL, row.vram == 0),
                        Col::Threads => (fmt::thousands(row.threads as u64), 0.0, CPU_COL, true),
                        Col::Handles => (fmt::thousands(row.handles as u64), 0.0, CPU_COL, true),
                        Col::Name => (String::new(), 0.0, CPU_COL, true),
                    };
                    let k = heat.clamp(0.0, 1.0);
                    if k > 0.02 {
                        p.rect_filled(cell.shrink2(vec2(2.0, 0.0)), 5.0, hcol.gamma_multiply(0.08 + 0.32 * k));
                    }
                    p.text(pos2(xs[i] - 10.0, cy), Align2::RIGHT_CENTER, text, reg(12.5), if dim { txt_dim() } else { txt() });
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
