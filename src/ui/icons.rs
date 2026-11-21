//! Real application icons. Paths are looked up by the sampler; the icons are pulled out of the
//! executables on a small worker thread (so the UI never waits on the shell) and uploaded as
//! tiny textures. Anything without an icon falls back to the letter avatar.
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use super::widgets;
use crate::sys::win::wide;
use eframe::egui::{self, Color32, ColorImage, Painter, Rect, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

#[repr(C)]
struct ICONINFO {
    fIcon: i32,
    xHotspot: u32,
    yHotspot: u32,
    hbmMask: isize,
    hbmColor: isize,
}

#[repr(C)]
#[derive(Default)]
struct BITMAPINFOHEADER {
    biSize: u32,
    biWidth: i32,
    biHeight: i32,
    biPlanes: u16,
    biBitCount: u16,
    biCompression: u32,
    biSizeImage: u32,
    biXPelsPerMeter: i32,
    biYPelsPerMeter: i32,
    biClrUsed: u32,
    biClrImportant: u32,
}

#[repr(C)]
struct BITMAPINFO {
    h: BITMAPINFOHEADER,
    colors: [u32; 4],
}

#[repr(C)]
struct SHFILEINFOW {
    hIcon: isize,
    iIcon: i32,
    dwAttributes: u32,
    szDisplayName: [u16; 260],
    szTypeName: [u16; 80],
}

#[link(name = "user32")]
extern "system" {
    fn PrivateExtractIconsW(file: *const u16, idx: i32, cx: i32, cy: i32, icons: *mut isize, ids: *mut u32, n: u32, flags: u32) -> u32;
    fn GetIconInfo(icon: isize, info: *mut ICONINFO) -> i32;
    fn DestroyIcon(icon: isize) -> i32;
    fn GetDC(h: isize) -> isize;
    fn ReleaseDC(h: isize, dc: isize) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn GetDIBits(dc: isize, bm: isize, start: u32, lines: u32, bits: *mut u8, bmi: *mut BITMAPINFO, usage: u32) -> i32;
    fn DeleteObject(o: isize) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn SHGetFileInfoW(path: *const u16, attrs: u32, sfi: *mut SHFILEINFOW, cb: u32, flags: u32) -> usize;
}

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(r: *mut std::ffi::c_void, model: u32) -> i32;
}

/// Reads one bitmap as top-down BGRA
unsafe fn bits(dc: isize, bm: isize, size: i32) -> Option<Vec<u8>> {
    let mut bmi = BITMAPINFO {
        h: BITMAPINFOHEADER { biSize: 40, biWidth: size, biHeight: -size, biPlanes: 1, biBitCount: 32, ..Default::default() },
        colors: [0; 4],
    };
    let mut out = vec![0u8; (size * size * 4) as usize];
    (GetDIBits(dc, bm, 0, size as u32, out.as_mut_ptr(), &mut bmi, 0) != 0).then_some(out)
}

/// HICON -> image (destroys the icon)
unsafe fn icon_image(icon: isize, size: i32) -> Option<ColorImage> {
    let mut ii: ICONINFO = std::mem::zeroed();
    if GetIconInfo(icon, &mut ii) == 0 {
        DestroyIcon(icon);
        return None;
    }
    let dc = GetDC(0);
    let color = if ii.hbmColor != 0 { bits(dc, ii.hbmColor, size) } else { None };
    let mask = bits(dc, ii.hbmMask, size);
    ReleaseDC(0, dc);
    if ii.hbmColor != 0 {
        DeleteObject(ii.hbmColor);
    }
    DeleteObject(ii.hbmMask);
    DestroyIcon(icon);
    let mut px = color?;
    // Old icons have no alpha channel: take transparency from the mask instead
    if px.chunks(4).all(|c| c[3] == 0) {
        let mask = mask?;
        for (c, m) in px.chunks_mut(4).zip(mask.chunks(4)) {
            c[3] = if m[0] == 0 { 255 } else { 0 };
        }
    }
    let rgba: Vec<u8> = px.chunks(4).flat_map(|c| [c[2], c[1], c[0], c[3]]).collect();
    Some(ColorImage::from_rgba_unmultiplied([size as usize, size as usize], &rgba))
}

fn extract(path: &str, size: i32) -> Option<ColorImage> {
    let w = wide(path);
    unsafe {
        let mut icon = 0isize;
        let mut id = 0u32;
        if PrivateExtractIconsW(w.as_ptr(), 0, size, size, &mut icon, &mut id, 1, 0) == 1 && icon != 0 {
            return icon_image(icon, size);
        }
        // No icon resource: the shell's icon for the file (the generic app icon)
        let mut sfi: SHFILEINFOW = std::mem::zeroed();
        if SHGetFileInfoW(w.as_ptr(), 0, &mut sfi, std::mem::size_of::<SHFILEINFOW>() as u32, 0x100) != 0 && sfi.hIcon != 0 {
            return icon_image(sfi.hIcon, 32).map(|i| shrink(&i, size as usize));
        }
    }
    None
}

/// Box-filter down to `size` (SHGetFileInfo only hands out 32px)
fn shrink(img: &ColorImage, size: usize) -> ColorImage {
    let s = img.size[0];
    if size >= s {
        return img.clone();
    }
    let mut out = vec![Color32::TRANSPARENT; size * size];
    for y in 0..size {
        for x in 0..size {
            let (x0, x1, y0, y1) = (x * s / size, ((x + 1) * s / size).max(x * s / size + 1), y * s / size, ((y + 1) * s / size).max(y * s / size + 1));
            let mut acc = [0u32; 4];
            let mut n = 0;
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let c = img.pixels[yy * s + xx];
                    acc[0] += c.r() as u32;
                    acc[1] += c.g() as u32;
                    acc[2] += c.b() as u32;
                    acc[3] += c.a() as u32;
                    n += 1;
                }
            }
            out[y * size + x] = Color32::from_rgba_premultiplied((acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8, (acc[3] / n) as u8);
        }
    }
    ColorImage { size: [size, size], pixels: out }
}

struct Cache {
    tex: HashMap<Arc<str>, Option<TextureHandle>>,
    pending: HashSet<Arc<str>>,
    tx: Sender<(Arc<str>, i32)>,
    rx: Receiver<(Arc<str>, Option<ColorImage>)>,
}

fn cache(ctx: &egui::Context) -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| {
        let (tx, worker_rx) = channel::<(Arc<str>, i32)>();
        let (worker_tx, rx) = channel();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("purgeon-icons".into()).spawn(move || {
            unsafe { CoInitializeEx(std::ptr::null_mut(), 2) };
            while let Ok((path, size)) = worker_rx.recv() {
                let img = extract(&path, size);
                if worker_tx.send((path, img)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Mutex::new(Cache { tex: HashMap::new(), pending: HashSet::new(), tx, rx })
    })
}

/// Drops the icons of programs that are no longer running (their textures are freed)
pub fn retain(ctx: &egui::Context, live: &HashSet<&str>) {
    if let Ok(mut c) = cache(ctx).lock() {
        c.tex.retain(|k, _| live.contains(&**k));
    }
}

/// Paints the process's icon, or its letter avatar while the icon loads / when it has none
pub fn paint(ctx: &egui::Context, p: &Painter, r: Rect, name: &str, path: Option<&Arc<str>>) {
    let Some(path) = path else {
        widgets::avatar(p, r, name);
        return;
    };
    let Ok(mut c) = cache(ctx).lock() else {
        widgets::avatar(p, r, name);
        return;
    };
    while let Ok((path, img)) = c.rx.try_recv() {
        c.pending.remove(&path);
        let tex = img.map(|img| ctx.load_texture(format!("icon:{path}"), img, TextureOptions::LINEAR));
        c.tex.insert(path, tex);
    }
    match c.tex.get(path) {
        Some(Some(t)) => {
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            p.image(t.id(), r, uv, Color32::WHITE);
        }
        Some(None) => widgets::avatar(p, r, name),
        None => {
            if !c.pending.contains(path) {
                c.pending.insert(path.clone());
                let size = (r.width() * ctx.pixels_per_point()).round().clamp(16.0, 64.0) as i32;
                let _ = c.tx.send((path.clone(), size));
            }
            widgets::avatar(p, r, name);
        }
    }
}
