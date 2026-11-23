//! The details panel: click any program (in Processes, on the Dashboard, or in a page's "Who's
//! using it" list) and it slides in from the right with what the program is, why it is busy,
//! its last minute of use, where it came from, and what you can do about it.
//!
//! A program's processes are always shown together (chrome.exe ×30). The panel lists them, and
//! clicking one opens that single process.

use super::widgets;
use crate::anim::*;
use crate::explain::{self, Ident, Res};
use crate::fmt;
use crate::sys::{self, net::Reach, procinfo, Proc, Snapshot};
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Painter, Rect, Sense, Stroke, Ui};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// What the panel shows: every process with one name, or one process
#[derive(Clone, PartialEq, Debug)]
pub enum Target {
    /// Lower-case process name
    Group(String),
    /// pid and start time (so a reused id never shows the wrong program)
    Proc(u32, i64),
}

fn req_id() -> egui::Id {
    egui::Id::new("detail_req")
}
fn shown_id() -> egui::Id {
    egui::Id::new("detail_shown")
}
fn end_id() -> egui::Id {
    egui::Id::new("detail_end")
}

/// Opens the panel on every process named `name`
pub fn open_group(ctx: &egui::Context, name: &str) {
    ctx.data_mut(|d| d.insert_temp(req_id(), Some(Target::Group(name.to_lowercase()))));
    ctx.request_repaint();
}

pub fn close(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(req_id(), None::<Target>));
    ctx.request_repaint();
}

/// The lower-case name of the program the panel shows, if it is open
pub fn shown_group(ctx: &egui::Context) -> Option<String> {
    ctx.data(|d| d.get_temp::<Option<String>>(shown_id())).flatten()
}

/// End task for every process named `key` (lower case): opens the panel on it and asks the
/// user to confirm, or explains why not
pub fn ask_end(ctx: &egui::Context, s: &Snapshot, key: &str) {
    let members: Vec<&Proc> = s.procs.iter().filter(|p| p.name.to_lowercase() == key).collect();
    let Some(first) = members.first() else { return };
    open_group(ctx, key);
    let req = match end_check(&members) {
        Ok(()) => Ok((first.name.clone(), members.iter().map(|p| p.pid).collect::<Vec<u32>>())),
        Err(e) => Err(e),
    };
    ctx.data_mut(|d| d.insert_temp(end_id(), Some(req)));
}

/// Ending several processes at once is refused for Windows' own: ending every svchost.exe,
/// say, would crash Windows. One at a time stays possible from each process's own page.
fn end_check(members: &[&Proc]) -> Result<(), String> {
    if members.len() < 2 {
        return Ok(());
    }
    let win = windows_dir();
    let system = members.iter().any(|m| crate::platform::end_blocked(m.pid, &m.name).is_some() || m.path.as_deref().is_some_and(|p| p.to_lowercase().starts_with(&win)));
    if system {
        Err("These are part of Windows; ending them all could crash it. Open one to end it on its own.".into())
    } else {
        Ok(())
    }
}

type EndReq = Option<Result<(String, Vec<u32>), String>>;

/// One process with what the panel worked out about it
struct Member<'a> {
    p: &'a Proc,
    ident: Option<Ident>,
    /// For web views: the app they show web pages for
    owner: Option<String>,
}

impl Member<'_> {
    /// One line for the process list
    fn label(&self, chromium_family: bool) -> String {
        let base = match (&self.ident, &self.owner) {
            (Some(i), _) => i.short(),
            (None, _) if chromium_family => "Main process".into(),
            (None, _) => format!("PID {}", self.p.pid),
        };
        match &self.owner {
            Some(o) => format!("{base} for {o}"),
            None => base,
        }
    }
}

pub struct Panel {
    target: Option<Target>,
    /// Where the back arrow goes (a group, after opening one of its processes)
    back: Vec<Target>,
    /// What was last drawn, kept while the panel slides away
    last: Option<Target>,
    /// Command lines, looked up once per process: (pid, start time) -> line
    cmd: HashMap<(u32, i64), Option<String>>,
    svc: HashMap<u32, Vec<String>>,
    svc_seq: u64,
    prio: Option<(u32, Instant, Option<u32>)>,
    peers: Option<(Vec<u32>, Instant, Vec<sys::net::Peer>)>,
    /// End task waiting for the user's confirmation: (name, pids)
    confirm: Option<(String, Vec<u32>)>,
    /// Result of the last action, shown for a few seconds
    note: Option<(String, bool, Instant)>,
}

impl Default for Panel {
    fn default() -> Self {
        Panel {
            target: None,
            back: Vec::new(),
            last: None,
            cmd: HashMap::new(),
            svc: HashMap::new(),
            svc_seq: 0,
            prio: None,
            peers: None,
            confirm: None,
            note: None,
        }
    }
}

/// The panel's width for this content area
pub fn width(content: Rect) -> f32 {
    (content.width() * 0.4).clamp(340.0, 440.0)
}

impl Panel {
    fn set(&mut self, t: Option<Target>) {
        if t != self.target {
            self.target = t;
            self.back.clear();
            self.prio = None;
        }
    }

    pub fn close_now(&mut self) {
        self.set(None);
    }

    fn apply_requests(&mut self, ctx: &egui::Context) {
        if let Some(r) = ctx.data(|d| d.get_temp::<Option<Target>>(req_id())) {
            ctx.data_mut(|d| d.remove::<Option<Target>>(req_id()));
            self.set(r);
        }
        if let Some(Some(req)) = ctx.data(|d| d.get_temp::<EndReq>(end_id())) {
            ctx.data_mut(|d| d.remove::<EndReq>(end_id()));
            match req {
                Ok(c) => self.confirm = Some(c),
                Err(e) => self.note = Some((e, false, Instant::now())),
            }
        }
    }

    /// How open the panel is, 0..1, for pages that make room for it
    pub fn openness(&mut self, ctx: &egui::Context) -> f32 {
        self.apply_requests(ctx);
        ease(ctx, egui::Id::new("detail_k"), self.target.is_some())
    }

    /// Draws the panel over the right side of `content`
    pub fn show(&mut self, ctx: &egui::Context, content: Rect, s: &Snapshot) {
        self.apply_requests(ctx);
        // dev captures can start with a program open: PURGEON_DETAIL=python.exe
        if let Some(n) = std::env::var("PURGEON_DETAIL").ok().filter(|_| !ctx.data(|d| d.get_temp::<bool>(egui::Id::new("detail_test")).unwrap_or(false))) {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("detail_test"), true));
            self.set(Some(Target::Group(n.to_lowercase())));
        }
        let k = ease(ctx, egui::Id::new("detail_k"), self.target.is_some());
        if self.target.is_some() {
            self.last = self.target.clone();
        }
        // what the page highlights
        let shown = self.target.as_ref().and_then(|t| match t {
            Target::Group(n) => Some(n.clone()),
            Target::Proc(pid, _) => s.procs.iter().find(|p| p.pid == *pid).map(|p| p.name.to_lowercase()),
        });
        ctx.data_mut(|d| d.insert_temp(shown_id(), shown));

        if self.target.is_some() && self.confirm.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) && ctx.memory(|m| m.focused().is_none()) {
            self.set(None);
        }
        self.confirm_dialog(ctx);
        if k < 0.01 {
            self.last = None;
            return;
        }
        let Some(target) = self.last.clone() else { return };
        if s.seq != self.svc_seq {
            self.svc_seq = s.seq;
            self.svc = procinfo::services();
            let live: std::collections::HashSet<(u32, i64)> = s.procs.iter().map(|p| (p.pid, p.created)).collect();
            self.cmd.retain(|key, _| live.contains(key));
        }

        let w = width(content);
        let r = Rect::from_min_max(pos2(content.max.x - w, content.min.y), content.max);
        let slide = (1.0 - k) * 40.0;
        egui::Area::new(egui::Id::new("detail_area"))
            .order(egui::Order::Middle)
            .fixed_pos(r.min)
            .constrain(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(content);
                ui.multiply_opacity(k);
                let (r, _) = ui.allocate_exact_size(r.size(), Sense::click());
                let r = r.translate(vec2(slide, 0.0));
                let p = ui.painter().clone();
                let sh = egui::epaint::Shadow { offset: vec2(-6.0, 0.0), blur: 24.0, spread: 0.0, color: Color32::from_black_alpha(90) };
                p.add(sh.as_shape(r, egui::Rounding::ZERO));
                p.rect_filled(r, egui::Rounding { nw: 14.0, ..Default::default() }, Color32::from_gray(0x1b));
                p.line_segment([r.left_top(), r.left_bottom()], Stroke::new(1.0_f32, border()));
                self.body(ui, r, s, &target);
            });
    }

    fn body(&mut self, ui: &mut Ui, r: Rect, s: &Snapshot, target: &Target) {
        let ctx = ui.ctx().clone();
        let members: Vec<&Proc> = match target {
            Target::Group(n) => s.procs.iter().filter(|p| p.name.to_lowercase() == *n).collect(),
            Target::Proc(pid, created) => s.procs.iter().filter(|p| p.pid == *pid && p.created == *created).collect(),
        };
        let pad = 22.0;
        let x0 = r.min.x + pad;
        let x1 = r.max.x - pad;
        let p = ui.painter().clone();

        // ---- header: back, icon, name, close
        let mut hx = x0;
        let hy = r.min.y + 34.0;
        if !self.back.is_empty() {
            let br = Rect::from_center_size(pos2(x0 + 10.0, hy), vec2(26.0, 26.0));
            let resp = ui.interact(br, egui::Id::new("detail_back"), Sense::click());
            let (h, _) = hover_fx(&ctx, &resp);
            p.circle_filled(br.center(), 13.0, wa((h * 22.0) as u8));
            let c = br.center();
            let st = Stroke::new(1.5_f32, mix(txt_dim(), hi(), h));
            p.line_segment([c + vec2(2.5, -5.0), c + vec2(-2.5, 0.0)], st);
            p.line_segment([c + vec2(-2.5, 0.0), c + vec2(2.5, 5.0)], st);
            if resp.clicked() {
                if let Some(t) = self.back.pop() {
                    self.target = Some(t);
                    self.prio = None;
                }
            }
            hx += 30.0;
        }
        let first = members.first().copied();
        let (name, path) = match (first, target) {
            (Some(m), _) => (m.name.clone(), m.path.clone()),
            (None, Target::Group(n)) => (n.clone(), None),
            (None, Target::Proc(pid, _)) => (format!("PID {pid}"), None),
        };
        super::icons::paint(&ctx, &p, Rect::from_center_size(pos2(hx + 17.0, hy), vec2(34.0, 34.0)), &name, path.as_ref());
        let base = name.trim_end_matches(".exe").trim_end_matches(".EXE");
        let title = match explain::about(&name) {
            Some((short, _)) if path.is_none() || name.eq_ignore_ascii_case("svchost.exe") => short.to_string(),
            _ => crate::platform::friendly_name(base, path.as_deref()),
        };
        let tx = hx + 44.0;
        widgets::fit(&p, pos2(tx, hy - 9.0), Align2::LEFT_CENTER, &title, sb(16.0), x1 - 30.0 - tx, hi());
        let sub = match (target, members.len()) {
            (_, 0) => format!("{name}  ·  not running any more"),
            (Target::Group(_), n) if n > 1 => format!("{name}  ·  {n} processes"),
            (_, _) => format!("{name}  ·  PID {}", members[0].pid),
        };
        widgets::fit(&p, pos2(tx, hy + 11.0), Align2::LEFT_CENTER, &sub, reg(12.0), x1 - 30.0 - tx, txt_faint());
        let cr = Rect::from_center_size(pos2(x1 - 6.0, hy - 6.0), vec2(26.0, 26.0));
        let resp = ui.interact(cr, egui::Id::new("detail_close"), Sense::click());
        let (h, _) = hover_fx(&ctx, &resp);
        p.circle_filled(cr.center(), 13.0, wa((h * 22.0) as u8));
        let c = cr.center();
        let st = Stroke::new(1.3_f32, mix(txt_dim(), hi(), h));
        p.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], st);
        p.line_segment([c + vec2(4.0, -4.0), c + vec2(-4.0, 4.0)], st);
        if resp.clicked() {
            self.set(None);
            return;
        }

        if members.is_empty() {
            p.text(pos2(x0, r.min.y + 96.0), Align2::LEFT_CENTER, "This program has closed.", sb(13.5), txt_dim());
            p.text(pos2(x0, r.min.y + 118.0), Align2::LEFT_CENTER, "Nothing with this name is running now.", reg(12.0), txt_faint());
            return;
        }

        // command lines for every member, looked up once each
        for m in &members {
            self.cmd.entry((m.pid, m.created)).or_insert_with(|| procinfo::command_line(m.pid));
        }
        let mut info: Vec<Member> = members
            .iter()
            .map(|m| {
                let svc = self.svc.get(&m.pid).map(|v| v.as_slice()).unwrap_or(&[]);
                let cmd = self.cmd.get(&(m.pid, m.created)).cloned().flatten();
                Member { p: m, ident: explain::identify(&m.name, cmd.as_deref(), svc), owner: web_owner(s, m) }
            })
            .collect();
        info.sort_by(|a, b| b.p.cpu.total_cmp(&a.p.cpu).then(b.p.ram().cmp(&a.p.ram())));
        let single = info.len() == 1;
        let pids: Vec<u32> = info.iter().map(|m| m.p.pid).collect();

        // open connections' far ends, refreshed every few seconds
        let tcp: u32 = info.iter().map(|m| m.p.conns.tcp).sum();
        if tcp == 0 {
            self.peers = None;
        } else if self.peers.as_ref().is_none_or(|(k, at, _)| *k != pids || at.elapsed() > Duration::from_secs(3)) {
            self.peers = Some((pids.clone(), Instant::now(), sys::net::peers(&pids)));
        }
        let peers: Vec<sys::net::Peer> = self.peers.as_ref().map(|x| x.2.clone()).unwrap_or_default();

        // ---- footer actions
        let foot_h = 58.0;
        let foot = Rect::from_min_max(pos2(r.min.x, r.max.y - foot_h), r.max);
        p.line_segment([pos2(r.min.x + 1.0, foot.min.y), pos2(r.max.x, foot.min.y)], Stroke::new(1.0_f32, border()));
        self.footer(ui, foot.shrink2(vec2(pad, 12.0)), &info, path.as_deref(), &name);

        // ---- the scrolling middle
        let body = Rect::from_min_max(pos2(r.min.x, r.min.y + 64.0), pos2(r.max.x, foot.min.y));
        let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("detail_body"));
        bui.set_clip_rect(body.intersect(ui.clip_rect()));
        let scroll_id = egui::Id::new(("detail_scroll", format!("{target:?}")));
        egui::ScrollArea::vertical().id_salt(scroll_id).auto_shrink(false).show(&mut bui, |sui| {
            sui.add_space(8.0);
            let w = x1 - x0;
            let mut col = sui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(x0, sui.cursor().min.y), vec2(w, 100_000.0))));
            {
                let ui = &mut col;
                ui.spacing_mut().item_spacing.y = 0.0;
                self.what_it_is(ui, s, &info, w);
                self.why(ui, s, &info, &peers, w);
                self.graphs(ui, s, &info, w);
                if single {
                    self.facts(ui, s, &info[0], w);
                } else {
                    self.member_list(ui, &info, target, w);
                }
                if !peers.is_empty() {
                    connections(ui, &peers, w);
                }
                ui.add_space(18.0);
            }
            let used = col.min_rect().height();
            sui.allocate_exact_size(vec2(1.0, used), Sense::hover());
        });

        // ---- result of the last action
        if let Some((msg, ok, at)) = &self.note {
            let age = at.elapsed().as_secs_f32();
            if age < 5.0 {
                let nr = Rect::from_min_max(pos2(x0, foot.min.y - 46.0), pos2(x1, foot.min.y - 10.0));
                p.rect_filled(nr, 10.0, Color32::from_gray(0x2a));
                p.rect_stroke(nr, 10.0, Stroke::new(1.0_f32, border()));
                p.circle_filled(pos2(nr.min.x + 16.0, nr.center().y), 3.5, if *ok { GPU_COL } else { DANGER });
                widgets::fit(&p, pos2(nr.min.x + 28.0, nr.center().y), Align2::LEFT_CENTER, msg, reg(12.0), nr.width() - 40.0, txt());
                ctx.request_repaint_after(Duration::from_secs_f32(5.0 - age));
            } else {
                self.note = None;
            }
        }
    }

    /// "What it is": the script or job in plain words, then what the program is in general
    fn what_it_is(&self, ui: &mut Ui, s: &Snapshot, info: &[Member], w: f32) {
        heading(ui, "What it is");
        let first = &info[0];
        let name = &first.p.name;
        let about = explain::about(name);
        let file = first.p.path.as_deref().map(procinfo::file_info).unwrap_or_default();
        let mut lines: Vec<(String, egui::FontId, Color32)> = Vec::new();
        if info.len() == 1 {
            match &first.ident {
                Some(id) => {
                    lines.push((id.kind.clone(), sb(14.0), hi()));
                    if let Some(t) = &id.target {
                        lines.push((t.clone(), reg(12.5), txt()));
                    }
                }
                None => {
                    let head = file.description.clone().or_else(|| about.map(|a| a.0.to_string())).unwrap_or_else(|| name.clone());
                    lines.push((head, sb(14.0), hi()));
                }
            }
            if let Some(o) = &first.owner {
                lines.push((format!("Shows web content for {o}"), reg(12.5), txt()));
            }
        } else {
            // what the copies are doing, summed up: "4 Python scripts: scraper.py, bot.py, …"
            let mut kinds: Vec<(String, Vec<String>)> = Vec::new();
            for m in info {
                let (k, t) = match &m.ident {
                    Some(id) => (id.kind.clone(), id.target.as_deref().map(|t| explain::file_name(t).to_string())),
                    None => continue,
                };
                match kinds.iter_mut().find(|x| x.0 == k) {
                    Some(x) => x.1.extend(t),
                    None => kinds.push((k, t.into_iter().collect())),
                }
            }
            let svchost = name.eq_ignore_ascii_case("svchost.exe");
            let head = if svchost {
                let n: usize = info.iter().map(|m| self.svc.get(&m.p.pid).map_or(0, |v| v.len())).sum();
                format!("{} copies hosting {n} Windows services", info.len())
            } else {
                about.map(|a| a.0.to_string()).or_else(|| file.description.clone()).unwrap_or_else(|| name.clone())
            };
            lines.push((head, sb(14.0), hi()));
            let owners: Vec<String> = {
                let mut v: Vec<String> = info.iter().filter_map(|m| m.owner.clone()).collect();
                v.sort();
                v.dedup();
                v
            };
            if !owners.is_empty() {
                lines.push((format!("Shows web content for {}", owners.join(", ")), reg(12.5), txt()));
            }
            if !svchost {
                for (k, targets) in kinds.iter().take(3) {
                    let mut t: Vec<String> = targets.clone();
                    t.dedup();
                    let n = info.iter().filter(|m| m.ident.as_ref().is_some_and(|i| i.kind == *k)).count();
                    let list = if t.is_empty() {
                        String::new()
                    } else if t.len() > 3 {
                        format!(": {}, +{} more", t[..3].join(", "), t.len() - 3)
                    } else {
                        format!(": {}", t.join(", "))
                    };
                    lines.push((format!("{n} × {k}{list}"), reg(12.5), txt()));
                }
            }
        }
        if let Some((_, long)) = about {
            lines.push((long.to_string(), reg(12.0), txt_dim()));
        } else if let Some(c) = &file.company {
            lines.push((format!("Made by {c}"), reg(12.0), txt_dim()));
        }
        if first.p.session == 0 && info.len() == 1 && !s.info.admin && first.p.path.is_none() {
            lines.push(("Windows keeps some details of background processes private. Click Elevate to see more.".into(), reg(11.5), txt_faint()));
        }

        // a soft box around it
        let galleys: Vec<_> = lines.iter().map(|(t, f, c)| ui.painter().layout(t.clone(), f.clone(), *c, w - 28.0)).collect();
        let h: f32 = galleys.iter().map(|g| g.size().y).sum::<f32>() + 6.0 * (galleys.len() as f32 - 1.0).max(0.0) + 28.0;
        let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, 10.0, ACCENT.gamma_multiply(0.07));
        p.rect_stroke(r, 10.0, Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.25)));
        let mut y = r.min.y + 14.0;
        for g in galleys {
            let gh = g.size().y;
            p.galley(pos2(r.min.x + 14.0, y), g, txt());
            y += gh + 6.0;
        }
    }

    /// "Why it's busy": plain sentences from the numbers
    fn why(&self, ui: &mut Ui, s: &Snapshot, info: &[Member], peers: &[sys::net::Peer], w: f32) {
        let pids: Vec<u32> = info.iter().map(|m| m.p.pid).collect();
        let hist = sys::history(&pids);
        let sum = |f: &dyn Fn(&Proc) -> f64| info.iter().map(|m| f(m.p)).sum::<f64>();
        let busiest_gpu = info.iter().max_by(|a, b| a.p.gpu.total_cmp(&b.p.gpu)).map_or("", |m| m.p.gpu_engine);
        let facts = explain::Facts {
            count: info.len() as u32,
            cpu: sum(&|p| p.cpu as f64) as f32,
            logical: s.info.logical,
            cpu_hist: &hist[sys::PH_CPU],
            ram_hist: &hist[sys::PH_RAM],
            interval: s.interval.max(0.25),
            ram: sum(&|p| p.private_ws as f64) as u64 + info.iter().map(|m| m.p.shared()).max().unwrap_or(0),
            total_ram: s.mem.total,
            commit: sum(&|p| p.commit as f64) as u64,
            gpu: s.gpu_proc.then(|| info.iter().map(|m| m.p.gpu).fold(0.0, f32::max)),
            gpu_engine: busiest_gpu,
            vram: sum(&|p| p.vram as f64) as u64,
            disk: s.disk_trace.then(|| (sum(&|p| p.disk_read), sum(&|p| p.disk_write))),
            io: sum(&|p| p.io_read + p.io_write),
            net: s.net_trace.then(|| (sum(&|p| p.net_rx), sum(&|p| p.net_tx))),
            tcp: info.iter().map(|m| m.p.conns.tcp).sum(),
            listen: info.iter().map(|m| m.p.conns.listen).sum(),
            internet_peers: {
                let mut a: Vec<&str> = peers.iter().filter(|x| x.reach == Reach::Internet).map(|x| x.addr.as_str()).collect();
                a.sort();
                a.dedup();
                a.len()
            },
            handles: info.iter().map(|m| m.p.handles).sum(),
            threads: info.iter().map(|m| m.p.threads).sum(),
        };
        let hints = explain::hints(&facts);
        ui.add_space(20.0);
        heading(ui, if hints.iter().any(|h| h.weight >= 1.0) { "Why it's busy" } else { "Right now" });
        for h in &hints {
            let g = ui.painter().layout(h.text.clone(), reg(12.5), txt(), w - 18.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y + 9.0), Sense::hover());
            let col = match h.res {
                Res::Cpu => CPU_COL,
                Res::Mem => MEM_COL,
                Res::Disk => DISK_COL,
                Res::Gpu => GPU_COL,
                Res::Net => NET_COL,
                Res::Note => txt_faint(),
            };
            ui.painter().circle_filled(pos2(r.min.x + 4.0, r.min.y + 8.5), 3.5, col);
            ui.painter().galley(pos2(r.min.x + 16.0, r.min.y), g, txt());
        }
        if !s.disk_trace || !s.net_trace {
            let g = ui.painter().layout("Click Elevate (bottom left) to also see disk and network use per program.".to_string(), reg(11.5), txt_faint(), w);
            ui.add_space(2.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
            ui.painter().galley(r.min, g, txt_faint());
        }
    }

    /// The last minute of CPU, memory, disk, GPU and network, one small graph each
    fn graphs(&self, ui: &mut Ui, s: &Snapshot, info: &[Member], w: f32) {
        ui.add_space(20.0);
        heading(ui, "Last minute");
        let pids: Vec<u32> = info.iter().map(|m| m.p.pid).collect();
        let mut h = sys::history(&pids);
        // memory as the Processes list counts it: each copy's own memory, plus the shared part once
        let shared = info.iter().map(|m| m.p.shared()).max().unwrap_or(0) as f32;
        for v in h[sys::PH_RAM].iter_mut() {
            *v += shared;
        }
        let last = |k: usize| h[k].last().copied().unwrap_or(0.0);
        let peak = |k: usize, floor: f32| h[k].iter().copied().fold(floor, f32::max) * 1.15;
        let mut rows: Vec<(&str, String, usize, f32, Color32)> = vec![
            ("CPU", fmt::pct(last(sys::PH_CPU)), sys::PH_CPU, peak(sys::PH_CPU, 5.0), CPU_COL),
            ("Memory", fmt::bytes(last(sys::PH_RAM) as u64), sys::PH_RAM, peak(sys::PH_RAM, 64.0 * 1048576.0), MEM_COL),
            (if s.disk_trace { "Disk" } else { "All I/O" }, fmt::rate(last(sys::PH_DISK) as f64), sys::PH_DISK, peak(sys::PH_DISK, 1048576.0), DISK_COL),
        ];
        if s.gpu_proc {
            rows.push(("GPU ≈", fmt::pct(last(sys::PH_GPU)), sys::PH_GPU, peak(sys::PH_GPU, 10.0), GPU_COL));
        }
        if s.net_trace {
            rows.push(("Network ≈", fmt::bits(last(sys::PH_NET) as f64), sys::PH_NET, peak(sys::PH_NET, 131072.0), NET_COL));
        }
        let scroll = if reduced() { 1.0 } else { egui::emath::easing::cubic_out((s.at.elapsed().as_secs_f32() / 0.35).min(1.0)) };
        for (label, value, k, max, col) in rows {
            let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
            let p = ui.painter();
            p.text(pos2(r.min.x, r.min.y + 12.0), Align2::LEFT_CENTER, label, reg(11.5), txt_faint());
            p.text(pos2(r.min.x, r.min.y + 28.0), Align2::LEFT_CENTER, value, sb(12.5), txt());
            let gr = Rect::from_min_max(pos2(r.min.x + 96.0, r.min.y + 4.0), pos2(r.max.x, r.max.y - 4.0));
            p.rect_filled(gr, 6.0, well());
            spark(p, gr.shrink2(vec2(1.0, 2.0)), &h[k], max, col, scroll);
        }
        if s.gpu_proc || s.net_trace {
            ui.add_space(4.0);
            let g = ui.painter().layout("≈ Per-program GPU and network figures are Windows' own estimates.".to_string(), reg(11.0), txt_faint(), w);
            let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
            ui.painter().galley(r.min, g, txt_faint());
        }
    }

    /// One process: where it came from, its file, its command line
    fn facts(&mut self, ui: &mut Ui, s: &Snapshot, m: &Member, w: f32) {
        ui.add_space(20.0);
        heading(ui, "Details");
        let p = m.p;
        let ctx = ui.ctx().clone();
        let now = procinfo::now_filetime();
        if p.created > 0 {
            let age = (now - p.created) as f64 / 1e7;
            let when = procinfo::local_time(p.created).map(|t| format!("{t}  ·  {} ago", fmt::span(age))).unwrap_or_else(|| format!("{} ago", fmt::span(age)));
            kv(ui, "Started", &when, w);
        }
        let parent = sys::parent_of(&s.procs, p);
        let scheduler = parent.is_some_and(|q| self.svc.get(&q.pid).is_some_and(|v| v.iter().any(|x| x == "Task Scheduler")));
        let origin = explain::origin(parent.map(|q| q.name.as_str()), p.session, scheduler);
        kv(ui, "Started by", &origin, w);
        if let Some(q) = parent {
            // the parent, one click away
            let (r, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
            let lr = Rect::from_min_max(pos2(r.min.x + 96.0, r.min.y), pos2(r.max.x, r.min.y + 22.0));
            let resp = ui.interact(lr, egui::Id::new("detail_parent"), Sense::click());
            let (h, _) = hover_fx(&ctx, &resp);
            ui.painter().rect_filled(lr, 6.0, wa((h * 12.0) as u8));
            super::icons::paint(&ctx, ui.painter(), Rect::from_center_size(pos2(lr.min.x + 13.0, lr.center().y), vec2(16.0, 16.0)), &q.name, q.path.as_ref());
            widgets::fit(ui.painter(), pos2(lr.min.x + 28.0, lr.center().y), Align2::LEFT_CENTER, &format!("Open {} (PID {})", q.name, q.pid), reg(12.0), lr.width() - 34.0, mix(ACCENT, hi(), h * 0.3));
            if resp.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                let cur = self.target.clone();
                self.target = Some(Target::Proc(q.pid, q.created));
                self.back.extend(cur);
                self.prio = None;
            }
            ui.add_space(6.0);
        }
        if p.session == 0 {
            kv(ui, "Runs as", "A background process (session 0), not in your desktop session", w);
        }
        if let Some(svc) = self.svc.get(&p.pid) {
            let list = if svc.len() > 8 { format!("{}, and {} more", svc[..8].join(", "), svc.len() - 8) } else { svc.join(", ") };
            kv(ui, if svc.len() == 1 { "Service" } else { "Services" }, &list, w);
        }
        match p.path.as_deref() {
            Some(path) => {
                kv(ui, "File", path, w);
                let f = procinfo::file_info(path);
                if let Some(c) = &f.company {
                    kv(ui, "Made by", c, w);
                }
                if let Some(d) = f.description.as_ref().or(f.product.as_ref()) {
                    kv(ui, "Description", d, w);
                }
                if let Some(v) = &f.version {
                    kv(ui, "Version", v, w);
                }
            }
            None => kv(ui, "File", if s.info.admin { "Windows keeps this one private" } else { "Hidden. Click Elevate to see it" }, w),
        }
        if self.prio.as_ref().is_none_or(|(pid, at, _)| *pid != p.pid || at.elapsed() > Duration::from_secs(2)) {
            self.prio = Some((p.pid, Instant::now(), procinfo::priority(p.pid)));
        }
        if let Some((_, _, Some(c))) = self.prio {
            kv(ui, "Priority", procinfo::priority_name(c), w);
        }
        kv(ui, "Inside", &format!("{} threads  ·  {} handles", fmt::thousands(p.threads as u64), fmt::thousands(p.handles as u64)), w);
        let cmd = self.cmd.get(&(p.pid, p.created)).cloned().flatten();
        ui.add_space(8.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
        ui.painter().text(pos2(r.min.x, r.center().y), Align2::LEFT_CENTER, "Command line", reg(12.0), txt_faint());
        match cmd {
            Some(c) => {
                let lr = Rect::from_min_max(pos2(r.max.x - 44.0, r.min.y), r.max);
                let resp = ui.interact(lr, egui::Id::new("detail_copy_cmd"), Sense::click());
                let (h, _) = hover_fx(&ctx, &resp);
                ui.painter().text(pos2(r.max.x, r.center().y), Align2::RIGHT_CENTER, "Copy", sb(11.5), mix(ACCENT, hi(), h * 0.3));
                if resp.clicked() {
                    ctx.output_mut(|o| o.copied_text = c.clone());
                    self.note = Some(("Command line copied.".into(), true, Instant::now()));
                }
                ui.add_space(4.0);
                let g = ui.painter().layout(c, egui::FontId::monospace(11.5), txt_dim(), w - 20.0);
                let gh = g.size().y.min(160.0);
                let (r, _) = ui.allocate_exact_size(vec2(w, gh + 16.0), Sense::hover());
                ui.painter().rect_filled(r, 8.0, well());
                ui.painter().with_clip_rect(r.shrink(6.0)).galley(pos2(r.min.x + 10.0, r.min.y + 8.0), g, txt_dim());
            }
            None => {
                let t = if s.info.admin { "Windows keeps this one private." } else { "Hidden. Click Elevate to see it." };
                let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
                ui.painter().text(pos2(r.min.x, r.center().y), Align2::LEFT_CENTER, t, reg(12.0), txt_dim());
            }
        }
    }

    /// A program's processes, busiest first; click one to open it
    fn member_list(&mut self, ui: &mut Ui, info: &[Member], target: &Target, w: f32) {
        ui.add_space(20.0);
        heading(ui, &format!("Its {} processes", info.len()));
        let ctx = ui.ctx().clone();
        let chromium = info.iter().any(|m| m.ident.as_ref().is_some_and(|i| is_role(&i.kind)));
        let mut open = None;
        for (i, m) in info.iter().enumerate().take(60) {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 44.0), Sense::click());
            let (h, _) = hover_fx(&ctx, &resp);
            let p = ui.painter();
            p.rect_filled(r.expand2(vec2(8.0, 0.0)), 8.0, wa((h * 10.0) as u8));
            let label = m.label(chromium);
            widgets::fit(p, pos2(r.min.x, r.min.y + 14.0), Align2::LEFT_CENTER, &label, reg(12.5), w - 110.0, txt());
            let sub = format!("PID {}  ·  {}", m.p.pid, fmt::bytes(m.p.ram()));
            p.text(pos2(r.min.x, r.min.y + 31.0), Align2::LEFT_CENTER, sub, reg(11.0), txt_faint());
            let cpu = fmt::pct(m.p.cpu);
            p.text(pos2(r.max.x - 14.0, r.center().y), Align2::RIGHT_CENTER, format!("{cpu} CPU"), reg(12.0), if m.p.cpu < 0.05 { txt_faint() } else { txt() });
            super::pages::chevron_right(p, pos2(r.max.x - 3.0, r.center().y), txt_faint());
            if resp.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                open = Some(Target::Proc(m.p.pid, m.p.created));
            }
            if i + 1 < info.len().min(60) {
                ui.painter().line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0_f32, wa(8)));
            }
        }
        if info.len() > 60 {
            let (r, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
            ui.painter().text(pos2(r.min.x, r.center().y), Align2::LEFT_CENTER, format!("and {} more", info.len() - 60), reg(12.0), txt_faint());
        }
        if let Some(t) = open {
            self.back.push(target.clone());
            self.target = Some(t);
            self.prio = None;
        }
    }

    fn footer(&mut self, ui: &mut Ui, r: Rect, info: &[Member], path: Option<&str>, name: &str) {
        let single = info.len() == 1;
        let pids: Vec<u32> = info.iter().map(|m| m.p.pid).collect();
        let mut x = r.min.x;
        let h = r.height();
        if let Some(path) = path {
            let b = Rect::from_min_size(pos2(x, r.min.y), vec2(108.0, h));
            if button(ui, b, "Open location", egui::Id::new("detail_loc"), false).clicked() {
                crate::platform::open_location(path);
            }
            x = b.max.x + 8.0;
        }
        let b = Rect::from_min_size(pos2(x, r.min.y), vec2(72.0, h));
        if button(ui, b, "Search", egui::Id::new("detail_search"), false).clicked() {
            // a single service host is best searched by its service
            let svc = single.then(|| self.svc.get(&pids[0]).filter(|v| v.len() == 1).map(|v| v[0].clone())).flatten();
            let q = match (svc, info[0].ident.as_ref().and_then(|i| i.target.clone())) {
                (Some(s), _) => format!("{s} Windows service"),
                _ => format!("{name} process"),
            };
            crate::platform::search_online(&q);
        }
        x = b.max.x + 8.0;
        if single {
            let b = Rect::from_min_size(pos2(x, r.min.y), vec2(74.0, h));
            let resp = button(ui, b, "Priority", egui::Id::new("detail_prio"), false);
            let pop = egui::Id::new("detail_prio_pop");
            if resp.clicked() {
                ui.memory_mut(|m| m.toggle_popup(pop));
            }
            let cur = self.prio.and_then(|x| x.2);
            let mut picked = None;
            egui::popup::popup_above_or_below_widget(ui, pop, &resp, egui::AboveOrBelow::Above, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(150.0);
                for (class, label) in procinfo::PRIORITIES.iter().rev() {
                    let t = if cur == Some(*class) { format!("✓  {label}") } else { format!("     {label}") };
                    if ui.button(t).clicked() {
                        picked = Some(*class);
                    }
                }
            });
            if let Some(c) = picked {
                let pid = pids[0];
                self.note = Some(match procinfo::set_priority(pid, c) {
                    Ok(()) => (format!("Priority set to {}.", procinfo::priority_name(c)), true, Instant::now()),
                    Err(e) => (format!("Couldn't change priority: {e}"), false, Instant::now()),
                });
                self.prio = None;
            }
        }
        // End task, on the right
        let label = if single { "End task" } else { "End all" };
        let b = Rect::from_min_max(pos2(r.max.x - 86.0, r.min.y), r.max);
        if button(ui, b, label, egui::Id::new("detail_end"), true).clicked() {
            let members: Vec<&Proc> = info.iter().map(|m| m.p).collect();
            match end_check(&members) {
                Ok(()) => self.confirm = Some((name.to_string(), pids)),
                Err(e) => self.note = Some((e, false, Instant::now())),
            }
        }
    }

    /// "End chrome.exe?" with Cancel and End task. Enter confirms, Esc cancels.
    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some((name, pids)) = self.confirm.clone() else { return };
        let blocked = if pids.len() == 1 { crate::platform::end_blocked(pids[0], &name) } else { None };
        let (mut go, mut cancel) = ctx.input(|i| (i.key_pressed(egui::Key::Enter) && blocked.is_none(), i.key_pressed(egui::Key::Escape)));
        let screen = ctx.screen_rect();
        ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("end_dim"))).rect_filled(screen, 0.0, Color32::from_black_alpha(110));
        let n = pids.len();
        egui::Window::new("end_task")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([380.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                let title = if n == 1 { format!("End {name}?") } else { format!("End all {n} {name} processes?") };
                ui.label(egui::RichText::new(title).font(sb(16.0)).color(hi()));
                ui.add_space(2.0);
                let sub = if n == 1 { format!("PID {}", pids[0]) } else { "Every process with this name".into() };
                ui.label(egui::RichText::new(sub).font(reg(12.0)).color(txt_faint()));
                ui.add_space(8.0);
                let body = blocked.unwrap_or("The program closes immediately. Anything unsaved in it is lost.");
                ui.label(egui::RichText::new(body).font(reg(12.5)).color(txt_dim()));
                ui.add_space(14.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if blocked.is_none() {
                        let b = egui::Button::new(egui::RichText::new(if n == 1 { "End task" } else { "End all" }).font(sb(12.5)).color(Color32::WHITE))
                            .fill(DANGER)
                            .min_size(vec2(92.0, 30.0));
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
            let mut errs = Vec::new();
            for pid in &pids {
                if let Err(e) = crate::platform::end_process(*pid, &name) {
                    errs.push(e);
                }
            }
            self.note = Some(match (n, errs.first()) {
                (1, None) => (format!("Ended {name} (PID {}).", pids[0]), true, Instant::now()),
                (1, Some(e)) => (format!("Couldn't end {name}: {e}"), false, Instant::now()),
                (_, None) => (format!("Ended all {n} {name} processes."), true, Instant::now()),
                (_, Some(e)) => (format!("Ended {} of {n}. {e}", n - errs.len()), false, Instant::now()),
            });
        } else if cancel {
            self.confirm = None;
        }
    }
}

fn windows_dir() -> String {
    let w = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    format!("{}\\", w.to_lowercase().trim_end_matches('\\'))
}

/// A Chromium process role (so the main process, which has none, can be named)
fn is_role(kind: &str) -> bool {
    matches!(
        kind,
        "Tab or site" | "Extension" | "Window content" | "Graphics helper" | "Crash reporter" | "Security broker" | "Network service" | "Storage service" | "Audio service" | "Camera service" | "File decoder" | "Helper" | "Media decoder"
    )
}

/// The app a WebView2 process shows web pages for: the first ancestor that isn't WebView2
fn web_owner(s: &Snapshot, p: &Proc) -> Option<String> {
    if !p.name.eq_ignore_ascii_case("msedgewebview2.exe") {
        return None;
    }
    let mut cur = p;
    for _ in 0..8 {
        let q = sys::parent_of(&s.procs, cur)?;
        if !q.name.eq_ignore_ascii_case("msedgewebview2.exe") {
            let base = q.name.trim_end_matches(".exe").trim_end_matches(".EXE");
            return Some(crate::platform::friendly_name(base, q.path.as_deref()));
        }
        cur = q;
    }
    None
}

fn heading(ui: &mut Ui, t: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    ui.painter().text(pos2(r.min.x, r.min.y + 9.0), Align2::LEFT_CENTER, t.to_uppercase(), sb(10.5), txt_faint());
}

/// A label on the left and a value that wraps on the right
fn kv(ui: &mut Ui, label: &str, value: &str, w: f32) {
    let g = ui.painter().layout(value.to_string(), reg(12.5), txt(), w - 96.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y.max(16.0) + 10.0), Sense::hover());
    ui.painter().text(pos2(r.min.x, r.min.y + 8.0), Align2::LEFT_CENTER, label, reg(12.0), txt_faint());
    ui.painter().galley(pos2(r.min.x + 96.0, r.min.y), g, txt());
}

/// The connections' far ends: "142.250.74.14  ·  secure web  ·  3"
fn connections(ui: &mut Ui, peers: &[sys::net::Peer], w: f32) {
    ui.add_space(20.0);
    let n: u32 = peers.iter().map(|x| x.count).sum();
    heading(ui, &format!("Connected to  ·  {n}"));
    for x in peers.iter().take(8) {
        let (r, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
        let p = ui.painter();
        let (dot, reach) = match x.reach {
            Reach::Internet => (NET_COL, "internet"),
            Reach::Local => (NET_UP_COL, "your network"),
            Reach::ThisPc => (txt_faint(), "this PC"),
        };
        p.circle_filled(pos2(r.min.x + 4.0, r.center().y), 3.0, dot);
        let what = match x.port {
            443 => "secure web".to_string(),
            80 => "web".to_string(),
            53 => "DNS".to_string(),
            22 => "SSH".to_string(),
            3389 => "remote desktop".to_string(),
            p => format!("port {p}"),
        };
        widgets::fit(p, pos2(r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, &format!("{}  ·  {what}", x.addr), reg(12.0), w - 150.0, txt());
        let c = if x.count > 1 { format!("{reach}  ·  ×{}", x.count) } else { reach.to_string() };
        p.text(pos2(r.max.x, r.center().y), Align2::RIGHT_CENTER, c, reg(11.5), txt_faint());
    }
    if peers.len() > 8 {
        let (r, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
        ui.painter().text(pos2(r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, format!("and {} more addresses", peers.len() - 8), reg(11.5), txt_faint());
    }
}

/// A small filled line graph, newest sample on the right, sliding in as it arrives
fn spark(p: &Painter, r: Rect, data: &[f32], max: f32, col: Color32, scroll: f32) {
    const N: usize = 60;
    let n = data.len();
    if n < 2 {
        return;
    }
    let p = p.with_clip_rect(r.intersect(p.clip_rect()));
    let dx = r.width() / (N - 1) as f32;
    let shift = (1.0 - scroll) * dx;
    let pts: Vec<egui::Pos2> = data
        .iter()
        .enumerate()
        .map(|(i, v)| pos2(r.max.x - (n - 1 - i) as f32 * dx + shift, r.max.y - (v / max.max(1e-6)).clamp(0.0, 1.0) * (r.height() - 2.0) - 1.0))
        .collect();
    let mut m = egui::epaint::Mesh::default();
    let top = col.gamma_multiply(0.28);
    let low = col.gamma_multiply(0.02);
    for q in &pts {
        m.colored_vertex(*q, top);
        m.colored_vertex(pos2(q.x, r.max.y), low);
    }
    for i in 1..pts.len() as u32 {
        let j = i * 2;
        m.add_triangle(j - 2, j - 1, j);
        m.add_triangle(j - 1, j + 1, j);
    }
    p.add(egui::Shape::mesh(m));
    p.add(egui::Shape::line(pts, Stroke::new(1.4_f32, col)));
}

/// A rounded text button; `danger` paints it red
fn button(ui: &mut Ui, r: Rect, label: &str, id: egui::Id, danger: bool) -> egui::Response {
    let resp = ui.interact(r, id, Sense::click());
    let (h, pr) = hover_fx(ui.ctx(), &resp);
    let p = ui.painter();
    let r = r.shrink(pr * 1.0);
    if danger {
        p.rect_filled(r, 9.0, DANGER.gamma_multiply(0.10 + 0.16 * h));
        p.rect_stroke(r, 9.0, Stroke::new(1.0_f32, DANGER.gamma_multiply(0.45 + 0.3 * h)));
    } else {
        p.rect_filled(r, 9.0, mix(Color32::from_gray(0x26), Color32::from_gray(0x30), h));
        p.rect_stroke(r, 9.0, Stroke::new(1.0_f32, border()));
    }
    widgets::fit(p, r.center(), Align2::CENTER_CENTER, label, sb(12.0), r.width() - 12.0, mix(txt(), hi(), h));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}
