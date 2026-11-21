//! One palette for the whole UI (the Graphite look shared with Tangra).

use eframe::egui::{Color32, FontFamily, FontId};

/// Window chrome: title bar and sidebar (darkest)
pub fn chrome() -> Color32 {
    Color32::from_gray(0x0f)
}
/// The content sheet behind the pages
pub fn bg() -> Color32 {
    Color32::from_gray(0x14)
}
/// Cards
pub fn card() -> Color32 {
    Color32::from_gray(0x1b)
}
/// A card under the pointer
pub fn card_hi() -> Color32 {
    Color32::from_gray(0x21)
}
/// Inputs, tracks and other recessed surfaces
pub fn well() -> Color32 {
    Color32::from_gray(0x11)
}
pub fn border() -> Color32 {
    Color32::from_gray(0x2b)
}
pub fn txt() -> Color32 {
    Color32::from_gray(236)
}
pub fn txt_dim() -> Color32 {
    Color32::from_gray(165)
}
pub fn txt_faint() -> Color32 {
    Color32::from_gray(115)
}
pub fn hi() -> Color32 {
    Color32::WHITE
}
/// A white wash (hover fills, tracks)
pub fn wa(a: u8) -> Color32 {
    Color32::from_white_alpha(a)
}

pub const ACCENT: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);
pub const CPU_COL: Color32 = ACCENT;
pub const KERNEL_COL: Color32 = Color32::from_rgb(0x93, 0xc5, 0xfd);
pub const MEM_COL: Color32 = Color32::from_rgb(0x8b, 0x5c, 0xf6);
pub const COMMIT_COL: Color32 = Color32::from_rgb(0xc4, 0xb5, 0xfd);
pub const GPU_COL: Color32 = Color32::from_rgb(0x22, 0xc5, 0x5e);
pub const VRAM_COL: Color32 = Color32::from_rgb(0x86, 0xef, 0xac);
pub const DISK_COL: Color32 = Color32::from_rgb(0x2d, 0xd4, 0xbf);
pub const IO_COL: Color32 = Color32::from_rgb(0x14, 0xb8, 0xa6);
/// Network download, and upload (lighter)
pub const NET_COL: Color32 = Color32::from_rgb(0xec, 0x48, 0x99);
pub const NET_UP_COL: Color32 = Color32::from_rgb(0xf9, 0xa8, 0xd4);
pub const WARN_COL: Color32 = Color32::from_rgb(0xf9, 0x73, 0x16);
pub const DANGER: Color32 = Color32::from_rgb(232, 17, 35);

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// Semibold UI font (labels that must stay readable, big numbers)
pub fn sb(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("sb".into()))
}
pub fn reg(size: f32) -> FontId {
    FontId::proportional(size)
}
