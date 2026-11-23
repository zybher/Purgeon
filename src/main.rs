//! Purgeon: a detailed Windows system monitor.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alerts;
mod anim;
mod autostart;
mod apps;
mod boost;
mod fmt;
mod games;
mod platform;
mod settings;
mod shot;
mod steady;
mod sys;
mod theme;
mod tray;
mod trial;
mod ui;

use eframe::egui;

pub const TITLE: &str = "Purgeon";

/// Exits Purgeon for good: from the tray menu, after relaunching elevated, or after a screenshot
pub fn quit() -> ! {
    tray::remove();
    // the boosted app's priority and the power plan go back to how they were
    boost::on_quit();
    // kernel trace sessions outlive the process unless stopped
    sys::etw::stop();
    std::process::exit(0)
}

fn main() -> eframe::Result<()> {
    #[cfg(feature = "bench")]
    if let Some(i) = std::env::args().position(|a| a == "--bench") {
        let secs = std::env::args().nth(i + 1).and_then(|s| s.parse().ok()).unwrap_or(30);
        sys::bench::run(secs);
        return Ok(());
    }
    #[cfg(feature = "bench")]
    if std::env::args().any(|a| a == "--memprobe") {
        sys::bench::memprobe();
        return Ok(());
    }
    #[cfg(feature = "bench")]
    if std::env::args().any(|a| a == "--bench-mem") {
        sys::bench::run_mem();
        return Ok(());
    }
    #[cfg(feature = "bench")]
    if let Some(i) = std::env::args().position(|a| a == "--load") {
        sys::bench::load(&std::env::args().skip(i + 1).collect::<Vec<_>>());
        return Ok(());
    }
    // Purgeon already running (perhaps hidden in the tray): bring that one back instead
    if std::env::var_os("PURGEON_SHOT").is_none() && !tray::first_instance() {
        return Ok(());
    }
    alerts::load();
    if std::env::var_os("PURGEON_SHOT").is_none() {
        autostart::refresh();
    }
    boost::load();
    egui::epaint::text::set_subpixel(platform::subpixel_order());
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(TITLE)
        .with_inner_size(shot::size().unwrap_or([1180.0, 760.0]))
        .with_min_inner_size([860.0, 540.0])
        .with_decorations(false)
        .with_resizable(true)
        .with_icon(platform::app_icon());
    if std::env::var_os("PURGEON_SHOT").is_some() {
        viewport = viewport.with_position([-20000.0, -20000.0]).with_active(false);
    } else if autostart::started_in_tray() {
        // opens off-screen and goes to the tray on its first frame (see `platform::hide_window`)
        viewport = viewport.with_position([-32000.0, -32000.0]).with_active(false);
    }
    let options = eframe::NativeOptions {
        viewport,
        // centring replaces any position given above, which put screenshot windows on the desktop
        centered: std::env::var_os("PURGEON_SHOT").is_none(),
        ..Default::default()
    };
    eframe::run_native(TITLE, options, Box::new(|cc| Ok(Box::new(ui::App::new(cc)))))
}
