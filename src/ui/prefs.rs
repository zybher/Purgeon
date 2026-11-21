//! The Settings page: the overlay (size, shortcut) and starting with Windows.

use super::overlay;
use super::tools::button;
use super::widgets;
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Event, Key, Rect, ScrollArea, Sense, Stroke, Ui};
use std::sync::Mutex;

struct State {
    /// Waiting for the new shortcut to be pressed
    capturing: bool,
    /// What happened last, shown under the shortcut
    note: Option<(String, Color32)>,
}
static STATE: Mutex<State> = Mutex::new(State { capturing: false, note: None });

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

const OK: Color32 = GPU_COL;

/// Called when another page is opened: a shortcut half typed is dropped and the old one works again
pub fn leave() {
    let mut st = state();
    if st.capturing {
        st.capturing = false;
        crate::tray::pause_overlay_key(false);
    }
    st.note = None;
}

/// A setting's name and one line on what it does, on the left of a row
fn label(p: &egui::Painter, x: f32, y: f32, title: &str, sub: &str, w: f32) {
    p.text(pos2(x, y - 9.0), Align2::LEFT_CENTER, title, sb(13.5), txt());
    widgets::fit(p, pos2(x, y + 10.0), Align2::LEFT_CENTER, sub, reg(12.0), w, txt_faint());
}

/// The combination pressed, as settings.ini writes it ("Ctrl+Alt+O"), or why it can't be used
fn combo(key: Key, m: egui::Modifiers) -> Result<String, &'static str> {
    let name = key.name();
    let fkey = name.len() >= 2 && name.starts_with('F') && name[1..].parse::<u8>().is_ok();
    let plain = name.len() == 1 && name.chars().all(|c| c.is_ascii_alphanumeric());
    if !fkey && !plain {
        return Err("Use a letter, a number or an F key, together with Ctrl, Alt or Shift");
    }
    let mut parts = Vec::new();
    if m.ctrl {
        parts.push("Ctrl");
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Shift");
    }
    // one modifier and a letter (Ctrl+S, Shift+A) would take that key away from every other app
    if parts.is_empty() || (parts.len() < 2 && !fkey) {
        return Err("Hold two of Ctrl, Alt and Shift (or one with an F key), so other apps keep their shortcuts");
    }
    if (m.alt || m.ctrl) && parts.len() == 1 && name == "F4" {
        return Err("That one closes windows and tabs, so pick another");
    }
    parts.push(name);
    Ok(parts.join("+"))
}

pub fn show(ui: &mut Ui) {
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        // the page reaches the window's edge so the scroll bar sits there; the content keeps its margin
        ui.set_max_width(ui.available_width() - super::PAGE_PAD);
        let w = ui.available_width();
        {
            let (r, _) = ui.allocate_exact_size(vec2(w, 58.0), Sense::hover());
            let p = ui.painter();
            p.text(pos2(r.min.x, r.min.y + 14.0), Align2::LEFT_CENTER, "Settings", sb(24.0), txt());
            p.text(pos2(r.min.x, r.min.y + 40.0), Align2::LEFT_CENTER, "Saved as you change them", reg(12.5), txt_faint());
        }
        overlay_card(ui, w);
        ui.add_space(16.0);
        startup_card(ui, w);
    });
}

fn startup_card(ui: &mut Ui, w: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(w, 112.0), Sense::hover());
    let p = ui.painter().clone();
    widgets::card(&p, r, card());
    let (x, iw) = (r.min.x + 18.0, r.width() - 36.0);
    super::pages::caption_w(&p, x, r.min.y + 24.0, "Startup", ACCENT, iw);
    let y = r.min.y + 72.0;
    label(&p, x, y, "Start with Windows", "Waits in the tray from sign-in, so the graphs already have history when you open Purgeon", iw - 70.0);
    // read once and kept, not from the registry every frame
    let id = egui::Id::new("pref_autostart");
    let mut on = ui.ctx().data_mut(|d| *d.get_temp_mut_or_insert_with(id, crate::autostart::is_on));
    let sr = Rect::from_min_size(pos2(r.max.x - 18.0 - 44.0, y - 13.0), vec2(44.0, 26.0));
    if widgets::switch(ui, sr, &mut on).clicked() {
        if !crate::autostart::set(on) {
            on = crate::autostart::is_on();
        }
        ui.ctx().data_mut(|d| d.insert_temp(id, on));
    }
}

fn overlay_card(ui: &mut Ui, w: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(w, 236.0), Sense::hover());
    let p = ui.painter().clone();
    widgets::card(&p, r, card());
    let (x, iw) = (r.min.x + 18.0, r.width() - 36.0);
    super::pages::caption_w(&p, x, r.min.y + 24.0, "Overlay", ACCENT, iw - 140.0);
    let ctl_w = 300.0_f32.min(iw * 0.5);
    let cx = r.max.x - 18.0 - ctl_w;
    let lw = cx - 16.0 - x;
    let open = overlay::is_open();
    let pr = Rect::from_min_size(pos2(r.max.x - 18.0 - 120.0, r.min.y + 10.0), vec2(120.0, 28.0));
    if button(ui, pr, "pref_preview", if open { "Hide the bar" } else { "Show the bar" }, false, true) {
        overlay::toggle(ui.ctx());
    }

    // ---- size ----
    let y = r.min.y + 76.0;
    label(&p, x, y, "Size", "How big the bar at the top of the screen is", lw);
    let mut pick = overlay::size_pick();
    let sr = Rect::from_min_size(pos2(cx, y - 14.0 - 6.0), vec2(ctl_w, 30.0));
    if widgets::segmented(ui, sr, &overlay::SIZES, &mut pick, egui::Id::new("pref_size")) {
        overlay::set_size_pick(pick);
    }
    let hint = if pick == 0 {
        format!("Auto uses {:.0}% on this screen, more on very wide ones", overlay::auto_here() * 100.0)
    } else {
        "The same on every screen".to_string()
    };
    widgets::fit(&p, pos2(cx + 2.0, sr.max.y + 12.0), Align2::LEFT_CENTER, &hint, reg(11.5), ctl_w, txt_faint());

    p.line_segment([pos2(x, r.min.y + 132.0), pos2(r.max.x - 18.0, r.min.y + 132.0)], Stroke::new(1.0_f32, border()));

    // ---- shortcut ----
    let y = r.min.y + 172.0;
    label(&p, x, y, "Shortcut", "Opens and closes the bar from anywhere, even in a game", lw);
    let key = crate::tray::overlay_key();
    let mut st = state();
    let kr = Rect::from_min_size(pos2(cx, y - 20.0), vec2(ctl_w - 96.0, 32.0));
    let resp = ui.interact(kr, egui::Id::new("pref_key"), Sense::click());
    let (h, _) = crate::anim::hover_fx(ui.ctx(), &resp);
    let fill = if st.capturing { well() } else { mix(well(), card_hi(), h) };
    p.rect_filled(kr, 8.0, fill);
    p.rect_stroke(kr, 8.0, Stroke::new(if st.capturing { 1.5_f32 } else { 1.0 }, if st.capturing { ACCENT } else { border() }));
    if st.capturing {
        p.text(kr.center(), Align2::CENTER_CENTER, "Press the new keys…", reg(12.5), txt_dim());
    } else {
        widgets::fit(&p, kr.center(), Align2::CENTER_CENTER, &key, sb(13.0), kr.width() - 16.0, txt());
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let br = Rect::from_min_size(pos2(kr.max.x + 8.0, kr.min.y + 1.0), vec2(88.0, 30.0));
    let is_default = key.eq_ignore_ascii_case(crate::tray::DEFAULT_KEY);
    let mut reset = false;
    if st.capturing {
        if button(ui, br, "pref_key_cancel", "Cancel", false, true) {
            st.capturing = false;
            st.note = None;
            crate::tray::pause_overlay_key(false);
        }
    } else if button(ui, br, "pref_key_reset", "Reset", false, !is_default) {
        reset = true;
    }

    if resp.clicked() && !st.capturing {
        st.capturing = true;
        st.note = None;
        crate::tray::pause_overlay_key(true);
    } else if st.capturing {
        let (evs, focused) = ui.input(|i| (i.events.clone(), i.focused));
        // clicking somewhere else, or switching away, cancels
        let pos = ui.input(|i| i.pointer.interact_pos());
        let away = !focused || (ui.input(|i| i.pointer.any_pressed()) && pos.is_some_and(|q| !kr.contains(q) && !br.contains(q)));
        if away {
            st.capturing = false;
            crate::tray::pause_overlay_key(false);
        }
        for e in evs {
            if !st.capturing {
                break;
            }
            let Event::Key { key: k, pressed: true, modifiers, .. } = e else { continue };
            if k == Key::Escape {
                st.capturing = false;
                st.note = None;
                crate::tray::pause_overlay_key(false);
                break;
            }
            match combo(k, modifiers) {
                Err(why) => st.note = Some((why.to_string(), WARN_COL)),
                Ok(c) if !crate::tray::valid_key(&c) => st.note = Some(("That key can't be used, pick another".to_string(), WARN_COL)),
                Ok(c) => {
                    st.capturing = false;
                    st.note = Some(if crate::tray::set_overlay_key(&c) {
                        (format!("Saved. {c} now opens the overlay"), OK)
                    } else {
                        (format!("Another app already uses {c}, so the shortcut stays {key}"), WARN_COL)
                    });
                }
            }
        }
        ui.ctx().request_repaint();
    }
    if reset {
        st.note = Some(if crate::tray::set_overlay_key(crate::tray::DEFAULT_KEY) {
            (format!("Back to {}", crate::tray::DEFAULT_KEY), OK)
        } else {
            (format!("Another app already uses {}, so the shortcut stays {key}", crate::tray::DEFAULT_KEY), WARN_COL)
        });
    }
    let (note, col) = match &st.note {
        Some((t, c)) => (t.clone(), *c),
        None if st.capturing => ("Hold Ctrl, Alt or Shift and press a key. Esc cancels".to_string(), txt_faint()),
        None => ("Click it, then press the keys you want".to_string(), txt_faint()),
    };
    widgets::fit(&p, pos2(cx + 2.0, kr.max.y + 13.0), Align2::LEFT_CENTER, &note, reg(11.5), r.max.x - 18.0 - cx, col);
}
