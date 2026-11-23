//! The process table: one row per program (its processes added together), sortable columns,
//! search, heat-tinted cells. Clicking a row opens the details panel.

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

/// The RAM cell's hover readout: the values only
fn ram_breakdown(r: &Row) -> String {
    format!(
        "In RAM  {}\nOnly this app  {}\nShared  {}\nReserved  {}",
        fmt::bytes(r.ram()),
        fmt::bytes(r.private_ws),
        fmt::bytes(r.shared),
        fmt::bytes(r.commit),
    )
}

#[derive(Clone)]
struct Row {
    name: String,
    /// Lower-case name: what the row and the details panel are keyed by
    key: String,
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
    rows: Vec<Row>,
    key: Option<(usize, Col, bool, String)>,
    scroll_y: f32,
    view_h: f32,
    scroll_to: Option<f32>,
    /// Where each row was before the user changed the sort, and when: rows slide from there
    reorder: Option<(HashMap<String, usize>, Instant)>,
    /// Delete was pressed on this row
    end_key: Option<String>,
}

impl Default for TableState {
    fn default() -> Self {
        TableState {
            search: String::new(),
            sort: Col::Cpu,
            desc: true,
            rows: Vec::new(),
            key: None,
            scroll_y: 0.0,
            view_h: 400.0,
            scroll_to: None,
            reorder: None,
            end_key: None,
        }
    }
}

impl TableState {
    /// Restores `--sort=cpu[:asc]` (used when relaunching elevated)
    pub fn from_args() -> Self {
        let mut t = TableState::default();
        for a in std::env::args() {
            if let Some(v) = a.strip_prefix("--sort=") {
                let (name, asc) = v.split_once(':').map_or((v, false), |(n, d)| (n, d == "asc"));
                if let Some((c, _)) = COL_NAMES.iter().find(|(_, n)| *n == name) {
                    t.sort = *c;
                    t.desc = !asc;
                }
            }
        }
        t
    }

    /// The arguments `from_args` reads back
    pub fn state_args(&self) -> Vec<String> {
        let name = COL_NAMES.iter().find(|(c, _)| *c == self.sort).map_or("cpu", |(_, n)| *n);
        vec![format!("--sort={name}{}", if self.desc { "" } else { ":asc" })]
    }

    fn rebuild(&mut self, s: &Snapshot) {
        let key = (s.seq as usize, self.sort, self.desc, self.search.clone());
        if self.key.as_ref() == Some(&key) {
            return;
        }
        // A new sort order (not a data update or a search) animates rows to their new places
        let resorted = self.key.as_ref().is_some_and(|k| (k.1, k.2) != (key.1, key.2) && k.3 == key.3);
        if resorted && !reduced() {
            let old = self.rows.iter().enumerate().map(|(i, r)| (r.key.clone(), i)).collect();
            self.reorder = Some((old, Instant::now()));
        }
        self.key = Some(key);
        let q = self.search.trim().to_lowercase();
        let hit = |name: &str, pid: u32| q.is_empty() || name.to_lowercase().contains(&q) || pid.to_string() == q;
        // a program's processes always form one row; searching a PID finds its program
        let mut m: HashMap<String, Row> = HashMap::new();
        for p in s.procs.iter().filter(|p| hit(&p.name, p.pid)) {
            let r = m.entry(p.name.to_lowercase()).or_insert_with(|| Row {
                name: p.name.clone(),
                key: p.name.to_lowercase(),
                path: None,
                pid: Some(p.pid),
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
            if r.count > 1 {
                r.pid = None;
            }
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
        let mut rows: Vec<Row> = m.into_values().collect();
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
            let o = if o == Ordering::Equal { b.key.cmp(&a.key) } else { o };
            o.reverse()
        });
        if !self.desc {
            rows.reverse();
        }
        self.rows = rows;
    }

    /// `room`: width the details panel takes on the right while it is open
    pub fn show(&mut self, ui: &mut Ui, s: &Snapshot, room: f32) {
        let ctx = ui.ctx().clone();
        let w = ui.available_width();

        // Header row: title, counts, search
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

        let sr = Rect::from_min_size(pos2(hr.max.x - 260.0, hr.min.y + 4.0), vec2(260.0, 31.0));
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
        if !self.rows.is_empty() && std::env::var_os("PURGEON_PROC_SELECT").is_some() && !ctx.data(|d| d.get_temp::<bool>(egui::Id::new("proc_test_sel")).unwrap_or(false)) {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("proc_test_sel"), true));
            super::detail::open_group(&ctx, &self.rows[0].key);
        }
        let selected = super::detail::shown_group(&ctx);
        let pitch = ROW_H + ui.spacing().item_spacing.y;
        self.keyboard(&ctx, focus_id, pitch, selected.as_deref());
        if let Some(k) = self.end_key.take() {
            super::detail::ask_end(&ctx, s, &k);
        }

        // The table card fills the rest
        // (ui.put above moved the layout cursor back up to the search box, so anchor on the header)
        let avail = ui.available_rect_before_wrap();
        let card = Rect::from_min_max(pos2(avail.min.x, hr.max.y + 6.0), pos2(avail.max.x, avail.max.y - 18.0));
        widgets::card(&p, card, crate::theme::card());
        let inner = card.shrink2(vec2(8.0, 0.0));

        // The list, leaving room on the right while the details panel is open
        let list = Rect::from_min_max(inner.min, pos2((inner.max.x - room).max(inner.min.x + 240.0), inner.max.y));

        // Four measures, the ones people sort by; the rest lives in the details
        let disk_col = if s.disk_trace {
            (Col::Disk, "Disk")
        } else {
            (Col::Io, "I/O")
        };
        let mut cols: Vec<(Col, &str)> = vec![(Col::Cpu, "CPU"), (Col::Ram, "Memory"), disk_col, (Col::Gpu, "GPU")];
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
        let mut header_cell = |ui: &mut Ui, r: Rect, col: Col, title: &str, right: bool| {
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
        };
        header_cell(ui, Rect::from_min_max(head.min, pos2(list.min.x + name_w, head.max.y)), Col::Name, "Name", false);
        for (i, c) in cols.iter().enumerate() {
            let r = Rect::from_min_max(pos2(xs[i] - col_w, head.min.y), pos2(xs[i], head.max.y));
            header_cell(ui, r, c.0, c.1, true);
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
        let mut clicked: Option<String> = None;
        let mut end_req: Option<String> = None;
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
                    let from = old.get(&row.key).map_or(new_i + 3.0, |&i| i as f32);
                    let off = ((from - new_i) * pitch).clamp(-view_h - ROW_H, view_h + ROW_H) * (1.0 - k);
                    r = r.translate(vec2(0.0, off));
                }
                let p = ui.painter();
                let id = egui::Id::new(("row", &row.key));
                let h = ease(&ctx, id, resp.hovered());
                if resp.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let is_sel = selected.as_deref() == Some(row.key.as_str());
                let selk = ease(&ctx, id.with("sel"), is_sel);
                let fill = mix(wa((h * 8.0) as u8), ACCENT.gamma_multiply(0.16), selk);
                p.rect_filled(r.shrink2(vec2(0.0, 2.0)), 9.0, fill);
                let cy = r.center().y;

                // name, and a quiet second line
                super::icons::paint(&ctx, p, Rect::from_center_size(pos2(r.min.x + 22.0, cy), vec2(22.0, 22.0)), &row.name, row.path.as_ref());
                let maxw = name_w - 64.0;
                let name = clip(p, &row.name, maxw);
                p.text(pos2(r.min.x + 44.0, cy - 7.5), Align2::LEFT_CENTER, name, reg(13.0), mix(txt(), hi(), selk));
                // what it is, in a few words, next to how many processes it has
                let what = crate::explain::about(&row.name).map(|a| a.0.to_string()).or_else(|| row.path.as_deref().and_then(|p| crate::sys::procinfo::file_info(p).description));
                let count = match row.pid {
                    Some(pid) => format!("PID {pid}"),
                    None => format!("{} processes", row.count),
                };
                let sub = match what {
                    Some(w) if !w.eq_ignore_ascii_case(row.name.trim_end_matches(".exe")) => format!("{count}  ·  {w}"),
                    _ => count,
                };
                widgets::fit(p, pos2(r.min.x + 44.0, cy + 8.5), Align2::LEFT_CENTER, &sub, reg(11.0), maxw, txt_faint());

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
                    clicked = Some(row.key.clone());
                }
                let (nm, key, pid, n) = (row.name.clone(), row.key.clone(), row.pid, row.count);
                resp.context_menu(|ui| {
                    if ui.button("Details").clicked() {
                        clicked = Some(key.clone());
                        ui.close_menu();
                    }
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
                    if ui.button(if n > 1 { "End all…" } else { "End task…" }).clicked() {
                        end_req = Some(key.clone());
                        ui.close_menu();
                    }
                });
            }
        });
        self.scroll_y = out.state.offset.y;
        self.view_h = out.inner_rect.height();
        if let Some(c) = clicked {
            if selected.as_deref() == Some(c.as_str()) {
                super::detail::close(&ctx);
            } else {
                super::detail::open_group(&ctx, &c);
            }
        }
        if let Some(key) = end_req {
            super::detail::ask_end(&ctx, s, &key);
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

    }

    /// Arrow keys move the selection, Delete ends it (after a confirm), typing filters,
    /// Esc clears the selection or the search
    fn keyboard(&mut self, ctx: &egui::Context, focus_id: egui::Id, pitch: f32, selected: Option<&str>) {
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
                super::detail::close(ctx);
            }
            ctx.memory_mut(|m| m.surrender_focus(focus_id));
        }
        let n = self.rows.len();
        if n > 0 && (up || down || home || end) {
            let cur = selected.and_then(|k| self.rows.iter().position(|r| r.key == k));
            let i = match (cur, up, down, home) {
                (_, _, _, true) => 0,
                _ if end => n - 1,
                (None, _, _, _) => 0,
                (Some(c), true, _, _) => c.saturating_sub(1),
                (Some(c), _, true, _) => (c + 1).min(n - 1),
                (Some(c), _, _, _) => c,
            };
            super::detail::open_group(ctx, &self.rows[i].key);
            // keep the selected row in view
            let top = i as f32 * pitch;
            if top < self.scroll_y {
                self.scroll_to = Some(top);
            } else if top + pitch > self.scroll_y + self.view_h {
                self.scroll_to = Some(top + pitch - self.view_h);
            }
        }
        if del {
            if let Some(k) = selected {
                self.end_key = Some(k.to_string());
            }
        }
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
