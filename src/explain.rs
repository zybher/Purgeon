//! Plain-English explanations for the details panel: what a process is (from its name and the
//! command line it was started with), who started it, and why it is busy. Fixed rules only,
//! no guessing: every sentence comes from a number Purgeon measured or a fact Windows gave.

use crate::fmt;

/// What a process is, in a few words, plus the specific thing it runs when there is one
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Ident {
    /// "Python script", "Tab or site", "Windows services"
    pub kind: String,
    /// The script, file, module or services: "C:\bots\scraper.py"
    pub target: Option<String>,
}

impl Ident {
    fn new(kind: &str, target: Option<String>) -> Self {
        Ident { kind: kind.into(), target: target.filter(|t| !t.trim().is_empty()) }
    }

    /// One line for a list: "Python script: scraper.py"
    pub fn short(&self) -> String {
        match &self.target {
            Some(t) => format!("{}: {}", self.kind, file_name(t)),
            None => self.kind.clone(),
        }
    }
}

/// The last part of a path ("C:\bots\scraper.py" -> "scraper.py"); other text, such as
/// "TCP/IP NetBIOS Helper", is kept whole
pub fn file_name(s: &str) -> &str {
    let t = s.trim_end_matches(['\\', '/']);
    let path = t.contains('\\') || (t.contains('/') && !t.contains(char::is_whitespace));
    if !path {
        return s;
    }
    t.rsplit(['\\', '/']).next().filter(|x| !x.is_empty()).unwrap_or(s)
}

/// Splits a command line the way Windows programs do (CommandLineToArgvW's rules): the
/// program name ends at the first space or closing quote; after that, quotes group words
/// and backslashes only escape quotes.
pub fn split_args(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let c: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    // the program name
    let mut first = String::new();
    if c.first() == Some(&'"') {
        i = 1;
        while i < c.len() && c[i] != '"' {
            first.push(c[i]);
            i += 1;
        }
        i += 1;
    } else {
        while i < c.len() && !c[i].is_whitespace() {
            first.push(c[i]);
            i += 1;
        }
    }
    out.push(first);
    loop {
        while i < c.len() && c[i].is_whitespace() {
            i += 1;
        }
        if i >= c.len() {
            break;
        }
        let mut arg = String::new();
        let mut quoted = false;
        while i < c.len() {
            let ch = c[i];
            if ch == '\\' {
                let mut n = 0;
                while i < c.len() && c[i] == '\\' {
                    n += 1;
                    i += 1;
                }
                if i < c.len() && c[i] == '"' {
                    arg.extend(std::iter::repeat_n('\\', n / 2));
                    if n % 2 == 1 {
                        arg.push('"');
                        i += 1;
                    }
                } else {
                    arg.extend(std::iter::repeat_n('\\', n));
                }
                continue;
            }
            if ch == '"' {
                // "" inside quotes is a literal quote
                if quoted && c.get(i + 1) == Some(&'"') {
                    arg.push('"');
                    i += 2;
                    continue;
                }
                quoted = !quoted;
                i += 1;
                continue;
            }
            if ch.is_whitespace() && !quoted {
                break;
            }
            arg.push(ch);
            i += 1;
        }
        out.push(arg);
    }
    out
}

fn is_browser(name: &str) -> bool {
    matches!(name, "chrome.exe" | "msedge.exe" | "brave.exe" | "opera.exe" | "opera_gx.exe" | "vivaldi.exe" | "firefox.exe" | "arc.exe" | "thorium.exe" | "yandex.exe")
}

/// The job of one process of a Chromium-based program (Chrome, Edge, and Electron apps such as
/// Discord, Teams, VS Code or Claude), from its `--type=` switch. Firefox's own helper switch too.
pub fn chromium_role(name: &str, args: &[String]) -> Option<&'static str> {
    let get = |key: &str| args.iter().find_map(|a| a.strip_prefix(key));
    if let Some(t) = get("--type=") {
        let sub = get("--utility-sub-type=").unwrap_or("");
        return Some(match t {
            "renderer" if args.iter().any(|a| a == "--extension-process") => "Extension",
            "renderer" if is_browser(name) => "Tab or site",
            "renderer" => "Window content",
            "gpu-process" => "Graphics helper",
            "crashpad-handler" => "Crash reporter",
            "broker" | "gpu-broker" => "Security broker",
            "utility" => match sub {
                s if s.contains("NetworkService") => "Network service",
                s if s.contains("StorageService") => "Storage service",
                s if s.contains("AudioService") => "Audio service",
                s if s.contains("VideoCapture") => "Camera service",
                s if s.contains("DataDecoder") || s.contains("data_decoder") => "File decoder",
                s if s.contains("on_device_model") || s.contains("OnDeviceModel") => "On-device AI model",
                s if s.contains("Printing") || s.contains("printing") => "Printing",
                s if s.contains("Unzipper") || s.contains("unzip") => "Unzipper",
                _ => "Helper",
            },
            _ => "Helper",
        });
    }
    if name == "firefox.exe" && args.iter().any(|a| a == "-contentproc") {
        return Some(match args.last().map(String::as_str) {
            Some("tab") => "Tab or site",
            Some("gpu") => "Graphics helper",
            Some("rdd") => "Media decoder",
            Some("socket") => "Network service",
            _ => "Helper",
        });
    }
    None
}

/// The first argument that isn't a switch, skipping the values of switches that take one
fn first_plain<'a>(args: &'a [String], with_value: &[&str]) -> Option<&'a str> {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if with_value.iter().any(|w| a.eq_ignore_ascii_case(w)) {
            i += 2;
            continue;
        }
        if a.starts_with('-') || (a.starts_with('/') && a.len() <= 3) {
            i += 1;
            continue;
        }
        return Some(a);
    }
    None
}

fn value_of<'a>(args: &'a [String], keys: &[&str]) -> Option<&'a str> {
    args.iter().position(|a| keys.iter().any(|k| a.eq_ignore_ascii_case(k))).and_then(|i| args.get(i + 1)).map(String::as_str)
}

/// Joins the arguments from `from` back into one line (for commands run with -c, /c and the like)
fn rest(args: &[String], from: usize) -> Option<String> {
    let s = args.get(from..)?.join(" ");
    let s = s.trim();
    (!s.is_empty()).then(|| if s.chars().count() > 160 { format!("{}…", s.chars().take(160).collect::<String>()) } else { s.to_string() })
}

/// What this process runs, read from its command line. None when there is nothing more
/// specific to say than its name.
pub fn identify(name: &str, cmd: Option<&str>, services: &[String]) -> Option<Ident> {
    let n = name.to_lowercase();
    if n == "svchost.exe" {
        return Some(match services.len() {
            0 => Ident::new("Windows service host", None),
            1 => Ident::new("Windows service", Some(services[0].clone())),
            _ => Ident::new("Windows services", Some(services.join(", "))),
        });
    }
    let all = split_args(cmd?);
    let args = &all[1..];
    if let Some(role) = chromium_role(&n, args) {
        return Some(Ident::new(role, None));
    }
    let stem = n.trim_end_matches(".exe");
    let id = match stem {
        s if (s.starts_with("python") || s == "py" || s == "pyw") => {
            if let Some(m) = value_of(args, &["-m"]) {
                match m {
                    "ipykernel_launcher" | "ipykernel" => Ident::new("Jupyter notebook kernel", None),
                    "pip" => Ident::new("pip, installing Python packages", rest(args, args.iter().position(|a| a == "-m").unwrap_or(0) + 2)),
                    "http.server" => Ident::new("Python web server", None),
                    _ => Ident::new("Python module", Some(m.to_string())),
                }
            } else if args.iter().any(|a| a == "-c") {
                Ident::new("Python command", rest(args, args.iter().position(|a| a == "-c").unwrap_or(0) + 1))
            } else {
                match first_plain(args, &["-W", "-X", "-Q"]) {
                    Some(script) => Ident::new("Python script", Some(script.to_string())),
                    None => Ident::new("Python shell", None),
                }
            }
        }
        "node" => match first_plain(args, &["-r", "--require", "--import", "--loader", "-e", "--eval"]) {
            Some(s) if s.ends_with("npm-cli.js") => Ident::new("npm", rest(args, 1)),
            Some(s) if s.ends_with("npx-cli.js") => Ident::new("npx", rest(args, 1)),
            Some(s) if s.contains("tsserver") => Ident::new("TypeScript language server", None),
            Some(s) => Ident::new("Node.js script", Some(s.to_string())),
            None if args.iter().any(|a| a == "-e" || a == "--eval") => Ident::new("Node.js command", None),
            None => Ident::new("Node.js shell", None),
        },
        "java" | "javaw" => {
            let joined = args.join(" ").to_lowercase();
            if joined.contains("net.minecraft") || joined.contains("minecraft") {
                Ident::new("Minecraft", None)
            } else if let Some(jar) = value_of(args, &["-jar"]) {
                Ident::new("Java app", Some(jar.to_string()))
            } else {
                match first_plain(args, &["-cp", "-classpath", "--class-path", "-p", "--module-path", "--add-opens", "--add-exports"]) {
                    Some(class) => Ident::new("Java app", Some(class.to_string())),
                    None => Ident::new("Java runtime", None),
                }
            }
        }
        "powershell" | "pwsh" => {
            if let Some(f) = value_of(args, &["-File", "-f"]) {
                Ident::new("PowerShell script", Some(f.to_string()))
            } else if let Some(i) = args.iter().position(|a| ["-command", "-c"].contains(&a.to_lowercase().as_str())) {
                Ident::new("PowerShell command", rest(args, i + 1))
            } else if args.iter().any(|a| a.to_lowercase().starts_with("-enc")) {
                Ident::new("PowerShell command (encoded)", None)
            } else {
                Ident::new("PowerShell window", None)
            }
        }
        "cmd" => match args.iter().position(|a| a.eq_ignore_ascii_case("/c") || a.eq_ignore_ascii_case("/k")) {
            Some(i) => match rest(args, i + 1) {
                Some(r) if r.trim_matches('"').to_lowercase().ends_with(".bat") || r.trim_matches('"').to_lowercase().ends_with(".cmd") => {
                    Ident::new("Batch file", Some(r.trim_matches('"').to_string()))
                }
                r => Ident::new("Command Prompt running", r),
            },
            None => Ident::new("Command Prompt window", None),
        },
        "wscript" | "cscript" => Ident::new("Windows script", Some(first_plain(args, &[])?.to_string())),
        "rundll32" => {
            let a = args.first()?;
            let (dll, entry) = a.split_once(',').unwrap_or((a, ""));
            let entry = entry.trim();
            Ident::new("Runs code from a DLL", Some(if entry.is_empty() { dll.to_string() } else { format!("{} ({entry})", file_name(dll)) }))
        }
        "dllhost" => Ident::new("COM surrogate", None),
        "msiexec" => Ident::new("Windows Installer", value_of(args, &["/i", "/package", "/x", "/uninstall"]).map(str::to_string)),
        "wsl" | "wslhost" | "wslservice" => Ident::new("Windows Subsystem for Linux", None),
        "code" if args.is_empty() => return None,
        _ => return None,
    };
    Some(id)
}

/// What Windows' own and other well-known processes are: (a few words, a sentence)
pub fn about(name: &str) -> Option<(&'static str, &'static str)> {
    let n = name.to_lowercase();
    Some(match n.as_str() {
        "system" => ("Windows kernel", "The core of Windows. Time spent here is drivers and hardware work, so high use usually points to a driver, not an app."),
        "registry" => ("Windows registry", "Holds Windows' settings database in memory."),
        "memory compression" | "memcompression" => ("Compressed memory", "Windows squeezes memory that apps rarely touch into here instead of writing it to disk. Being large is normal when RAM is busy."),
        "secure system" => ("Windows security core", "Virtualization-based security: protects passwords and Windows itself from tampering."),
        "smss.exe" => ("Session manager", "Part of Windows that starts your sign-in session at boot."),
        "csrss.exe" => ("Windows runtime", "Part of Windows that manages console windows, threads and shutdown."),
        "wininit.exe" => ("Windows start-up", "Starts Windows' background services at boot."),
        "winlogon.exe" => ("Sign-in", "Handles signing in, locking the PC and Ctrl+Alt+Del."),
        "services.exe" => ("Service manager", "Starts, stops and watches every Windows service."),
        "lsass.exe" | "lsaiso.exe" => ("Security and sign-in", "Checks passwords and keeps your sign-in tokens."),
        "svchost.exe" => ("Windows service host", "Runs Windows services. Each copy hosts one or a few of them, named below."),
        "dwm.exe" => ("Desktop Window Manager", "Draws every window on your screen. Its GPU and CPU use rise with animations, video and high refresh rates."),
        "explorer.exe" => ("Windows Explorer", "The taskbar, Start button, desktop and File Explorer windows."),
        "fontdrvhost.exe" => ("Font driver", "Draws text in Windows' fonts."),
        "ctfmon.exe" => ("Text input", "Keyboard layouts, the touch keyboard and input methods for other languages."),
        "sihost.exe" => ("Shell host", "Background parts of the Windows desktop such as notifications."),
        "taskhostw.exe" => ("Task host", "Runs scheduled tasks and background parts of Windows for your account."),
        "runtimebroker.exe" => ("Runtime Broker", "Checks what Store apps may use (camera, location, files). One copy per app."),
        "searchhost.exe" | "searchapp.exe" | "searchui.exe" => ("Windows Search", "The search box on the taskbar and in the Start menu."),
        "searchindexer.exe" => ("Search indexer", "Indexes your files so search is fast. Disk use after copying many files is normal."),
        "searchprotocolhost.exe" | "searchfilterhost.exe" => ("Search indexer helper", "Reads files for the search index."),
        "startmenuexperiencehost.exe" => ("Start menu", "The Start menu."),
        "shellexperiencehost.exe" => ("Taskbar panels", "The clock, volume, network and notification panels."),
        "textinputhost.exe" => ("Touch keyboard", "The touch keyboard, emoji panel and handwriting input."),
        "applicationframehost.exe" => ("App frame host", "Draws the window frame around Store apps."),
        "lockapp.exe" => ("Lock screen", "The lock screen."),
        "msmpeng.exe" => ("Microsoft Defender", "Windows' antivirus. It scans files as they're opened, so it gets busy while you install, download or unzip things."),
        "nissrv.exe" => ("Defender network check", "Microsoft Defender's network inspection."),
        "mpdefendercoreservice.exe" => ("Microsoft Defender", "Microsoft Defender's core service."),
        "securityhealthservice.exe" | "securityhealthsystray.exe" => ("Windows Security", "Keeps the Windows Security app and its tray icon up to date."),
        "smartscreen.exe" => ("SmartScreen", "Checks apps and downloads against Microsoft's list of known bad ones."),
        "audiodg.exe" => ("Windows audio", "Mixes sound and applies audio effects. CPU use rises with sound enhancements on."),
        "spoolsv.exe" => ("Print spooler", "Sends documents to printers."),
        "wmiprvse.exe" => ("WMI provider", "Answers programs that ask Windows about the system. High CPU means some program asks it a lot."),
        "conhost.exe" | "openconsole.exe" => ("Console host", "The window behind a command-line program."),
        "dllhost.exe" => ("COM surrogate", "Runs a part of Windows outside Explorer, often for thumbnails or right-click menu add-ons."),
        "rundll32.exe" => ("DLL runner", "Runs code that lives in a DLL file."),
        "backgroundtaskhost.exe" => ("Background task", "Runs a Store app's background work."),
        "wudfhost.exe" => ("Device driver host", "Runs drivers for devices such as sensors, fingerprint readers and some USB devices."),
        "mousocoreworker.exe" | "usocoreworker.exe" => ("Windows Update", "Checks for, downloads and installs Windows updates."),
        "tiworker.exe" | "trustedinstaller.exe" => ("Windows update installer", "Installs Windows updates and features. Heavy CPU and disk for a while is normal."),
        "compattelrunner.exe" => ("Compatibility check", "Windows checks apps for compatibility now and then. It uses CPU and disk for a few minutes."),
        "msiexec.exe" => ("Windows Installer", "Installs, repairs or removes a program."),
        "vmmem" | "vmmemwsl" | "vmmemcmzygote" => ("Virtual machine", "Memory and CPU used by a virtual machine: WSL, Docker or Windows Sandbox."),
        "msedgewebview2.exe" => ("Web view", "Shows web pages inside another app. The app using it is named below."),
        "msedge.exe" => ("Microsoft Edge", "A web browser. Each tab, extension and helper is its own process."),
        "chrome.exe" => ("Google Chrome", "A web browser. Each tab, extension and helper is its own process."),
        "firefox.exe" => ("Firefox", "A web browser. Tabs and helpers run in their own processes."),
        "brave.exe" | "opera.exe" | "opera_gx.exe" | "vivaldi.exe" => ("Web browser", "Each tab, extension and helper is its own process."),
        "widgets.exe" | "widgetservice.exe" => ("Widgets", "The Windows Widgets board."),
        "phoneexperiencehost.exe" => ("Phone Link", "Connects your phone to the PC."),
        "onedrive.exe" => ("OneDrive", "Syncs files with OneDrive. Disk and network use while files sync is normal."),
        "steam.exe" => ("Steam", "The Steam client."),
        "steamwebhelper.exe" => ("Steam's browser", "Draws Steam's store, library and in-game overlay."),
        "python.exe" | "pythonw.exe" | "py.exe" => ("Python", "Runs Python programs. Each copy runs one script, named below."),
        "node.exe" => ("Node.js", "Runs JavaScript programs, often developer tools and servers."),
        "java.exe" | "javaw.exe" => ("Java", "Runs Java programs, Minecraft among them."),
        "powershell.exe" | "pwsh.exe" => ("PowerShell", "A command line and script runner."),
        "cmd.exe" => ("Command Prompt", "A command line, or a batch file running."),
        "windowsterminal.exe" => ("Windows Terminal", "Windows' terminal app."),
        "nvdisplay.container.exe" => ("NVIDIA driver helper", "Part of the NVIDIA driver: its control panel and overlay."),
        "nvcontainer.exe" => ("NVIDIA app helper", "Background part of the NVIDIA app or GeForce Experience."),
        "crashpad_handler.exe" => ("Crash reporter", "Sends a report if its app crashes. Idle otherwise."),
        "purgeon.exe" => ("Purgeon", "This app."),
        _ => return None,
    })
}

/// Who started a process, in words, from its parent's name (None when the parent has exited)
/// and whether the parent hosts Task Scheduler
pub fn origin(parent: Option<&str>, session: u32, scheduler: bool) -> String {
    let Some(p) = parent else {
        return if session == 0 { "Started by Windows in the background".into() } else { "The program that started it has closed".into() };
    };
    let pl = p.to_lowercase();
    match pl.as_str() {
        "services.exe" => "Started by Windows as a background service".into(),
        "explorer.exe" => "You started it (Start menu, taskbar, desktop or a file)".into(),
        "svchost.exe" if scheduler => "Started by Task Scheduler (a scheduled task)".into(),
        "svchost.exe" => "Started by a Windows service".into(),
        "wininit.exe" | "winlogon.exe" | "smss.exe" | "userinit.exe" => "Started by Windows at sign-in".into(),
        "cmd.exe" | "powershell.exe" | "pwsh.exe" | "windowsterminal.exe" | "openconsole.exe" | "conhost.exe" | "bash.exe" | "wsl.exe" => {
            format!("Started from a command line ({p})")
        }
        _ => format!("Started by {p}"),
    }
}

/// The kind of thing a hint is about; picks its dot colour
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Res {
    Cpu,
    Mem,
    Disk,
    Gpu,
    Net,
    Note,
}

#[derive(Clone, Debug)]
pub struct Hint {
    pub res: Res,
    pub text: String,
    /// How much it matters, to put the biggest reason first
    pub weight: f32,
}

/// Everything the "why is it busy" rules look at. For a group, the members added together.
pub struct Facts<'a> {
    pub count: u32,
    /// Share of the whole CPU, 0..100
    pub cpu: f32,
    pub logical: usize,
    /// Recent CPU and RAM samples, oldest first, `interval` seconds apart
    pub cpu_hist: &'a [f32],
    pub ram_hist: &'a [f32],
    pub interval: f32,
    pub ram: u64,
    pub total_ram: u64,
    pub commit: u64,
    /// None when per-process GPU counters are missing
    pub gpu: Option<f32>,
    pub gpu_engine: &'a str,
    pub vram: u64,
    /// Physical disk (read, write) per second; None without administrator
    pub disk: Option<(f64, f64)>,
    /// All I/O (disk, network, devices) per second, always known
    pub io: f64,
    /// Network (received, sent) per second; None without administrator
    pub net: Option<(f64, f64)>,
    pub tcp: u32,
    pub listen: u32,
    /// Distinct internet addresses it holds connections to
    pub internet_peers: usize,
    pub handles: u32,
    pub threads: u32,
}

fn engine_words(e: &str) -> (&'static str, &'static str) {
    match e {
        "3D" => ("3D", "drawing graphics: a game, 3D, animations or video effects"),
        "VideoDecode" => ("video decode", "playing or streaming video"),
        "VideoEncode" => ("video encode", "recording, streaming or a video call"),
        "Compute" => ("compute", "number-crunching such as AI, rendering or mining"),
        "Copy" => ("copy", "moving data to and from the GPU"),
        _ => ("", ""),
    }
}

/// Why this process (or group) is busy, biggest reason first. Never empty.
pub fn hints(f: &Facts) -> Vec<Hint> {
    let mut v: Vec<Hint> = Vec::new();
    let mut add = |res: Res, weight: f32, text: String| v.push(Hint { res, text, weight });

    // ---- CPU
    let logical = f.logical.max(1) as f32;
    let threads_worth = f.cpu / 100.0 * logical;
    if f.cpu >= 0.5 {
        if f.count == 1 && logical >= 4.0 && (0.85..=1.15).contains(&threads_worth) {
            add(
                Res::Cpu,
                f.cpu.max(8.0),
                format!("Keeping one CPU thread fully busy ({}). That's typical of a script or a loop working through a big job, or stuck in one.", fmt::pct(f.cpu)),
            );
        } else {
            add(Res::Cpu, f.cpu, format!("Using {} of your CPU, about {threads_worth:.1} of {} threads' worth.", fmt::pct(f.cpu), f.logical));
        }
    }
    let recent = &f.cpu_hist[f.cpu_hist.len().saturating_sub(30)..];
    if recent.len() >= 10 {
        let mean = recent.iter().sum::<f32>() / recent.len() as f32;
        let max = recent.iter().copied().fold(0.0, f32::max);
        let secs = fmt::span((recent.len() as f32 * f.interval) as f64);
        let steady = recent.iter().filter(|x| **x >= mean * 0.5).count() as f32 / recent.len() as f32;
        if mean >= 3.0 && steady >= 0.85 {
            add(Res::Cpu, mean * 0.6, format!("Busy the whole time for the last {secs}, not just a spike."));
        } else if max >= 5.0 && max >= mean * 3.0 {
            add(Res::Cpu, max * 0.4, format!("Works in bursts: up to {} in the last {secs}, quiet in between.", fmt::pct(max)));
        }
    }

    // ---- memory
    let share = if f.total_ram > 0 { f.ram as f32 / f.total_ram as f32 * 100.0 } else { 0.0 };
    if share >= 5.0 || f.ram >= 1 << 30 {
        add(Res::Mem, share.max(5.0), format!("Holding {} of RAM, {share:.0}% of your PC's memory.", fmt::bytes(f.ram)));
    }
    if f.ram_hist.len() >= 20 {
        let first = f.ram_hist[0] as f64;
        let last = *f.ram_hist.last().unwrap_or(&0.0) as f64;
        let grew = last - first;
        if grew >= 100.0 * 1048576.0 && grew >= first * 0.1 {
            let secs = fmt::span(f.ram_hist.len() as f64 * f.interval as f64);
            add(Res::Mem, 12.0, format!("Memory grew by {} in the last {secs}. If it keeps climbing, it may be leaking.", fmt::bytes(grew as u64)));
        }
    }
    if f.commit > f.ram.saturating_mul(2) && f.commit - f.ram > 1 << 30 {
        add(
            Res::Mem,
            3.0,
            format!("Has {} set aside, more than it keeps in RAM. The rest is in the page file or not used yet.", fmt::bytes(f.commit)),
        );
    }

    // ---- GPU (Windows' per-process GPU figures are estimates)
    if let Some(g) = f.gpu.filter(|g| *g >= 2.0) {
        let (eng, what) = engine_words(f.gpu_engine);
        let text = if eng.is_empty() {
            format!("Using about {g:.0}% of the GPU (Windows' estimate).")
        } else {
            format!("Using about {g:.0}% of the GPU's {eng} engine (Windows' estimate): {what}.")
        };
        add(Res::Gpu, g, text);
    }
    if f.vram >= 512 << 20 {
        add(Res::Gpu, (f.vram as f32 / 1073741824.0 * 4.0).min(20.0), format!("Holding {} of video memory.", fmt::bytes(f.vram)));
    }

    // ---- disk
    match f.disk {
        Some((r, w)) if r + w >= 512.0 * 1024.0 => {
            let wgt = ((r + w) / 1048576.0) as f32 + 4.0;
            if w > r * 2.0 {
                add(Res::Disk, wgt, format!("Mostly writing to disk, {}: saving, installing, downloading, caching or logging.", fmt::rate(w)));
            } else if r > w * 2.0 {
                add(Res::Disk, wgt, format!("Mostly reading from disk, {}: loading or scanning files.", fmt::rate(r)));
            } else {
                add(Res::Disk, wgt, format!("Reading {} and writing {} on disk.", fmt::rate(r), fmt::rate(w)));
            }
        }
        None if f.io >= 1048576.0 => {
            add(Res::Disk, (f.io / 1048576.0) as f32 + 2.0, format!("Moving {} of data (disk, network and devices together).", fmt::rate(f.io)));
        }
        _ => {}
    }

    // ---- network (estimated from Windows' network trace)
    if let Some((rx, tx)) = f.net.filter(|(rx, tx)| rx + tx >= 50.0 * 1024.0) {
        let wgt = ((rx + tx) / 131072.0) as f32 + 4.0;
        let text = if rx > tx * 3.0 {
            format!("Downloading at about {}.", fmt::bits(rx))
        } else if tx > rx * 3.0 {
            format!("Uploading at about {}.", fmt::bits(tx))
        } else {
            format!("Downloading about {} and uploading {}.", fmt::bits(rx), fmt::bits(tx))
        };
        add(Res::Net, wgt, text);
    }
    if f.tcp >= 20 {
        let net = if f.internet_peers > 0 { format!(", to {} internet addresses", f.internet_peers) } else { String::new() };
        add(Res::Net, 3.0 + f.tcp as f32 / 20.0, format!("Holding {} open connections{net}.", f.tcp));
    } else if f.internet_peers > 0 {
        let s = if f.internet_peers == 1 { "" } else { "es" };
        add(Res::Net, 0.6, format!("Connected to {} internet address{s}.", f.internet_peers));
    }
    if f.listen > 0 {
        let s = if f.listen == 1 { "" } else { "s" };
        add(Res::Net, 0.4, format!("Waiting for incoming connections on {} port{s}.", f.listen));
    }

    // ---- leaks
    if f.handles >= 10_000 {
        add(Res::Note, 6.0, format!("Has {} handles open. A count this high that keeps growing usually means a leak.", fmt::thousands(f.handles as u64)));
    }
    if f.count == 1 && f.threads >= 300 {
        add(Res::Note, 2.0, format!("Runs {} threads.", fmt::thousands(f.threads as u64)));
    }

    v.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    if v.iter().all(|h| h.weight < 1.0) {
        v.insert(0, Hint { res: Res::Note, text: "Quiet right now: little CPU, memory, disk, GPU or network use.".into(), weight: 0.0 });
    }
    v.truncate(5);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn splits_like_windows() {
        assert_eq!(split_args(r#""C:\Program Files\Python\python.exe" "C:\my bots\scraper.py" --fast"#), s(&[r"C:\Program Files\Python\python.exe", r"C:\my bots\scraper.py", "--fast"]));
        assert_eq!(split_args(r#"python.exe a\\"b c" d"#), s(&["python.exe", r#"a\b c"#, "d"]));
        assert_eq!(split_args(r#"x.exe a\"b"#), s(&["x.exe", r#"a"b"#]));
        assert_eq!(split_args("x.exe"), s(&["x.exe"]));
    }

    #[test]
    fn python() {
        let i = identify("python.exe", Some(r#""C:\Py\python.exe" -u C:\bots\scraper.py --pages 3"#), &[]).unwrap();
        assert_eq!(i, Ident::new("Python script", Some(r"C:\bots\scraper.py".into())));
        assert_eq!(i.short(), "Python script: scraper.py");
        let i = identify("python.exe", Some(r#"python.exe -m ipykernel_launcher -f x.json"#), &[]).unwrap();
        assert_eq!(i.kind, "Jupyter notebook kernel");
        let i = identify("python.exe", Some("python.exe -X utf8 -m uvicorn app:app"), &[]).unwrap();
        assert_eq!(i, Ident::new("Python module", Some("uvicorn".into())));
        assert_eq!(identify("python.exe", Some("python.exe"), &[]).unwrap().kind, "Python shell");
        assert_eq!(identify("pythonw.exe", Some("pythonw.exe C:\\a\\tray.pyw"), &[]).unwrap().target.as_deref(), Some("C:\\a\\tray.pyw"));
    }

    #[test]
    fn others() {
        assert_eq!(identify("node.exe", Some(r#""node" "C:\nodejs\node_modules\npm\bin\npm-cli.js" run dev"#), &[]).unwrap().kind, "npm");
        assert_eq!(identify("node.exe", Some(r#"node --max-old-space-size=4096 server.js"#), &[]).unwrap().target.as_deref(), Some("server.js"));
        assert_eq!(identify("javaw.exe", Some(r#"javaw -Xmx2G -cp libs net.minecraft.client.main.Main"#), &[]).unwrap().kind, "Minecraft");
        assert_eq!(identify("java.exe", Some(r#"java -jar C:\srv\server.jar nogui"#), &[]).unwrap().target.as_deref(), Some(r"C:\srv\server.jar"));
        assert_eq!(identify("powershell.exe", Some(r#"powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\s\backup.ps1"#), &[]).unwrap().target.as_deref(), Some(r"C:\s\backup.ps1"));
        assert_eq!(identify("cmd.exe", Some(r#"C:\WINDOWS\system32\cmd.exe /c "C:\x\run.bat""#), &[]).unwrap().kind, "Batch file");
        let i = identify("rundll32.exe", Some(r#"C:\WINDOWS\system32\rundll32.exe C:\WINDOWS\System32\shell32.dll,SHCreateLocalServerRunDll {9aa}"#), &[]).unwrap();
        assert_eq!(i.target.as_deref(), Some("shell32.dll (SHCreateLocalServerRunDll)"));
        assert_eq!(identify("svchost.exe", None, &s(&["Windows Update"])).unwrap().short(), "Windows service: Windows Update");
        assert_eq!(identify("svchost.exe", None, &s(&["TCP/IP NetBIOS Helper"])).unwrap().short(), "Windows service: TCP/IP NetBIOS Helper");
        assert_eq!(file_name("./bots/run.py"), "run.py");
        assert!(identify("notepad.exe", Some("notepad.exe a.txt"), &[]).is_none());
        assert!(identify("python.exe", None, &[]).is_none());
    }

    #[test]
    fn chromium() {
        let r = |c: &str, n: &str| identify(n, Some(c), &[]).map(|i| i.kind);
        assert_eq!(r("chrome.exe --type=renderer --lang=en", "chrome.exe").as_deref(), Some("Tab or site"));
        assert_eq!(r("chrome.exe --type=renderer --extension-process", "chrome.exe").as_deref(), Some("Extension"));
        assert_eq!(r("claude.exe --type=renderer", "claude.exe").as_deref(), Some("Window content"));
        assert_eq!(r("msedgewebview2.exe --type=gpu-process", "msedgewebview2.exe").as_deref(), Some("Graphics helper"));
        assert_eq!(r("chrome.exe --type=utility --utility-sub-type=network.mojom.NetworkService", "chrome.exe").as_deref(), Some("Network service"));
        assert_eq!(r("firefox.exe -contentproc -childID 3 tab", "firefox.exe").as_deref(), Some("Tab or site"));
        assert_eq!(r("chrome.exe", "chrome.exe"), None);
    }

    fn facts() -> Facts<'static> {
        Facts {
            count: 1,
            cpu: 0.0,
            logical: 28,
            cpu_hist: &[],
            ram_hist: &[],
            interval: 1.0,
            ram: 50 << 20,
            total_ram: 32 << 30,
            commit: 50 << 20,
            gpu: Some(0.0),
            gpu_engine: "",
            vram: 0,
            disk: Some((0.0, 0.0)),
            io: 0.0,
            net: Some((0.0, 0.0)),
            tcp: 0,
            listen: 0,
            internet_peers: 0,
            handles: 200,
            threads: 10,
        }
    }

    #[test]
    fn quiet_and_busy() {
        let h = hints(&facts());
        assert!(h[0].text.starts_with("Quiet"));
        let mut f = facts();
        f.cpu = 100.0 / 28.0;
        let h = hints(&f);
        assert!(h[0].text.starts_with("Keeping one CPU thread fully busy"), "{:?}", h);
        let mut f = facts();
        f.disk = Some((0.0, 40.0 * 1048576.0));
        f.gpu = Some(35.0);
        f.gpu_engine = "VideoDecode";
        let h = hints(&f);
        assert!(h[0].text.starts_with("Mostly writing"), "{:?}", h);
        assert!(h[1].text.contains("video decode"), "{:?}", h);
        let hist: Vec<f32> = (0..30).map(|i| (1000 + i * 10) as f32 * 1048576.0).collect();
        let mut f = facts();
        f.ram_hist = Box::leak(hist.into_boxed_slice());
        assert!(hints(&f).iter().any(|h| h.text.starts_with("Memory grew by")));
    }
}
