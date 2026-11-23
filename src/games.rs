//! Finds the games installed on this PC, for the Boost row. Windows has no "this is a game"
//! flag, so this puts together what does exist:
//! - the launchers' own records: Steam, Epic, GOG, Riot, Ubisoft, EA, Xbox, and the install list
//!   for publishers that only make games
//! - Roblox and Minecraft, which keep their own
//! - installed, open and Game Bar-remembered programs, only when their folder holds what games
//!   ship with: an engine (Unity, Unreal, Godot, GameMaker, Ren'Py) or a store's game SDK. Game
//!   Bar's list alone is no proof, it also remembers music players and editors.
//!
//! It only reads files and the registry, on a thread of its own, and only while the row is shown.

use crate::sys::{win, Proc};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Which processes are a game: those in its folders or, when it has none, those with its exe name
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Target {
    /// Lower case
    pub exe: String,
    pub dirs: Vec<String>,
}

/// Runtimes many unrelated programs share, so their name alone never identifies a game
const SHARED: &[&str] = &["java.exe", "javaw.exe", "dotnet.exe", "python.exe", "pythonw.exe", "node.exe", "nw.exe", "electron.exe"];

fn shared(exe: &str) -> bool {
    SHARED.iter().any(|r| r.eq_ignore_ascii_case(exe))
}

/// Not the game itself: installers, crash reporters, launchers and, above all, anti-cheat,
/// which must never be touched
fn helper(exe: &str) -> bool {
    let e = exe.to_ascii_lowercase();
    let stem = e.trim_end_matches(".exe");
    const PARTS: &[&str] = &[
        "anticheat", "anti-cheat", "anti_cheat", "battleye", "beservice", "faceit", "gameguard", "unins", "crash", "redist", "setup", "install",
        "prereq", "dotnetfx", "helper", "subprocess", "cefprocess", "reporter", "updater", "launcher", "bootstrap", "touchup", "busybox", "quickedit",
    ];
    PARTS.iter().any(|p| stem.contains(p))
        || ["_be", "_eac", "server"].iter().any(|p| stem.ends_with(p))
        || matches!(stem, "vgc" | "vgtray" | "vgk" | "gamemon" | "gamemon64")
}

fn inside(path: &str, dir: &str) -> bool {
    path.get(..dir.len()).is_some_and(|head| head.eq_ignore_ascii_case(dir)) && path.as_bytes().get(dir.len()) == Some(&b'\\')
}

impl Target {
    pub fn matches(&self, p: &Proc) -> bool {
        self.has(&p.name, p.path.as_deref())
    }

    pub fn has(&self, name: &str, path: Option<&str>) -> bool {
        let named = name.eq_ignore_ascii_case(&self.exe);
        if self.dirs.is_empty() {
            return named;
        }
        match path {
            Some(path) => self.dirs.iter().any(|d| inside(path, d)) && !helper(name),
            // a game that hides its path still has its name
            None => named && !shared(name),
        }
    }

    /// The game and all that comes with it, helpers and anti-cheat included: what is left alone
    pub fn covers(&self, path: &str) -> bool {
        if self.dirs.is_empty() {
            return file_name(path).eq_ignore_ascii_case(&self.exe);
        }
        self.dirs.iter().any(|d| inside(path, d))
    }
}

#[derive(Clone)]
pub struct Game {
    pub name: String,
    pub target: Target,
    pub icon: Option<Arc<str>>,
}

struct State {
    list: Option<Arc<Vec<Game>>>,
    scanned: Option<Instant>,
    busy: bool,
    /// The open programs the last scan looked at
    seen: HashSet<Arc<str>>,
}
static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| State { list: None, scanned: None, busy: false, seen: HashSet::new() }))
}

/// The games as last found (None until the first look is done). Looks again every five minutes,
/// or sooner when a program opens that it hasn't seen; `open` is the programs with a window.
pub fn list(open: &[Arc<str>], done: impl FnOnce() + Send + 'static) -> Option<Arc<Vec<Game>>> {
    with(|st| {
        let due = st.scanned.map_or(true, |t| {
            let age = t.elapsed();
            age > Duration::from_secs(300) || (age > Duration::from_secs(20) && open.iter().any(|p| !st.seen.contains(p)))
        });
        if due && !st.busy {
            st.busy = true;
            st.scanned = Some(Instant::now());
            st.seen.extend(open.iter().cloned());
            let open = open.to_vec();
            let spawned = std::thread::Builder::new().name("purgeon-games".into()).spawn(move || {
                let found = Arc::new(scan(&open));
                with(|st| {
                    st.list = Some(found);
                    st.busy = false;
                });
                done();
            });
            if spawned.is_err() {
                st.busy = false;
            }
        }
        st.list.clone()
    })
}

/// The games as last found, without looking
pub fn known() -> Option<Arc<Vec<Game>>> {
    with(|st| st.list.clone())
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn clean(path: &str) -> String {
    path.trim().trim_matches('"').replace('/', "\\").trim_end_matches('\\').to_string()
}

fn file_name(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}

fn parent(path: &str) -> &str {
    path.rsplit_once('\\').map_or("", |p| p.0)
}

/// A folder narrow enough that everything in it belongs to one game. Boost raises what runs in
/// it, so "C:\Program Files" or a whole Steam library must never pass.
pub fn safe_dir(dir: &str) -> bool {
    let d = dir.trim_end_matches('\\').to_lowercase();
    if d.len() < 4 || d.as_bytes()[1] != b':' || d.as_bytes()[2] != b'\\' || d.split('\\').any(|c| c == "." || c == "..") {
        return false;
    }
    const BROAD: &[&str] = &[
        "games", "game library", "my games", "steam", "steamapps", "common", "steamlibrary", "epic games", "riot games", "xboxgames", "gog games",
        "gog galaxy", "ubisoft", "ea games", "origin games", "rockstar games", "desktop", "downloads", "documents", "programs",
    ];
    if BROAD.contains(&file_name(&d)) {
        return false;
    }
    let roots = ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData", "USERPROFILE", "APPDATA", "LOCALAPPDATA"];
    for (key, root) in roots.iter().filter_map(|k| env(k).map(|v| (*k, v.to_lowercase()))) {
        if d == root || inside(&root, &d) || (key == "SystemRoot" && inside(&d, &root)) {
            return false;
        }
    }
    true
}

/// Names of tools that arrive the way games do
fn not_a_game(name: &str) -> bool {
    let n = name.to_lowercase();
    const WORDS: &[&str] = &[
        "redistributable", "steamworks", "steam linux runtime", "steamvr", "proton ", "soundtrack", "dedicated server", "anticheat", "anti-cheat",
        "battleye", "vanguard", "wallpaper engine", "lossless scaling", "crosshair x", "soundpad", "vtube studio", "obs studio", "3dmark",
        "benchmark", "aseprite", "rpg maker", "unreal engine", "unreal editor", "unity hub", "unity editor", "launcher", "bootstrapper", "roblox studio",
        "robloxstudio", "riot client", "ubisoft connect", "social club", "ea app", "gog galaxy", "epic online services", " sdk", "modding tool",
    ];
    WORDS.iter().any(|w| n.contains(w)) || matches!(n.as_str(), "battle.net" | "steam" | "origin" | "blender" | "krita" | "hoyoplay")
}

/// What most games ship with
fn ships_like_a_game(dir: &str) -> bool {
    // driver suites carry game SDKs too (NVIDIA's streamer ships Steam's)
    let low = format!("{}\\", dir.to_lowercase());
    if ["nvidia corporation", "nvidia", "amd", "intel", "common files"].iter().any(|v| low.contains(&format!("\\{v}\\"))) {
        return false;
    }
    let d = Path::new(dir);
    const FILES: &[&str] = &[
        "UnityPlayer.dll", "steam_api64.dll", "steam_api.dll", "EOSSDK-Win64-Shipping.dll", "Galaxy64.dll", "data.win", "renpy", "Engine\\Binaries\\Win64",
    ];
    if FILES.iter().any(|f| d.join(f).exists()) {
        return true;
    }
    // Unreal's game exe, and Godot's pack beside an exe of the same name
    std::fs::read_dir(d).into_iter().flatten().flatten().take(500).any(|e| {
        let n = e.file_name().to_string_lossy().to_lowercase();
        n.ends_with("-win64-shipping.exe") || n.strip_suffix(".pck").is_some_and(|stem| d.join(format!("{stem}.exe")).is_file())
    })
}

/// The exe most likely to be the game: named like it, else the biggest that is no helper
fn main_exe(dir: &str, name: &str) -> Option<String> {
    let plain = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let want = plain(name);
    let mut best: Option<((bool, u64), String)> = None;
    let mut todo = vec![(dir.to_string(), 0)];
    let mut looked = 0;
    while let Some((d, depth)) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            looked += 1;
            if looked > 3000 {
                break;
            }
            let Ok(kind) = e.file_type() else { continue };
            let file = e.file_name().to_string_lossy().into_owned();
            let path = format!("{d}\\{file}");
            if kind.is_dir() && depth < 3 {
                // the root's files come first, whatever the folder holds
                todo.insert(0, (path, depth + 1));
            } else if kind.is_file() && file.to_lowercase().ends_with(".exe") && !helper(&file) {
                let stem = plain(&file[..file.len() - 4]);
                let like = stem.len() >= 4 && (stem.contains(&want) || want.contains(&stem));
                let key = (like, e.metadata().map_or(0, |m| m.len()));
                if best.as_ref().map_or(true, |b| key > b.0) {
                    best = Some((key, path));
                }
            }
        }
    }
    best.map(|b| b.1)
}

struct Found(Vec<Game>);

impl Found {
    fn has_exe(&self, path: &str) -> bool {
        self.0.iter().any(|g| g.target.has(file_name(path), Some(path)))
    }

    fn add(&mut self, name: &str, exe: Option<String>, dirs: Vec<String>, icon: Option<String>) {
        let name = name.replace(['™', '®'], "").trim().to_string();
        let dirs: Vec<String> = dirs.iter().map(|d| clean(d)).filter(|d| safe_dir(d)).collect();
        let exe = exe.map(|e| clean(&e));
        let file = exe.as_deref().map_or(String::new(), |e| file_name(e).to_lowercase());
        // without a folder, only an exe name of its own will do
        if name.is_empty() || not_a_game(&name) || (dirs.is_empty() && (file.is_empty() || shared(&file))) {
            return;
        }
        let dup = self.0.iter().any(|g| {
            g.name.eq_ignore_ascii_case(&name)
                || dirs.iter().any(|d| g.target.dirs.iter().any(|o| o.eq_ignore_ascii_case(d) || inside(o, d) || inside(d, o)))
                || ((dirs.is_empty() || g.target.dirs.is_empty()) && !file.is_empty() && g.target.exe == file)
        });
        if dup {
            return;
        }
        let icon = icon
            .map(|i| clean(i.split(',').next().unwrap_or("")))
            .filter(|i| Path::new(i).is_file())
            // a shared runtime's icon says nothing about the game
            .or_else(|| exe.filter(|_| !shared(&file)));
        self.0.push(Game { name, target: Target { exe: file, dirs }, icon: icon.map(|i| Arc::from(i.as_str())) });
    }

    fn add_folder(&mut self, name: &str, dir: &str, icon: Option<String>) {
        let dir = clean(dir);
        if safe_dir(&dir) && Path::new(&dir).is_dir() && !not_a_game(name) {
            self.add(name, main_exe(&dir, name), vec![dir.clone()], icon);
        }
    }
}

fn scan(open: &[Arc<str>]) -> Vec<Game> {
    let mut f = Found(Vec::new());
    roblox(&mut f);
    minecraft(&mut f);
    if let Some(root) = reg::value(reg::HKCU, "Software\\Valve\\Steam", "SteamPath") {
        steam(&mut f, &clean(&root));
    }
    if let Some(pd) = env("ProgramData") {
        epic(&mut f, &format!("{pd}\\Epic\\EpicGamesLauncher\\Data\\Manifests"));
        riot(&mut f, &format!("{pd}\\Riot Games\\Metadata"));
    }
    gog(&mut f);
    ubisoft_and_ea(&mut f);
    for drive in fixed_drives() {
        xbox(&mut f, &format!("{drive}\\XboxGames"));
    }
    install_list(&mut f);
    game_bar(&mut f);
    for path in open {
        if !f.has_exe(path) {
            by_signature(&mut f, path);
        }
    }
    f.0.sort_by_key(|g| g.name.to_lowercase());
    f.0
}

fn roblox(f: &mut Found) {
    let Some(cmd) = reg::value(reg::HKCU, "Software\\Classes\\roblox-player\\shell\\open\\command", "") else { return };
    let exe = cmd.trim_start().strip_prefix('"').and_then(|c| c.split('"').next()).unwrap_or("");
    if Path::new(exe).is_file() {
        // by exe name, as every update and bootstrapper has a folder of its own. The command
        // can point at a bootstrapper, but the game it starts is always this exe.
        f.0.push(Game { name: "Roblox".into(), target: Target { exe: "robloxplayerbeta.exe".into(), dirs: vec![] }, icon: Some(Arc::from(exe)) });
    }
}

/// Java Minecraft runs on the Java that its launcher keeps, so that folder identifies it
fn minecraft(f: &mut Found) {
    let mut dirs = Vec::new();
    if let Some(d) = env("APPDATA") {
        dirs.push(format!("{d}\\.minecraft\\runtime"));
    }
    if let Some(d) = env("LOCALAPPDATA") {
        dirs.push(format!("{d}\\Packages\\Microsoft.4297127D64EC6_8wekyb3d8bbwe\\LocalCache\\Local\\runtime"));
    }
    if let Some(d) = env("ProgramFiles(x86)") {
        dirs.push(format!("{d}\\Minecraft Launcher\\runtime"));
    }
    dirs.retain(|d| Path::new(d).is_dir());
    if !dirs.is_empty() {
        let icon = env("ProgramFiles(x86)").map(|d| format!("{d}\\Minecraft Launcher\\MinecraftLauncher.exe"));
        f.add("Minecraft", Some("javaw.exe".into()), dirs, icon);
    }
}

/// `"key"  "value"` on a line of a Steam file
fn vdf<'a>(text: &'a str, key: &'a str) -> impl Iterator<Item = String> + 'a {
    text.lines().filter_map(move |l| {
        let mut parts = l.trim().splitn(5, '"');
        (parts.next()? == "" && parts.next()? == key).then_some(())?;
        parts.next()?;
        Some(parts.next()?.replace("\\\\", "\\"))
    })
}

fn steam(f: &mut Found, root: &str) {
    let mut libs = vec![root.to_string()];
    if let Ok(t) = std::fs::read_to_string(format!("{root}\\steamapps\\libraryfolders.vdf")) {
        for lib in vdf(&t, "path").map(|p| clean(&p)) {
            if !libs.iter().any(|l| l.eq_ignore_ascii_case(&lib)) {
                libs.push(lib);
            }
        }
    }
    for lib in libs {
        for e in std::fs::read_dir(format!("{lib}\\steamapps")).into_iter().flatten().flatten() {
            let file = e.file_name().to_string_lossy().into_owned();
            if !file.starts_with("appmanifest_") || !file.ends_with(".acf") {
                continue;
            }
            let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
            let get = |k| vdf(&t, k).next();
            let (Some(id), Some(name), Some(dir)) = (get("appid"), get("name"), get("installdir")) else { continue };
            let installed = get("StateFlags").and_then(|s| s.parse::<u32>().ok()).is_some_and(|s| s & 4 != 0);
            // 228980: the redistributables, whose name is translated
            if !installed || id == "228980" || dir.is_empty() {
                continue;
            }
            let icon = [reg::UNINSTALL, reg::UNINSTALL_32].iter().find_map(|base| reg::value(reg::HKLM, &format!("{base}\\Steam App {id}"), "DisplayIcon"));
            f.add_folder(&name, &format!("{lib}\\steamapps\\common\\{dir}"), icon);
        }
    }
}

/// The text value of a key in a flat JSON file
fn json(text: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let rest = text[text.find(&pat)? + pat.len()..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let mut out = String::new();
    let mut it = rest.chars();
    loop {
        match it.next()? {
            '"' => return Some(out),
            '\\' => match it.next()? {
                'u' => out.push(char::from_u32(u32::from_str_radix(&it.by_ref().take(4).collect::<String>(), 16).ok()?).unwrap_or(' ')),
                c => out.push(c),
            },
            c => out.push(c),
        }
    }
}

fn epic(f: &mut Found, manifests: &str) {
    for e in std::fs::read_dir(manifests).into_iter().flatten().flatten() {
        let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
        let (Some(name), Some(dir)) = (json(&t, "DisplayName"), json(&t, "InstallLocation")) else { continue };
        // the engine, plugins and tools say so in their categories
        let cats = t.split_once("\"AppCategories\"").and_then(|c| c.1.split(']').next()).unwrap_or("");
        let dir = clean(&dir);
        if !cats.contains("\"games\"") || !Path::new(&dir).is_dir() {
            continue;
        }
        match json(&t, "LaunchExecutable").map(|x| format!("{dir}\\{}", clean(&x))).filter(|x| Path::new(x).is_file()) {
            Some(exe) => f.add(&name, Some(exe), vec![dir], None),
            None => f.add_folder(&name, &dir, None),
        }
    }
}

fn riot(f: &mut Found, metadata: &str) {
    for product in std::fs::read_dir(metadata).into_iter().flatten().flatten() {
        for e in std::fs::read_dir(product.path()).into_iter().flatten().flatten() {
            if !e.file_name().to_string_lossy().ends_with(".product_settings.yaml") {
                continue;
            }
            let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
            let get = |k: &str| t.lines().find_map(|l| l.trim().strip_prefix(k)?.strip_prefix(':').map(|v| v.trim().trim_matches('"').to_string()));
            if let Some(dir) = get("product_install_full_path") {
                let name = get("shortcut_name").map(|n| n.trim_end_matches(".lnk").to_string()).unwrap_or_else(|| {
                    // ...\Riot Games\VALORANT\live: the folder above "live" is the name
                    let d = clean(&dir);
                    let d = if file_name(&d).eq_ignore_ascii_case("live") { parent(&d) } else { &d };
                    file_name(d).to_string()
                });
                f.add_folder(&name, &dir, None);
            }
        }
    }
}

fn gog(f: &mut Found) {
    let base = "SOFTWARE\\WOW6432Node\\GOG.com\\Games";
    for id in reg::children(reg::HKLM, base) {
        let key = format!("{base}\\{id}");
        let get = |v| reg::value(reg::HKLM, &key, v).filter(|s| !s.is_empty());
        // add-ons say which game they depend on
        if let (Some(name), Some(dir), None) = (get("gameName"), get("path"), get("dependsOn")) {
            match get("exe").filter(|x| Path::new(x).is_file()) {
                Some(exe) => f.add(&name, Some(exe), vec![dir], None),
                None => f.add_folder(&name, &dir, None),
            }
        }
    }
}

fn ubisoft_and_ea(f: &mut Found) {
    let base = "SOFTWARE\\WOW6432Node\\Ubisoft\\Launcher\\Installs";
    for id in reg::children(reg::HKLM, base) {
        if let Some(dir) = reg::value(reg::HKLM, &format!("{base}\\{id}"), "InstallDir").map(|d| clean(&d)) {
            f.add_folder(file_name(&dir), &dir, None);
        }
    }
    for base in ["SOFTWARE\\EA Games", "SOFTWARE\\WOW6432Node\\EA Games"] {
        for name in reg::children(reg::HKLM, base) {
            if let Some(dir) = reg::value(reg::HKLM, &format!("{base}\\{name}"), "Install Dir") {
                f.add_folder(&name, &dir, None);
            }
        }
    }
}

fn xbox(f: &mut Found, root: &str) {
    for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let dir = format!("{root}\\{name}\\Content");
        let Ok(t) = std::fs::read_to_string(format!("{dir}\\MicrosoftGame.config")) else { continue };
        let exe = t
            .split_once("<Executable ")
            .and_then(|x| x.1.split('>').next()?.split_once("Name=\"")?.1.split('"').next())
            .map(|x| format!("{dir}\\{}", clean(x)));
        f.add(&name, exe, vec![dir], None);
    }
}

fn install_list(f: &mut Found) {
    // publishers that make games and little else (their launchers are dropped by name)
    const GAME_MAKERS: &[&str] = &["riot games", "blizzard entertainment", "electronic arts", "ubisoft", "rockstar games", "hoyoverse", "cognosphere", "mihoyo"];
    for (root, base) in [(reg::HKLM, reg::UNINSTALL), (reg::HKLM, reg::UNINSTALL_32), (reg::HKCU, reg::UNINSTALL)] {
        for id in reg::children(root, base) {
            let key = format!("{base}\\{id}");
            let get = |v| reg::value(root, &key, v).filter(|s| !s.trim().is_empty());
            let (Some(name), Some(dir)) = (get("DisplayName"), get("InstallLocation").map(|d| clean(&d))) else { continue };
            // Steam answers for its own
            if id.starts_with("Steam App ") || dir.to_lowercase().contains("\\steamapps\\") || !safe_dir(&dir) {
                continue;
            }
            let maker = get("Publisher").unwrap_or_default().to_lowercase();
            if GAME_MAKERS.iter().any(|m| maker.contains(m)) || ships_like_a_game(&dir) {
                let icon = get("DisplayIcon").filter(|i| !file_name(i).to_lowercase().contains("unins"));
                f.add_folder(&name, &dir, icon);
            }
        }
    }
}

fn game_bar(f: &mut Found) {
    let base = "System\\GameConfigStore\\Children";
    for id in reg::children(reg::HKCU, base) {
        let Some(exe) = reg::value(reg::HKCU, &format!("{base}\\{id}"), "MatchedExeFullPath") else { continue };
        if Path::new(&exe).is_file() && !f.has_exe(&exe) {
            by_signature(f, &exe);
        }
    }
}

/// A program that no launcher lists counts as a game only if it ships like one
fn by_signature(f: &mut Found, exe: &str) {
    let (file, dir) = (file_name(exe), parent(exe));
    if helper(file) || not_a_game(file) || exe.to_lowercase().contains("\\steamapps\\") || !ships_like_a_game(dir) {
        return;
    }
    let name = crate::platform::friendly_name(&file[..file.len().saturating_sub(4)], Some(exe));
    f.add(&name, Some(exe.to_string()), vec![dir.to_string()], None);
}

#[link(name = "kernel32")]
extern "system" {
    fn GetDriveTypeW(root: *const u16) -> u32;
}

/// Only fixed drives: asking an empty card reader or a lost network drive can hang
fn fixed_drives() -> Vec<String> {
    const DRIVE_FIXED: u32 = 3;
    ('A'..='Z').map(|c| format!("{c}:")).filter(|d| unsafe { GetDriveTypeW(win::wide(&format!("{d}\\")).as_ptr()) } == DRIVE_FIXED).collect()
}

/// Read-only registry access
mod reg {
    use crate::sys::win;

    pub const HKCU: isize = 0x8000_0001_u32 as i32 as isize;
    pub const HKLM: isize = win::HKEY_LOCAL_MACHINE;
    pub const UNINSTALL: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
    pub const UNINSTALL_32: &str = "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
    const KEY_READ: u32 = 0x2_0019;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(key: isize, sub: *const u16, opts: u32, access: u32, out: *mut isize) -> i32;
        fn RegEnumKeyExW(key: isize, i: u32, name: *mut u16, len: *mut u32, res: *mut u32, class: *mut u16, class_len: *mut u32, time: *mut u64) -> i32;
        fn RegCloseKey(key: isize) -> i32;
    }

    pub fn children(root: isize, sub: &str) -> Vec<String> {
        let mut out = Vec::new();
        unsafe {
            let mut k = 0isize;
            if RegOpenKeyExW(root, win::wide(sub).as_ptr(), 0, KEY_READ, &mut k) != 0 {
                return out;
            }
            let null = std::ptr::null_mut;
            loop {
                let mut buf = [0u16; 256];
                let mut len = buf.len() as u32;
                if RegEnumKeyExW(k, out.len() as u32, buf.as_mut_ptr(), &mut len, null(), std::ptr::null_mut(), null(), std::ptr::null_mut()) != 0 {
                    break;
                }
                out.push(String::from_utf16_lossy(&buf[..len as usize]));
            }
            RegCloseKey(k);
        }
        out
    }

    pub fn value(root: isize, sub: &str, name: &str) -> Option<String> {
        let (s, n) = (win::wide(sub), win::wide(name));
        let mut buf = [0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe { win::RegGetValueW(root, s.as_ptr(), n.as_ptr(), win::RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as _, &mut len) };
        (r == 0).then(|| String::from_utf16_lossy(&buf[..(len as usize / 2).min(buf.len())]).trim_end_matches('\0').to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_too_broad_to_boost() {
        for d in ["C:", "C:\\", "C:\\Program Files", "C:\\Windows\\System32", "C:\\Users", "D:\\Games", "C:\\SteamLibrary\\steamapps\\common", "", "C:\\X\\.."] {
            assert!(!safe_dir(d), "{d}");
        }
        for d in ["D:\\Games\\Some Game", "D:\\SomeGame", "C:\\Program Files\\Some Game"] {
            assert!(safe_dir(d), "{d}");
        }
    }

    #[test]
    fn only_the_game_is_matched() {
        let t = Target { exe: "javaw.exe".into(), dirs: vec!["C:\\G\\runtime".into()] };
        assert!(t.has("javaw.exe", Some("c:\\g\\runtime\\bin\\javaw.exe")));
        assert!(!t.has("javaw.exe", Some("C:\\G\\runtime2\\javaw.exe")));
        assert!(!t.has("javaw.exe", None));
        for h in ["EasyAntiCheat_EOS.exe", "BEService.exe", "Game_BE.exe", "vgc.exe", "UnityCrashHandler64.exe"] {
            assert!(!t.has(h, Some(&format!("C:\\G\\runtime\\{h}"))), "{h}");
        }
    }

    /// Launchers that may not be on the test PC, played by made-up files
    #[test]
    fn launcher_records() {
        let root = std::env::current_exe().unwrap().parent().unwrap().join("games_fixture");
        let _ = std::fs::create_dir_all(&root);
        let r = root.to_string_lossy().into_owned();
        let put = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        let esc = r.replace('\\', "\\\\");
        put("e\\Alpha\\Bin\\Alpha.exe", "x");
        put("em\\a.item", &format!("{{\"DisplayName\": \"Alpha\", \"InstallLocation\": \"{esc}\\\\e\\\\Alpha\", \"LaunchExecutable\": \"Bin/Alpha.exe\", \"AppCategories\": [\"public\", \"games\"]}}"));
        put("em\\b.item", &format!("{{\"DisplayName\": \"Engine\", \"InstallLocation\": \"{esc}\\\\e\", \"AppCategories\": [\"engines\"]}}"));
        put("r\\Bravo\\live\\Bravo\\Binaries\\Win64\\Bravo-Win64-Shipping.exe", "x");
        put("rm\\bravo.live\\bravo.live.product_settings.yaml", &format!("product_install_full_path: \"{}/r/Bravo/live\"\n", r.replace('\\', "/")));
        put("x\\Charlie\\Content\\MicrosoftGame.config", "<Game><ExecutableList><Executable Name=\"Charlie.exe\" Id=\"Game\"/></ExecutableList></Game>");
        put("x\\GameSave\\readme", "x");
        put("s\\steamapps\\appmanifest_1.acf", "\"AppState\"\n{\n\t\"appid\"\t\t\"1\"\n\t\"name\"\t\t\"Delta\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"installdir\"\t\t\"Delta\"\n}");
        put("s\\steamapps\\common\\Delta\\Delta.exe", "x");

        let mut f = Found(Vec::new());
        epic(&mut f, &format!("{r}\\em"));
        riot(&mut f, &format!("{r}\\rm"));
        xbox(&mut f, &format!("{r}\\x"));
        steam(&mut f, &format!("{r}\\s"));
        let got: Vec<(&str, &str)> = f.0.iter().map(|g| (g.name.as_str(), g.target.exe.as_str())).collect();
        assert_eq!(got, [("Alpha", "alpha.exe"), ("Bravo", "bravo-win64-shipping.exe"), ("Charlie", "charlie.exe"), ("Delta", "delta.exe")]);
    }

    /// `cargo test -- --ignored --nocapture this_pc`: what it finds here, with every running
    /// program put through the engine check too (reads only)
    #[test]
    #[ignore]
    fn this_pc() {
        let running: HashSet<Arc<str>> = (2..50_000u32).filter_map(|n| win::process_path(n * 4)).map(|p| Arc::from(p.as_str())).collect();
        let running: Vec<Arc<str>> = running.into_iter().filter(|p| !p.to_lowercase().contains("\\windows\\")).collect();
        let t = Instant::now();
        let found = scan(&running);
        println!("{} games in {:?}, {} running programs checked", found.len(), t.elapsed(), running.len());
        for g in found {
            println!("{} | {} | {:?} | {:?}", g.name, g.target.exe, g.target.dirs, g.icon);
        }
    }
}
