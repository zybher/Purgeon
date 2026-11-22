//! Real application icons. Paths are looked up by the sampler; the icons are pulled out of the
//! executables on a small worker thread (so the UI never waits on the shell) and uploaded as
//! tiny textures. Anything without an icon falls back to the letter avatar.
//!
//! Sharpness: an icon file holds a few hand-drawn sizes (Brave: 16, 32, 48, 256). Asking Windows
//! for any other size makes it stretch one, which blurs, so Purgeon reads which sizes the file
//! has, takes the one that fits, and draws it pixel for pixel on whole screen pixels, like
//! Task Manager does.
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

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(file: *const u16, reserved: isize, flags: u32) -> isize;
    fn FreeLibrary(h: isize) -> i32;
    fn EnumResourceNamesW(h: isize, ty: *const u16, cb: unsafe extern "system" fn(isize, *const u16, *const u16, isize) -> i32, param: isize) -> i32;
    fn FindResourceW(h: isize, name: *const u16, ty: *const u16) -> isize;
    fn LoadResource(h: isize, res: isize) -> isize;
    fn LockResource(g: isize) -> *const u8;
    fn SizeofResource(h: isize, res: isize) -> u32;
}

/// RT_GROUP_ICON
const GROUP_ICON: *const u16 = 14 as *const u16;

/// Keeps the name of the first icon group (the one `PrivateExtractIconsW` index 0 reads)
unsafe extern "system" fn first_group(_: isize, _: *const u16, name: *const u16, param: isize) -> i32 {
    let out = &mut *(param as *mut Option<Vec<u16>>);
    *out = Some(if (name as usize) < 0x10000 {
        // a numbered resource: keep the number, marked by an empty string after it
        vec![name as usize as u16, 0, 0]
    } else {
        let mut v = Vec::new();
        let mut i = 0;
        while *name.add(i) != 0 {
            v.push(*name.add(i));
            i += 1;
        }
        v.push(0);
        v
    });
    0
}

/// The pixel sizes the file's main icon was drawn at, smallest first. The file is only opened
/// as data: none of its code is loaded or run.
fn native_sizes(path: &str) -> Vec<i32> {
    let w = wide(path);
    let mut out = Vec::new();
    unsafe {
        // LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE
        let h = LoadLibraryExW(w.as_ptr(), 0, 0x2 | 0x20);
        if h == 0 {
            return out;
        }
        let mut name: Option<Vec<u16>> = None;
        EnumResourceNamesW(h, GROUP_ICON, first_group, &mut name as *mut _ as isize);
        if let Some(n) = name {
            let id = if n.len() == 3 && n[1] == 0 && n[2] == 0 { n[0] as usize as *const u16 } else { n.as_ptr() };
            let res = FindResourceW(h, id, GROUP_ICON);
            let data = if res != 0 { LockResource(LoadResource(h, res)) } else { std::ptr::null() };
            if !data.is_null() {
                let d = std::slice::from_raw_parts(data, SizeofResource(h, res) as usize);
                let count = d.get(4..6).map_or(0, |c| u16::from_le_bytes([c[0], c[1]]) as usize);
                for i in 0..count {
                    if let Some(&s) = d.get(6 + 14 * i) {
                        out.push(if s == 0 { 256 } else { s as i32 });
                    }
                }
            }
        }
        FreeLibrary(h);
    }
    out.sort_unstable();
    out.dedup();
    out
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
    fn CreateCompatibleDC(dc: isize) -> isize;
    fn CreateCompatibleBitmap(dc: isize, w: i32, h: i32) -> isize;
    fn SelectObject(dc: isize, o: isize) -> isize;
    fn BitBlt(dc: isize, x: i32, y: i32, w: i32, h: i32, src: isize, sx: i32, sy: i32, rop: u32) -> i32;
    fn DeleteDC(dc: isize) -> i32;
}

/// A copy of part of the screen as it looks now (left, top, right, bottom in physical pixels),
/// for the overlay's morph into the main window. It is only held for the morph's half second.
pub fn grab_screen(r: [i32; 4]) -> Option<ColorImage> {
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    if w <= 0 || h <= 0 || w as i64 * h as i64 > 40_000_000 {
        return None;
    }
    unsafe {
        let screen = GetDC(0);
        if screen == 0 {
            return None;
        }
        let dc = CreateCompatibleDC(screen);
        let bm = CreateCompatibleBitmap(screen, w, h);
        let mut out = None;
        if dc != 0 && bm != 0 {
            let old = SelectObject(dc, bm);
            // SRCCOPY
            if BitBlt(dc, 0, 0, w, h, screen, r[0], r[1], 0x00CC_0020) != 0 {
                SelectObject(dc, old);
                let mut bmi = BITMAPINFO {
                    h: BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() },
                    colors: [0; 4],
                };
                let mut px = vec![0u8; (w * h * 4) as usize];
                if GetDIBits(dc, bm, 0, h as u32, px.as_mut_ptr(), &mut bmi, 0) != 0 {
                    for q in px.chunks_exact_mut(4) {
                        q.swap(0, 2);
                        q[3] = 255;
                    }
                    out = Some(ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px));
                }
            } else {
                SelectObject(dc, old);
            }
        }
        if bm != 0 {
            DeleteObject(bm);
        }
        if dc != 0 {
            DeleteDC(dc);
        }
        ReleaseDC(0, screen);
        out
    }
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

/// The icon for a spot `want` pixels wide. Comes back at a size the file really has when one
/// fits (up to a fifth smaller, drawn 1:1 and centred), else a bigger one scaled down here.
fn extract(path: &str, want: i32) -> Option<ColorImage> {
    let w = wide(path);
    let natives = native_sizes(path);
    let fits = natives.iter().rev().find(|&&s| s <= want && s * 5 >= want * 4).copied();
    let bigger = natives.iter().find(|&&s| s > want).copied();
    let (size, scale_to) = match (fits, bigger, natives.last()) {
        (Some(s), _, _) => (s, None),
        (None, Some(b), _) => (b, Some(want)),
        // every size it has is much smaller: show the biggest as it is
        (None, None, Some(&l)) => (l, None),
        (None, None, None) => (want, None),
    };
    unsafe {
        let mut icon = 0isize;
        let mut id = 0u32;
        if PrivateExtractIconsW(w.as_ptr(), 0, size, size, &mut icon, &mut id, 1, 0) == 1 && icon != 0 {
            let img = icon_image(icon, size)?;
            return Some(match scale_to {
                Some(n) => shrink(&img, n as usize),
                None => img,
            });
        }
        // No icon resource: the shell's icon for the file (the generic app icon)
        let mut sfi: SHFILEINFOW = std::mem::zeroed();
        if SHGetFileInfoW(w.as_ptr(), 0, &mut sfi, std::mem::size_of::<SHFILEINFOW>() as u32, 0x100) != 0 && sfi.hIcon != 0 {
            return icon_image(sfi.hIcon, 32).map(|i| shrink(&i, want as usize));
        }
    }
    None
}

/// Where the file of a process Windows won't name probably is. Windows keeps the path of its own
/// protected processes (svchost, dwm, WmiPrvSE, Defender's MsMpEng) from programs that aren't
/// elevated, and the kernel's own entries (System, Registry, Memory Compression) have no file
/// at all. Their files sit in known places, and reading an icon out of a file needs no access
/// to the process. The kernel entries get the kernel's file, which shows the plain Windows
/// program icon, as Task Manager does.
fn guess(name: &str) -> Option<Arc<str>> {
    static MEMO: OnceLock<Mutex<HashMap<String, Option<Arc<str>>>>> = OnceLock::new();
    let key = name.to_ascii_lowercase();
    let mut m = MEMO.get_or_init(Default::default).lock().ok()?;
    if let Some(v) = m.get(&key) {
        return v.clone();
    }
    let win = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let sys = format!("{win}\\System32");
    let mut tries: Vec<String> = Vec::new();
    if key.ends_with(".exe") {
        tries.push(format!("{sys}\\{name}"));
        tries.push(format!("{sys}\\wbem\\{name}"));
        tries.push(format!("{win}\\{name}"));
        // Defender's own folder: the newest platform version holds the running one
        let pd = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".into());
        let plat = format!("{pd}\\Microsoft\\Windows Defender\\Platform");
        if let Ok(rd) = std::fs::read_dir(&plat) {
            let mut v: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            v.sort();
            if let Some(last) = v.last() {
                tries.push(format!("{plat}\\{last}\\{name}"));
            }
        }
    } else if matches!(key.as_str(), "system" | "registry" | "memory compression" | "secure system" | "system idle process" | "idle" | "interrupts") {
        tries.push(format!("{sys}\\ntoskrnl.exe"));
    }
    let found = tries.into_iter().find(|t| std::path::Path::new(t).is_file()).map(|t| Arc::from(t.as_str()));
    m.insert(key, found.clone());
    found
}

/// Scales down to `size`, each new pixel the exact average of the area it covers
fn shrink(img: &ColorImage, size: usize) -> ColorImage {
    let s = img.size[0];
    if size >= s || size == 0 {
        return img.clone();
    }
    // how much of source pixel `i` falls inside output pixel `o`, along one axis
    let k = s as f32 / size as f32;
    let span = |o: usize| -> Vec<(usize, f32)> {
        let (a, b) = (o as f32 * k, (o + 1) as f32 * k);
        (a.floor() as usize..(b.ceil() as usize).min(s)).map(|i| (i, (b.min(i as f32 + 1.0) - a.max(i as f32)).max(0.0))).collect()
    };
    let spans: Vec<_> = (0..size).map(span).collect();
    let mut out = vec![Color32::TRANSPARENT; size * size];
    for (y, sy) in spans.iter().enumerate() {
        for (x, sx) in spans.iter().enumerate() {
            let mut acc = [0f32; 4];
            for &(yy, wy) in sy {
                for &(xx, wx) in sx {
                    let c = img.pixels[yy * s + xx];
                    let wgt = wx * wy;
                    acc[0] += c.r() as f32 * wgt;
                    acc[1] += c.g() as f32 * wgt;
                    acc[2] += c.b() as f32 * wgt;
                    acc[3] += c.a() as f32 * wgt;
                }
            }
            let n = k * k;
            let q = |v: f32| (v / n).round().clamp(0.0, 255.0) as u8;
            out[y * size + x] = Color32::from_rgba_premultiplied(q(acc[0]), q(acc[1]), q(acc[2]), q(acc[3]));
        }
    }
    ColorImage { size: [size, size], pixels: out }
}

/// Each icon is pulled out for the pixel size of the spot it is drawn in (one texture per
/// size), so a big icon (the overlay's, more so when it is zoomed) is never a small one stretched
type Key = (Arc<str>, i32);

/// Where a `px`-pixel texture goes to show pixel for pixel: centred in `r`, its corner on a
/// whole screen pixel (half a pixel off would smear every edge)
fn on_pixels(ppp: f32, r: Rect, px: usize) -> Rect {
    let s = px as f32;
    let c = r.center();
    let min = egui::pos2((c.x * ppp - s / 2.0).round() / ppp, (c.y * ppp - s / 2.0).round() / ppp);
    Rect::from_min_size(min, egui::vec2(s / ppp, s / ppp))
}

struct Cache {
    tex: HashMap<Key, Option<TextureHandle>>,
    pending: HashSet<Key>,
    tx: Sender<Key>,
    rx: Receiver<(Key, Option<ColorImage>)>,
}

fn cache(ctx: &egui::Context) -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| {
        let (tx, worker_rx) = channel::<Key>();
        let (worker_tx, rx) = channel();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("purgeon-icons".into()).spawn(move || {
            unsafe { CoInitializeEx(std::ptr::null_mut(), 2) };
            while let Ok((path, size)) = worker_rx.recv() {
                let img = extract(&path, size);
                if worker_tx.send(((path, size), img)).is_err() {
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
        c.tex.retain(|k, _| live.contains(&*k.0));
    }
}

/// Paints the process's icon, or its letter avatar while the icon loads / when it has none
pub fn paint(ctx: &egui::Context, p: &Painter, r: Rect, name: &str, path: Option<&Arc<str>>) {
    let guessed;
    let path = match path {
        Some(p) => Some(p),
        None => {
            guessed = guess(name);
            guessed.as_ref()
        }
    };
    let Some(path) = path else {
        widgets::avatar(p, r, name);
        return;
    };
    let Ok(mut c) = cache(ctx).lock() else {
        widgets::avatar(p, r, name);
        return;
    };
    while let Ok((key, img)) = c.rx.try_recv() {
        c.pending.remove(&key);
        let tex = img.map(|img| ctx.load_texture(format!("icon:{}:{}", key.1, key.0), img, TextureOptions::LINEAR));
        c.tex.insert(key, tex);
    }
    let ppp = ctx.pixels_per_point();
    let size = ((r.width() * ppp).round() as i32).clamp(8, 256);
    let key: Key = (path.clone(), size);
    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let draw = |t: &TextureHandle| p.image(t.id(), on_pixels(ppp, r, t.size()[0]), uv, Color32::WHITE);
    // while the right size loads, any size already there beats the letter
    let loaded = |c: &Cache| c.tex.iter().find(|(k, t)| k.0 == *path && t.is_some()).and_then(|(_, t)| t.clone());
    match c.tex.get(&key) {
        Some(Some(t)) => {
            draw(t);
        }
        Some(None) => widgets::avatar(p, r, name),
        None => {
            if !c.pending.contains(&key) {
                c.pending.insert(key.clone());
                let _ = c.tx.send(key);
            }
            match loaded(&c) {
                Some(t) => {
                    draw(&t);
                }
                None => widgets::avatar(p, r, name),
            }
        }
    }
}
