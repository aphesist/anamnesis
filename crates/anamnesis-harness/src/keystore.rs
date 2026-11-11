//! Which os credential store holds secrets, and whether it is usable on this machine.
//!
//! The `keyring` crate is built with one persistent store per os:
//!
//! - windows: the credential manager (per user, protected by the user's login);
//! - macos: the login keychain;
//! - linux (and the bsds): the freedesktop secret service (gnome keyring, kwallet,
//!   keepassxc) over the session d-bus.
//!
//! The linux kernel keyring (`keyutils`) is deliberately **not** used: it is cleared on
//! reboot, so a key saved there silently disappears.
//!
//! On linux there may be no secret service at all (a server, a container, a desktop
//! without a keyring daemon). [`os_store_available`] probes it once, with a timeout, so
//! callers can fall back to a private file in the data dir instead of failing or hanging.
//!
//! `ANAMNESIS_KEY_STORAGE` overrides the choice for every store in anamnesis:
//! `file` never touches the os store, `keychain` always uses it (errors are reported
//! instead of falling back), anything else (or unset) is `auto`.

use std::sync::OnceLock;
use std::time::Duration;

/// Keychain service name shared by every secret anamnesis stores.
pub const SERVICE: &str = "anamnesis";

/// How long the one-time linux probe may take before the store counts as unavailable
/// (a locked keyring may wait for an unlock prompt nobody sees).
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// The os credential store this build talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OsStore {
    WindowsCredentialManager,
    MacosKeychain,
    SecretService,
    /// No supported store on this platform.
    None,
}

impl OsStore {
    /// The store for the platform this binary was built for.
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            OsStore::WindowsCredentialManager
        } else if cfg!(target_os = "macos") {
            OsStore::MacosKeychain
        } else if cfg!(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "openbsd"
        )) {
            OsStore::SecretService
        } else {
            OsStore::None
        }
    }

    /// A short lowercase name for the ui.
    pub const fn label(self) -> &'static str {
        match self {
            OsStore::WindowsCredentialManager => "windows credential manager",
            OsStore::MacosKeychain => "macos keychain",
            OsStore::SecretService => "secret service",
            OsStore::None => "none",
        }
    }
}

/// The user's choice from `ANAMNESIS_KEY_STORAGE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyStoreMode {
    /// The os store when it is usable, otherwise the private file.
    #[default]
    Auto,
    /// Only the private file in the data dir.
    File,
    /// Only the os store.
    Keychain,
}

impl KeyStoreMode {
    /// Parse a value of `ANAMNESIS_KEY_STORAGE` (case-insensitive).
    pub fn parse(v: &str) -> Self {
        match v.trim().to_ascii_lowercase().as_str() {
            "file" => KeyStoreMode::File,
            "keychain" | "keyring" | "os" => KeyStoreMode::Keychain,
            _ => KeyStoreMode::Auto,
        }
    }

    /// From `ANAMNESIS_KEY_STORAGE`.
    pub fn from_env() -> Self {
        std::env::var("ANAMNESIS_KEY_STORAGE")
            .map(|v| Self::parse(&v))
            .unwrap_or_default()
    }

    /// Whether this mode should try the os store at all.
    pub fn uses_os_store(self) -> bool {
        match self {
            KeyStoreMode::File => false,
            KeyStoreMode::Keychain => true,
            KeyStoreMode::Auto => os_store_available(),
        }
    }
}

/// Whether the os store answers. Windows and macos always have one; on linux the secret
/// service is probed once per process (cached), with a timeout.
pub fn os_store_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| match OsStore::current() {
        OsStore::WindowsCredentialManager | OsStore::MacosKeychain => true,
        OsStore::SecretService => probe(),
        OsStore::None => false,
    })
}

/// Read a probe entry on a separate thread. `NoEntry` (or a value) means the store
/// answered; any other error, or no answer in time, means it is not usable.
fn probe() -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("keystore-probe".into())
        .spawn(move || {
            let ok = match keyring::Entry::new(SERVICE, "__probe__") {
                Ok(e) => matches!(e.get_password(), Ok(_) | Err(keyring::Error::NoEntry)),
                Err(_) => false,
            };
            let _ = tx.send(ok);
        });
    if spawned.is_err() {
        return false;
    }
    let ok = rx.recv_timeout(PROBE_TIMEOUT).unwrap_or(false);
    if !ok {
        tracing::info!(
            "no usable {} (no session d-bus or keyring daemon); secrets go to a private file in the data dir",
            OsStore::current().label()
        );
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_modes() {
        assert_eq!(KeyStoreMode::parse("file"), KeyStoreMode::File);
        assert_eq!(KeyStoreMode::parse(" FILE "), KeyStoreMode::File);
        assert_eq!(KeyStoreMode::parse("keychain"), KeyStoreMode::Keychain);
        assert_eq!(KeyStoreMode::parse("auto"), KeyStoreMode::Auto);
        assert_eq!(KeyStoreMode::parse("whatever"), KeyStoreMode::Auto);
        assert!(!KeyStoreMode::File.uses_os_store());
        assert!(KeyStoreMode::Keychain.uses_os_store());
    }

    #[test]
    fn current_store_matches_the_platform() {
        let s = OsStore::current();
        #[cfg(target_os = "windows")]
        assert_eq!(s, OsStore::WindowsCredentialManager);
        #[cfg(target_os = "macos")]
        assert_eq!(s, OsStore::MacosKeychain);
        #[cfg(target_os = "linux")]
        assert_eq!(s, OsStore::SecretService);
        assert!(!s.label().is_empty());
    }

    #[test]
    fn probe_never_hangs() {
        // whatever the machine has, the answer comes back within the probe timeout
        let t = std::time::Instant::now();
        let _ = os_store_available();
        assert!(t.elapsed() < PROBE_TIMEOUT + Duration::from_secs(2));
    }
}
