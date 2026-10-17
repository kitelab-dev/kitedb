//! Connection passwords. They live in the desktop keyring (Secret Service:
//! GNOME Keyring, KWallet, KeePassXC…). Without a keyring they fall back to
//! a file only the user can read, never to `connections.json`. Set
//! `KITEDB_NO_KEYRING=1` to always use that file (tests, headless setups).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use uuid::Uuid;

const SERVICE: &str = "com.kitelab.KiteDB";

fn fallback_path() -> PathBuf {
    super::config_dir().join("passwords.json")
}

fn read_fallback() -> BTreeMap<String, String> {
    fs::read(fallback_path()).ok().and_then(|data| serde_json::from_slice(&data).ok()).unwrap_or_default()
}

fn write_fallback(passwords: &BTreeMap<String, String>) {
    let path = fallback_path();
    if passwords.is_empty() {
        let _ = fs::remove_file(path);
        return;
    }
    let _ = fs::create_dir_all(super::config_dir());
    let Ok(data) = serde_json::to_vec_pretty(passwords) else { return };
    let temp = path.with_extension("json.tmp");
    let written = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)
        .and_then(|mut file| file.write_all(&data));
    if written.is_ok() {
        let _ = fs::rename(temp, path);
    }
}

fn entry(id: Uuid) -> Option<keyring::Entry> {
    if std::env::var_os("KITEDB_NO_KEYRING").is_some_and(|v| !v.is_empty() && v != "0") {
        return None;
    }
    keyring::Entry::new(SERVICE, &id.to_string()).ok()
}

/// Blocking: may wait on D-Bus, so call it off the main thread.
pub fn password(id: Uuid) -> String {
    // Not in the keyring (or no keyring): try the fallback file.
    if let Some(Ok(password)) = entry(id).map(|entry| entry.get_password()) {
        return password;
    }
    read_fallback().remove(&id.to_string()).unwrap_or_default()
}

/// Blocking: may wait on D-Bus, so call it off the main thread.
pub fn set_password(id: Uuid, password: &str) {
    delete_password(id);
    if password.is_empty() {
        return;
    }
    let stored = entry(id).is_some_and(|entry| entry.set_password(password).is_ok());
    if !stored {
        let mut passwords = read_fallback();
        passwords.insert(id.to_string(), password.to_string());
        write_fallback(&passwords);
    }
}

/// Blocking: may wait on D-Bus, so call it off the main thread.
pub fn delete_password(id: Uuid) {
    if let Some(entry) = entry(id) {
        let _ = entry.delete_credential();
    }
    let mut passwords = read_fallback();
    if passwords.remove(&id.to_string()).is_some() {
        write_fallback(&passwords);
    }
}
