//! Embeds assets/purgeon.ico as the exe's icon (what Explorer, the taskbar pin and Alt+Tab show
//! before the window sets its own). Writes the compiled resource (.res) by hand and passes it to
//! the MSVC linker, which accepts .res files directly, so no resource compiler or crate is needed.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=assets/purgeon.ico");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !(windows && msvc) {
        return;
    }
    let Ok(ico) = std::fs::read("assets/purgeon.ico") else { return };
    let Some(res) = res_from_ico(&ico) else {
        println!("cargo:warning=assets/purgeon.ico could not be read; the exe gets no icon");
        return;
    };
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("purgeon.res");
    std::fs::write(&out, res).unwrap();
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const LANG_EN_US: u16 = 0x0409;

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// One resource with a numeric type and id: 32-byte header, then the data padded to 4 bytes
fn entry(out: &mut Vec<u8>, ty: u16, id: u16, flags: u16, data: &[u8]) {
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(32u32.to_le_bytes());
    out.extend([0xFF, 0xFF]);
    out.extend(ty.to_le_bytes());
    out.extend([0xFF, 0xFF]);
    out.extend(id.to_le_bytes());
    out.extend(0u32.to_le_bytes()); // data version
    out.extend(flags.to_le_bytes());
    out.extend(LANG_EN_US.to_le_bytes());
    out.extend(0u32.to_le_bytes()); // version
    out.extend(0u32.to_le_bytes()); // characteristics
    out.extend(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

fn res_from_ico(ico: &[u8]) -> Option<Vec<u8>> {
    if u16_at(ico, 0)? != 0 || u16_at(ico, 2)? != 1 {
        return None;
    }
    let n = u16_at(ico, 4)? as usize;
    // a .res file starts with an empty entry
    let mut out = vec![0, 0, 0, 0, 0x20, 0, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0];
    out.extend([0u8; 16]);
    let mut group = Vec::new();
    group.extend(0u16.to_le_bytes());
    group.extend(1u16.to_le_bytes());
    group.extend((n as u16).to_le_bytes());
    for i in 0..n {
        let e = 6 + i * 16;
        let dir = ico.get(e..e + 16)?;
        let size = u32_at(ico, e + 8)? as usize;
        let off = u32_at(ico, e + 12)? as usize;
        let data = ico.get(off..off + size)?;
        let id = (i + 1) as u16;
        entry(&mut out, RT_ICON, id, 0x1010, data);
        // GRPICONDIRENTRY: the ICONDIRENTRY's first 12 bytes, then the resource id
        group.extend(&dir[..12]);
        group.extend(id.to_le_bytes());
    }
    entry(&mut out, RT_GROUP_ICON, 1, 0x1030, &group);
    Some(out)
}
