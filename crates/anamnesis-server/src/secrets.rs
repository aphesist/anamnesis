//! Api keys: the os credential store when there is a usable one, otherwise a private
//! file in the data dir.
//!
//! - Os store ([`anamnesis_harness::keystore`]): service `anamnesis`, account
//!   `<backend id>`. Windows credential manager, macos keychain, or the linux secret
//!   service. The linux kernel keyring is never used (it is cleared on reboot).
//! - File: `<data dir>/anamnesis/secrets.json`, `{ "<account>": "<key>" }`, written
//!   atomically. On unix it is created with mode 0600; on windows it relies on the
//!   per-user acl of `%APPDATA%` (only the user and administrators can read it). It is
//!   not a setting, so it never enters the event log or an exported bundle, and nothing
//!   logs its contents.
//! - Environment: the provider's usual variable (`ANTHROPIC_API_KEY`, …), read last.
//!
//! `ANAMNESIS_KEY_STORAGE=file|keychain` overrides the automatic choice.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub use anamnesis_harness::keystore::KeyStoreMode;
use anamnesis_harness::keystore::OsStore;
use anyhow::{Context, Result};
use serde::Serialize;

/// Keychain service name for api keys.
pub const KEYRING_SERVICE: &str = anamnesis_harness::keystore::SERVICE;
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

/// Where new keys are written (`key_store` in `/api/backends`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyStoreInfo {
    /// `keychain` or `file`.
    pub storage: KeyStorage,
    /// The os store's name (`windows credential manager`, `macos keychain`,
    /// `secret service`), or `file`.
    pub backend: &'static str,
    /// What `ANAMNESIS_KEY_STORAGE` asked for: `auto`, `file` or `keychain`.
    pub mode: &'static str,
    /// The fallback file (shown so the user knows where it is).
    pub file: String,
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

    /// Whether the os store is used (by the mode, and on linux by the probe).
    fn os(&self) -> bool {
        self.mode.uses_os_store()
    }

    /// Where new keys go.
    pub fn info(&self) -> KeyStoreInfo {
        let os = self.os();
        KeyStoreInfo {
            storage: if os {
                KeyStorage::Keychain
            } else {
                KeyStorage::File
            },
            backend: if os {
                OsStore::current().label()
            } else {
                "file"
            },
            mode: match self.mode {
                KeyStoreMode::Auto => "auto",
                KeyStoreMode::File => "file",
                KeyStoreMode::Keychain => "keychain",
            },
            file: self.file.display().to_string(),
        }
    }

    /// The key for `account`: keychain, then file, then the first non-empty `env` var.
    pub fn get(&self, account: &str, env: &[&str]) -> Option<(String, KeyStorage)> {
        if self.os()
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

    /// Store a key. Returns where it went. In `keychain` mode a failing os store is an
    /// error; in `auto` mode the key falls back to the file.
    pub fn set(&self, account: &str, key: &str) -> Result<KeyStorage> {
        if self.os() {
            match keychain_set(account, key) {
                Ok(()) => {
                    // a stale copy in the file would shadow nothing, but keep one place per key
                    self.file_remove(account)?;
                    return Ok(KeyStorage::Keychain);
                }
                Err(e) if self.mode == KeyStoreMode::Keychain => {
                    anyhow::bail!(
                        "could not store the key in the {}: {e}",
                        OsStore::current().label()
                    )
                }
                Err(e) => tracing::info!(
                    "the {} did not take the key ({e}); storing it in the data dir",
                    OsStore::current().label()
                ),
            }
        }
        let _g = self.lock.lock().unwrap();
        let mut all = self.read_file();
        all.insert(account.to_string(), key.to_string());
        self.write_file(&all)?;
        Ok(KeyStorage::File)
    }

    /// Remove a key from every store that holds it (environment variables are left alone).
    pub fn delete(&self, account: &str) -> Result<()> {
        if self.os()
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

/// Store in the os store and read it back.
fn keychain_set(account: &str, key: &str) -> std::result::Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| e.to_string())?;
    entry.set_password(key).map_err(|e| e.to_string())?;
    if keychain_get(account).as_deref() == Some(key.trim()) {
        Ok(())
    } else {
        Err("the stored key did not read back".into())
    }
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
    fn file_mode_reports_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let s = Secrets::new(dir.path(), KeyStoreMode::File);
        let info = s.info();
        assert_eq!(info.storage, KeyStorage::File);
        assert_eq!(info.backend, "file");
        assert_eq!(info.mode, "file");
        assert!(info.file.ends_with(SECRETS_FILE));
        let v = serde_json::to_value(&info).unwrap();
        assert_eq!(v["storage"], "file");
    }

    #[test]
    fn auto_mode_follows_the_os_store() {
        let dir = tempfile::tempdir().unwrap();
        let s = Secrets::new(dir.path(), KeyStoreMode::Auto);
        let info = s.info();
        if anamnesis_harness::keystore::os_store_available() {
            assert_eq!(info.storage, KeyStorage::Keychain);
            assert_eq!(info.backend, OsStore::current().label());
        } else {
            // e.g. linux without a session bus: never the kernel keyring, the file instead
            assert_eq!(info.storage, KeyStorage::File);
            assert_eq!(s.set("probe-test", "k").unwrap(), KeyStorage::File);
            assert_eq!(s.get("probe-test", &[]).unwrap().1, KeyStorage::File);
        }
        assert_eq!(info.mode, "auto");
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
