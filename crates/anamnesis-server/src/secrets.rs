//! Api keys: the os keychain when there is one, otherwise a 0600 file in the data dir.
//!
//! - Keychain: service `anamnesis`, account `<backend id>`.
//! - File: `<data dir>/anamnesis/secrets.json`, `{ "<account>": "<key>" }`, written
//!   atomically with mode 0600. It is not a setting, so it never enters the event log or
//!   an exported bundle, and nothing logs its contents.
//! - Environment: the provider's usual variable (`ANTHROPIC_API_KEY`, …), read last.
//!
//! `ANAMNESIS_KEY_STORAGE=file` skips the keychain entirely (useful on linux, where the
//! kernel keyring the `keyring` crate uses does not survive a reboot).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::Serialize;

/// Keychain service name for api keys.
pub const KEYRING_SERVICE: &str = "anamnesis";
/// File name of the fallback store, inside the data root.
pub const SECRETS_FILE: &str = "secrets.json";

/// Where a key was found (`key_storage` in `/api/backends`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStorage {
    Keychain,
    File,
    Env,
}

/// Which stores to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyStoreMode {
    /// The os keychain, falling back to the 0600 file when it is unavailable.
    #[default]
    Auto,
    /// Only the 0600 file (no keychain access at all).
    File,
}

impl KeyStoreMode {
    /// From `ANAMNESIS_KEY_STORAGE` (`file` or `auto`; anything else is `auto`).
    pub fn from_env() -> Self {
        match std::env::var("ANAMNESIS_KEY_STORAGE").as_deref() {
            Ok("file") => KeyStoreMode::File,
            _ => KeyStoreMode::Auto,
        }
    }
}

pub struct Secrets {
    file: PathBuf,
    mode: KeyStoreMode,
    lock: Mutex<()>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // never print keys
        f.debug_struct("Secrets")
            .field("file", &self.file)
            .field("mode", &self.mode)
            .finish()
    }
}

impl Secrets {
    /// Keys under `root` (`<root>/secrets.json`).
    pub fn new(root: &Path, mode: KeyStoreMode) -> Self {
        Secrets {
            file: root.join(SECRETS_FILE),
            mode,
            lock: Mutex::new(()),
        }
    }

    pub fn file_path(&self) -> &Path {
        &self.file
    }

    /// The key for `account`: keychain, then file, then the first non-empty `env` var.
    pub fn get(&self, account: &str, env: &[&str]) -> Option<(String, KeyStorage)> {
        if self.mode == KeyStoreMode::Auto
            && let Some(k) = keychain_get(account)
        {
            return Some((k, KeyStorage::Keychain));
        }
        if let Some(k) = self.read_file().remove(account) {
            let k = k.trim().to_string();
            if !k.is_empty() {
                return Some((k, KeyStorage::File));
            }
        }
        env.iter()
            .filter_map(|v| std::env::var(v).ok())
            .map(|k| k.trim().to_string())
            .find(|k| !k.is_empty())
            .map(|k| (k, KeyStorage::Env))
    }

    /// Store a key. Returns where it went.
    pub fn set(&self, account: &str, key: &str) -> Result<KeyStorage> {
        if self.mode == KeyStoreMode::Auto && keychain_set(account, key) {
            // a stale copy in the file would shadow nothing, but keep one place per key
            self.file_remove(account)?;
            return Ok(KeyStorage::Keychain);
        }
        let _g = self.lock.lock().unwrap();
        let mut all = self.read_file();
        all.insert(account.to_string(), key.to_string());
        self.write_file(&all)?;
        Ok(KeyStorage::File)
    }

    /// Remove a key from every store that holds it (environment variables are left alone).
    pub fn delete(&self, account: &str) -> Result<()> {
        if self.mode == KeyStoreMode::Auto
            && let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, account)
        {
            // no keychain, or no entry: nothing to remove there
            let _ = entry.delete_credential();
        }
        self.file_remove(account)
    }

    fn file_remove(&self, account: &str) -> Result<()> {
        let _g = self.lock.lock().unwrap();
        let mut all = self.read_file();
        if all.remove(account).is_some() {
            self.write_file(&all)?;
        }
        Ok(())
    }

    fn read_file(&self) -> BTreeMap<String, String> {
        std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn write_file(&self, all: &BTreeMap<String, String>) -> Result<()> {
        let dir = self.file.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let tmp = dir.join(format!(".{SECRETS_FILE}.tmp"));
        let body = serde_json::to_vec_pretty(all)?;
        {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts
                .open(&tmp)
                .with_context(|| format!("writing {}", tmp.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            f.write_all(&body)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.file)
            .with_context(|| format!("replacing {}", self.file.display()))?;
        Ok(())
    }
}

fn keychain_get(account: &str) -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account).ok()?;
    let k = entry.get_password().ok()?;
    let k = k.trim().to_string();
    (!k.is_empty()).then_some(k)
}

/// Store in the keychain and read it back; false when there is no usable keychain.
fn keychain_set(account: &str, key: &str) -> bool {
    let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, account) else {
        return false;
    };
    if let Err(e) = entry.set_password(key) {
        tracing::info!("no usable os keychain ({e}); storing the key in the data dir (0600)");
        return false;
    }
    keychain_get(account).as_deref() == Some(key.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_roundtrip_with_0600() {
        let dir = tempfile::tempdir().unwrap();
        let s = Secrets::new(dir.path(), KeyStoreMode::File);
        assert!(s.get("anthropic", &[]).is_none());
        assert_eq!(s.set("anthropic", "sk-test-1").unwrap(), KeyStorage::File);
        assert_eq!(s.set("gemini", "g-2").unwrap(), KeyStorage::File);
        assert_eq!(
            s.get("anthropic", &[]),
            Some(("sk-test-1".into(), KeyStorage::File))
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(s.file_path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        s.delete("anthropic").unwrap();
        assert!(s.get("anthropic", &[]).is_none());
        assert_eq!(s.get("gemini", &[]).unwrap().0, "g-2");
        // the debug form never shows a key
        assert!(!format!("{s:?}").contains("g-2"));
    }

    #[test]
    fn env_is_the_last_resort() {
        let dir = tempfile::tempdir().unwrap();
        let s = Secrets::new(dir.path(), KeyStoreMode::File);
        // PATH is always set: stands in for a provider variable
        let (_, from) = s.get("nobody", &["ANAMNESIS_NO_SUCH_VAR", "PATH"]).unwrap();
        assert_eq!(from, KeyStorage::Env);
        s.set("nobody", "from-file").unwrap();
        assert_eq!(s.get("nobody", &["PATH"]).unwrap().1, KeyStorage::File);
    }
}
