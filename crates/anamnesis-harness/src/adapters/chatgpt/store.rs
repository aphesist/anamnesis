//! Storage for anamnesis's *own* "continue with chatgpt" oauth credentials.
//!
//! Primary: the os keychain (`keyring`). Fallback: a json file written atomically with
//! mode 0600 (as the sign-in docs ask: "Store files atomically with 0600 permissions on
//! Unix, and never commit or log them" — https://developers.openai.com/siwc/token-sharing-open-source/sign-in).
//!
//! This never reads any other program's stored login (in particular not the codex cli's).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{HarnessError, Result};

const KEYRING_SERVICE: &str = "anamnesis";
const KEYRING_USER: &str = "chatgpt-auth";

/// The credential record (shape follows the sign-in docs' example).
#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StoredCredentials {
    pub issuer: String,
    /// Issued client id (`oaiapp_…`), saved before the code exchange and kept across sign-outs.
    pub client_id: Option<String>,
    pub ext_agent_host_id: String,
    pub subject: Option<String>,
    pub email: Option<String>,
    pub id_token: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    /// Unix seconds.
    pub expires_at: Option<i64>,
    pub scopes: Vec<String>,
    pub saved_at: Option<i64>,
}

impl std::fmt::Debug for StoredCredentials {
    // never print tokens
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredCredentials")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("has_access_token", &self.access_token.is_some())
            .field("has_refresh_token", &self.refresh_token.is_some())
            .field("expires_at", &self.expires_at)
            .field("scopes", &self.scopes)
            .finish()
    }
}

impl StoredCredentials {
    pub fn has_tokens(&self) -> bool {
        self.access_token.is_some()
    }
    pub fn plan_usage_granted(&self) -> bool {
        self.scopes.iter().any(|s| s == super::oauth::PLAN_SCOPE)
    }
    /// Drop tokens but keep the registration (client id, host id, email hint).
    pub fn clear_tokens(&mut self) {
        self.id_token = None;
        self.access_token = None;
        self.refresh_token = None;
        self.expires_at = None;
        self.scopes.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TokenStoreKind {
    /// Keychain first, 0600 file if the keychain is unavailable.
    #[default]
    Keychain,
    /// Only the 0600 file.
    File,
}

pub struct TokenStore {
    kind: TokenStoreKind,
    file: PathBuf,
}

impl TokenStore {
    pub fn new(kind: TokenStoreKind, dir: &Path) -> Self {
        Self { kind, file: dir.join("chatgpt-auth.json") }
    }

    pub fn file_path(&self) -> &Path {
        &self.file
    }

    fn entry() -> Option<keyring::Entry> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()
    }

    pub fn load(&self) -> Result<Option<StoredCredentials>> {
        if self.kind == TokenStoreKind::Keychain {
            if let Some(e) = Self::entry() {
                match e.get_password() {
                    Ok(s) => return Ok(Some(serde_json::from_str(&s)?)),
                    Err(keyring::Error::NoEntry) => {}
                    Err(err) => tracing::debug!("keychain unavailable, using file: {err}"),
                }
            }
        }
        match std::fs::read_to_string(&self.file) {
            Ok(s) => Ok(Some(serde_json::from_str(&s)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, creds: &StoredCredentials) -> Result<()> {
        let json = serde_json::to_string(creds)?;
        if self.kind == TokenStoreKind::Keychain {
            if let Some(e) = Self::entry() {
                if e.set_password(&json).is_ok() && e.get_password().ok().as_deref() == Some(json.as_str()) {
                    // the keychain holds it; remove any stale file copy
                    let _ = std::fs::remove_file(&self.file);
                    return Ok(());
                }
            }
        }
        write_private(&self.file, json.as_bytes())
    }

    pub fn delete(&self) -> Result<()> {
        if self.kind == TokenStoreKind::Keychain {
            if let Some(e) = Self::entry() {
                let _ = e.delete_credential();
            }
        }
        match std::fs::remove_file(&self.file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Atomic write with 0600 permissions (unix). Creates the parent directory (0700).
pub(crate) fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or_else(|| HarnessError::TokenStore("no parent dir".into()))?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("auth"),
        std::process::id()
    ));
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_roundtrip_is_0600() {
        let dir = tempfile::tempdir().unwrap();
        let st = TokenStore::new(TokenStoreKind::File, dir.path());
        assert!(st.load().unwrap().is_none());
        let c = StoredCredentials {
            issuer: "https://auth.example".into(),
            client_id: Some("oaiapp_x".into()),
            access_token: Some("secret".into()),
            ..Default::default()
        };
        st.save(&c).unwrap();
        assert_eq!(st.load().unwrap().unwrap(), c);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(st.file_path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert!(!format!("{c:?}").contains("secret"));
        st.delete().unwrap();
        assert!(st.load().unwrap().is_none());
    }
}
