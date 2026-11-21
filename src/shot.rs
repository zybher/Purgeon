//! Dev helper: `PURGEON_SHOT=out.png` opens the window off-screen and inactive, saves a
//! screenshot after a few seconds and quits (for checking the UI without disturbing the desktop).

use eframe::egui::{self, ColorImage, ViewportCommand};
use std::time::Instant;

pub struct Shot {
    path: String,
    start: Instant,
    asked: bool,
}

impl Shot {
    pub fn from_env() -> Option<Shot> {
        std::env::var("PURGEON_SHOT").ok().map(|path| Shot { path, start: Instant::now(), asked: false })
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        // no repaint request: new samples wake the UI every second, which is enough to time the shot
        if !self.asked && self.start.elapsed().as_secs_f32() > delay() {
            self.asked = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot);
        }
        let img = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = img {
            let _ = std::fs::write(&self.path, png(&img));
            crate::quit();
        }
    }
}

/// Seconds before the capture (`PURGEON_SHOT_SECS`, default 4.5)
fn delay() -> f32 {
    std::env::var("PURGEON_SHOT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(4.5)
}

/// Minimal PNG writer (stored deflate blocks)
fn png(img: &ColorImage) -> Vec<u8> {
    let (w, h) = (img.size[0] as u32, img.size[1] as u32);
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for y in 0..h as usize {
        raw.push(0);
        for px in &img.pixels[y * w as usize..(y + 1) * w as usize] {
            raw.extend_from_slice(&px.to_srgba_unmultiplied());
        }
    }
    let mut z = vec![0x78, 0x01];
    for (i, chunk) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        let n = chunk.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(chunk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    let mut c = 0xFFFF_FFFFu32;
    for &x in &out[start..] {
        c ^= x as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    out.extend_from_slice(&(!c).to_be_bytes());
}
