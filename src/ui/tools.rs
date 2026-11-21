//! The Overview page's two helpers, side by side: "Boost an app" and "Free up memory".
//! Both say plainly what they do and what they can't do.

use super::widgets;
use crate::apps;
use crate::boost::{self, Plan, Priority};
use crate::fmt;
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Rect, RichText, Sense, Ui};
use std::sync::Mutex;

pub const H: f32 = 258.0;

struct State {
    /// The app picked in the Boost list (exe), before Boost is clicked
    pick: Option<String>,
    /// Close waiting for the user's yes: exe, name, has a window
    confirm: Option<(String, String, bool)>,
}
static STATE: Mutex<State> = Mutex::new(State { pick: None, confirm: None });

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn cards(ui: &mut Ui, w: f32, gap: f32) {
    let (row, _) = ui.allocate_exact_size(vec2(w, H), Sense::hover());
    let hw = (w - gap) / 2.0;
    boost_card(ui, Rect::from_min_size(row.min, vec2(hw, H)));
    free_card(ui, Rect::from_min_size(pos2(row.min.x + hw + gap, row.min.y), vec2(hw, H)));
    confirm(ui.ctx());
}

/// A button placed at `r` without moving the page's layout
fn button(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, label: &str, primary: bool, enabled: bool) -> bool {
    let mut b = egui::Button::new(RichText::new(label).font(sb(12.5)).color(if primary && enabled { Color32::WHITE } else { txt() }));
    if primary {
        b = b.fill(ACCENT);
    }
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(r).id_salt(id));
    c.add_enabled(enabled, b.min_size(r.size())).clicked()
}

/// A wrapped paragraph; returns the y below it
fn para(ui: &Ui, x: f32, y: f32, w: f32, text: &str, col: Color32) -> f32 {
    let g = ui.painter().layout(text.to_string(), reg(12.0), col, w);
    let h = g.size().y;
    ui.painter().galley(pos2(x, y), g, col);
    y + h
}

/// A status line with a coloured dot
fn line(ui: &Ui, x: f32, y: f32, w: f32, dot: Color32, text: &str) {
    let p = ui.painter();
    p.circle_filled(pos2(x + 4.0, y), 3.5, dot);
    widgets::fit(p, pos2(x + 16.0, y), Align2::LEFT_CENTER, text, reg(12.5), w - 16.0, txt());
}

const OK: Color32 = GPU_COL;

fn boost_card(ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    widgets::card(&p, r, card());
    let (x, w) = (r.min.x + 18.0, r.width() - 36.0);
    super::pages::caption_w(&p, x, r.min.y + 24.0, "Boost an app", ACCENT, w);
    let foot = "Helps a little when your CPU is busy. It can't add FPS when the graphics card is what holds a game back.";

    match boost::status() {
        None => {
            let y = para(ui, x, r.min.y + 44.0, w, "Pick a game or app. While it runs, Windows serves it first and uses its fastest power plan. Everything goes back when it closes.", txt_dim());
            let open = apps::open_apps();
            let mut st = state();
            if st.pick.as_ref().is_some_and(|e| !open.iter().any(|a| &a.exe == e)) {
                st.pick = None;
            }
            let shown = st.pick.as_ref().and_then(|e| open.iter().find(|a| &a.exe == e)).map_or("Choose an open app…".to_string(), |a| a.name.clone());
            let cr = Rect::from_min_size(pos2(x, y + 14.0), vec2(w - 96.0, 30.0));
            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(cr).id_salt("boost_pick"));
            egui::ComboBox::from_id_salt("boost_combo").width(cr.width() - 8.0).selected_text(RichText::new(shown).font(reg(12.5))).show_ui(&mut cui, |ui| {
                ui.set_min_width(cr.width() - 8.0);
                if open.is_empty() {
                    ui.label(RichText::new("No apps with a window are open").font(reg(12.5)).color(txt_faint()));
                }
                for a in open.iter().take(20) {
                    let label = format!("{}   {}", a.name, fmt::bytes(a.mem));
                    if ui.selectable_label(st.pick.as_deref() == Some(a.exe.as_str()), RichText::new(label).font(reg(12.5))).clicked() {
                        st.pick = Some(a.exe.clone());
                    }
                }
            });
            let br = Rect::from_min_size(pos2(r.max.x - 18.0 - 86.0, y + 14.0), vec2(86.0, 30.0));
            let pick = st.pick.clone();
            drop(st);
            let chosen = pick.and_then(|e| open.into_iter().find(|a| a.exe == e));
            if button(ui, br, "boost_go", "Boost", true, chosen.is_some()) {
                if let Some(a) = &chosen {
                    boost::start(&a.exe, &a.name);
                    state().pick = None;
                }
            }
            para(ui, x, r.max.y - 44.0, w, foot, txt_faint());
        }
        Some(b) => {
            let nr = widgets::fit(&p, pos2(x, r.min.y + 56.0), Align2::LEFT_CENTER, &b.name, sb(20.0), w - 110.0, hi());
            p.text(pos2(nr.max.x + 8.0, r.min.y + 58.0), Align2::LEFT_CENTER, if b.running { "boosted" } else { "not running" }, reg(12.0), txt_faint());
            let y = r.min.y + 90.0;
            if !b.running {
                line(ui, x, y, w, txt_faint(), &format!("Waiting for {} to start. Nothing is changed until then.", b.name));
            } else {
                let (dot, t) = match b.priority {
                    Priority::Raised => (OK, "Priority raised to Above normal".to_string()),
                    Priority::LeftAlone => (txt_faint(), "Priority was already changed, so it's left alone".to_string()),
                    Priority::Refused => (WARN_COL, format!("{} doesn't allow a priority change (anti-cheat or admin)", b.name)),
                    Priority::Waiting => (txt_faint(), "Priority: waiting".to_string()),
                };
                line(ui, x, y, w, dot, &t);
                let (dot, t) = match &b.plan {
                    Plan::Switched(was) => (OK, format!("Power plan: High performance (goes back to {was} afterwards)")),
                    Plan::AlreadyHigh => (OK, "Power plan: already High performance".to_string()),
                    Plan::Missing => (txt_faint(), "This PC has no High performance power plan, so that part is skipped".to_string()),
                    Plan::Skipped => (txt_faint(), "Power plan left alone in screenshot mode".to_string()),
                    Plan::Usual => (txt_faint(), "Power plan: switching…".to_string()),
                };
                line(ui, x, y + 26.0, w, dot, &t);
            }
            line(ui, x, y + 52.0, w, txt_faint(), "Purgeon remembers this and boosts it every time it runs");
            let br = Rect::from_min_size(pos2(x, y + 76.0), vec2(110.0, 30.0));
            if button(ui, br, "boost_stop", "Stop boost", false, true) {
                boost::stop();
            }
            para(ui, x, r.max.y - 44.0, w, foot, txt_faint());
        }
    }
}

fn free_card(ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    widgets::card(&p, r, card());
    let (x, w) = (r.min.x + 18.0, r.width() - 36.0);
    super::pages::caption_w(&p, x, r.min.y + 24.0, "Free up memory", MEM_COL, w - 120.0);
    let skip = boost::status().map(|b| b.exe);
    let sg = apps::suggestions(skip.as_deref());
    p.text(pos2(r.max.x - 18.0, r.min.y + 24.0), Align2::RIGHT_CENTER, format!("{:.0}% of RAM free", sg.avail_frac * 100.0), reg(12.0), txt_faint());

    let mut y = r.min.y + 44.0;
    let intro = if sg.avail_frac > 0.25 {
        "You have plenty of free memory, so closing apps won't speed anything up right now."
    } else {
        "Apps you haven't used for 10 minutes that hold a lot of memory. Closing one gives its memory back."
    };
    y = para(ui, x, y, w, intro, txt_dim()) + 10.0;

    let row_h = 34.0;
    let room = ((r.max.y - 40.0 - y) / row_h).floor().max(0.0) as usize;
    if sg.apps.is_empty() {
        let t = match sg.warming {
            Some(left) => format!("Nothing yet. Purgeon needs to watch for 10 minutes first ({} min to go).", (left.as_secs() + 59) / 60),
            None => "Nothing to suggest. No app over 200 MB has sat unused for 10 minutes.".to_string(),
        };
        para(ui, x, y + 4.0, w, &t, txt_faint());
    }
    for (i, a) in sg.apps.iter().take(room).enumerate() {
        let rr = Rect::from_min_size(pos2(r.min.x + 8.0, y + i as f32 * row_h), vec2(r.width() - 16.0, row_h));
        super::icons::paint(ui.ctx(), &p, Rect::from_center_size(pos2(rr.min.x + 20.0, rr.center().y), vec2(18.0, 18.0)), &a.exe, a.path.as_ref());
        let br = Rect::from_min_size(pos2(rr.max.x - 10.0 - 70.0, rr.center().y - 13.0), vec2(70.0, 26.0));
        let mins = a.idle.as_secs() / 60;
        let idle = if mins >= 120 { format!("unused {} h", mins / 60) } else { format!("unused {mins} min") };
        let tr = p.text(pos2(br.min.x - 12.0, rr.center().y), Align2::RIGHT_CENTER, format!("{}  ·  {idle}", fmt::bytes(a.mem)), reg(12.0), txt_faint());
        widgets::fit(&p, pos2(rr.min.x + 36.0, rr.center().y), Align2::LEFT_CENTER, &a.name, reg(12.5), tr.min.x - 12.0 - (rr.min.x + 36.0), txt());
        if button(ui, br, ("free_close", i), "Close…", false, !sg.busy) {
            state().confirm = Some((a.exe.clone(), a.name.clone(), apps::has_window(&a.exe)));
        }
    }
    let foot = if sg.busy {
        ("Closing…".to_string(), txt_faint())
    } else if let Some((t, ok)) = &sg.note {
        (t.clone(), if *ok { OK } else { WARN_COL })
    } else {
        ("Purgeon never closes anything without asking you first.".to_string(), txt_faint())
    };
    widgets::fit(&p, pos2(x, r.max.y - 20.0), Align2::LEFT_CENTER, &foot.0, reg(12.0), w, foot.1);
    if sg.busy {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
}

/// "Close Discord?" with Cancel and Close. Enter confirms, Esc cancels.
fn confirm(ctx: &egui::Context) {
    let Some((exe, name, windowed)) = state().confirm.clone() else { return };
    let (mut go, mut cancel) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    let screen = ctx.screen_rect();
    ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("close_dim"))).rect_filled(screen, 0.0, Color32::from_black_alpha(110));
    egui::Window::new("close_app")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([400.0, 0.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("Close {name}?")).font(sb(16.0)).color(hi()));
            ui.add_space(8.0);
            let body = if windowed {
                format!("Purgeon asks {name} to close, the same as clicking its X. If something isn't saved, it may ask you first.")
            } else {
                format!("{name} has no window open, so Purgeon ends it straight away. Anything unsaved in it is lost.")
            };
            ui.label(RichText::new(body).font(reg(12.5)).color(txt_dim()));
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let b = egui::Button::new(RichText::new("Close").font(sb(12.5)).color(Color32::WHITE)).fill(if windowed { ACCENT } else { DANGER }).min_size(vec2(80.0, 30.0));
                if ui.add(b).clicked() {
                    go = true;
                }
                if ui.add(egui::Button::new(RichText::new("Cancel").font(sb(12.5))).min_size(vec2(80.0, 30.0))).clicked() {
                    cancel = true;
                }
            });
        });
    if go {
        state().confirm = None;
        apps::close(&exe);
    } else if cancel {
        state().confirm = None;
    }
}
