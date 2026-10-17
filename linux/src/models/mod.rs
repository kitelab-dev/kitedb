pub mod connection;
pub mod query;
pub mod secrets;
pub mod store;
pub mod table;

use std::path::PathBuf;
use std::time::Duration;

/// `$XDG_CONFIG_HOME/kitedb` (usually `~/.config/kitedb`).
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("kitedb")
}

pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs_f64();
    if seconds < 1.0 { format!("{} ms", (seconds * 1000.0).round() as u64) } else { format!("{seconds:.2} s") }
}

pub fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}
