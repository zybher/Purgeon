//! The few choices Purgeon remembers between runs, as `key=value` lines in
//! %APPDATA%\Purgeon\settings.ini.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

static VALUES: Mutex<Option<BTreeMap<String, String>>> = Mutex::new(None);

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Purgeon").join("settings.ini"))
}

fn with<R>(f: impl FnOnce(&mut BTreeMap<String, String>) -> R) -> R {
    let mut g = VALUES.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(|| {
        let text = path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect()
    });
    f(m)
}

/// A yes/no setting, `default` when it was never set
pub fn flag(key: &str, default: bool) -> bool {
    with(|m| m.get(key).map_or(default, |v| v == "on"))
}

/// A text setting, if it was set
pub fn text(key: &str) -> Option<String> {
    with(|m| m.get(key).cloned())
}

pub fn set_flag(key: &str, on: bool) {
    let text = with(|m| {
        m.insert(key.to_string(), if on { "on" } else { "off" }.to_string());
        m.iter().map(|(k, v)| format!("{k}={v}\n")).collect::<String>()
    });
    if let Some(p) = path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, text);
    }
}
