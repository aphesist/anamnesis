//! Encryption on the device: passphrase → key with argon2id, files sealed with
//! XChaCha20-Poly1305. The storage only ever holds ciphertext (and the plain `inbox/`).
//!
//! A sealed file is `MAGIC (4 bytes) ‖ nonce (24 bytes) ‖ ciphertext+tag`. The
//! additional authenticated data is the file's path inside the sync root, so a file
//! moved to another name (another device's folder, an older batch's name) fails to open.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::{Result, SyncError};

/// First bytes of every sealed file ("anamnesis sealed, version 1").
pub const MAGIC: &[u8; 4] = b"ANS1";
const NONCE_LEN: usize = 24;
/// The plaintext sealed into `vault.json`'s `check` to verify a passphrase.
const CHECK_PLAINTEXT: &[u8] = b"anamnesis sync vault";
const CHECK_AAD: &str = "vault.json#check";

/// The symmetric key every device of a vault shares (derived from the passphrase).
#[derive(Clone)]
pub struct Key([u8; 32]);

impl Drop for Key {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl Key {
    pub fn from_bytes(b: [u8; 32]) -> Key {
        Key(b)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    /// Hex, for storing in the os keychain.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
    pub fn from_hex(s: &str) -> Result<Key> {
        let v = hex::decode(s.trim()).map_err(|_| SyncError::BadKey)?;
        let b: [u8; 32] = v.try_into().map_err(|_| SyncError::BadKey)?;
        Ok(Key(b))
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(…)")
    }
}

/// argon2id parameters, stored in plain text in `vault.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Always `"argon2id"`.
    pub alg: String,
    /// Hex, 16 bytes.
    pub salt: String,
    /// Memory in KiB.
    pub m_kib: u32,
    /// Iterations.
    pub t: u32,
    /// Parallelism.
    pub p: u32,
}

impl KdfParams {
    /// Fresh parameters with a random salt: 64 MiB, 3 passes, 1 lane (OWASP's argon2id
    /// guidance is at least 19 MiB with 2 passes; this is above it and still about a
    /// second on a phone-class cpu).
    pub fn new() -> Result<KdfParams> {
        Ok(KdfParams {
            alg: "argon2id".into(),
            salt: hex::encode(random::<16>()?),
            m_kib: 64 * 1024,
            t: 3,
            p: 1,
        })
    }

    /// Cheap parameters for tests only.
    #[doc(hidden)]
    pub fn insecure_for_tests() -> KdfParams {
        KdfParams {
            alg: "argon2id".into(),
            salt: hex::encode([7u8; 16]),
            m_kib: 64,
            t: 1,
            p: 1,
        }
    }

    /// Derive the key from a passphrase.
    pub fn derive(&self, passphrase: &str) -> Result<Key> {
        if self.alg != "argon2id" {
            return Err(SyncError::Format(format!("unknown kdf {}", self.alg)));
        }
        let salt = hex::decode(&self.salt).map_err(|_| SyncError::Format("bad salt".into()))?;
        let params = Params::new(self.m_kib, self.t, self.p, Some(32))
            .map_err(|e| SyncError::Format(format!("kdf parameters: {e}")))?;
        let mut out = [0u8; 32];
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(passphrase.as_bytes(), &salt, &mut out)
            .map_err(|e| SyncError::Format(format!("kdf: {e}")))?;
        Ok(Key(out))
    }
}

pub(crate) fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).map_err(|e| SyncError::Io(format!("random: {e}")))?;
    Ok(b)
}

/// Seal `plaintext` for the file at `path` (the path is authenticated, not encrypted).
pub fn seal(key: &Key, path: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    let nonce = random::<NONCE_LEN>()?;
    let ct = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: path.as_bytes(),
            },
        )
        .map_err(|_| SyncError::Crypto)?;
    let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ct.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Open a sealed file read from `path`. Fails on a wrong key, a modified file, or a file
/// that was moved from another path.
pub fn open(key: &Key, path: &str, sealed: &[u8]) -> Result<Vec<u8>> {
    if sealed.len() < MAGIC.len() + NONCE_LEN + 16 || &sealed[..MAGIC.len()] != MAGIC {
        return Err(SyncError::Tampered(format!("{path}: not a sealed file")));
    }
    let nonce = &sealed[MAGIC.len()..MAGIC.len() + NONCE_LEN];
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: &sealed[MAGIC.len() + NONCE_LEN..],
                aad: path.as_bytes(),
            },
        )
        .map_err(|_| SyncError::Tampered(format!("{path}: wrong key or modified")))
}

/// `vault.json`: the one plain file of a sync root besides the inbox.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vault {
    /// `"anamnesis-sync"`.
    pub format: String,
    pub version: u32,
    /// Random id of this vault (hex), so a device notices a root swapped under it.
    pub vault_id: String,
    pub kdf: KdfParams,
    /// [`CHECK_PLAINTEXT`] sealed with the key, hex: verifies a passphrase.
    pub check: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub const VAULT_FORMAT: &str = "anamnesis-sync";
pub const VAULT_VERSION: u32 = 1;

impl Vault {
    /// A new vault for `passphrase`. Returns it with the derived key.
    pub fn create(passphrase: &str, kdf: KdfParams) -> Result<(Vault, Key)> {
        if passphrase.chars().count() < 8 {
            return Err(SyncError::Invalid(
                "the passphrase needs at least 8 characters".into(),
            ));
        }
        let key = kdf.derive(passphrase)?;
        let check = hex::encode(seal(&key, CHECK_AAD, CHECK_PLAINTEXT)?);
        Ok((
            Vault {
                format: VAULT_FORMAT.into(),
                version: VAULT_VERSION,
                vault_id: hex::encode(random::<16>()?),
                kdf,
                check,
                created_at: chrono::Utc::now(),
            },
            key,
        ))
    }

    /// Derive the key from `passphrase` and check it against the vault.
    pub fn unlock(&self, passphrase: &str) -> Result<Key> {
        let key = self.kdf.derive(passphrase)?;
        self.verify(&key)?;
        Ok(key)
    }

    /// Is `key` this vault's key?
    pub fn verify(&self, key: &Key) -> Result<()> {
        if self.format != VAULT_FORMAT {
            return Err(SyncError::Format(format!(
                "not a sync vault: {}",
                self.format
            )));
        }
        if self.version > VAULT_VERSION {
            return Err(SyncError::Format(format!(
                "the vault has format version {}; this app reads up to {VAULT_VERSION}",
                self.version
            )));
        }
        let sealed = hex::decode(&self.check).map_err(|_| SyncError::Format("bad check".into()))?;
        match open(key, CHECK_AAD, &sealed) {
            Ok(p) if p == CHECK_PLAINTEXT => Ok(()),
            _ => Err(SyncError::WrongPassphrase),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_round_trip_and_tamper() {
        let (vault, key) =
            Vault::create("correct horse battery", KdfParams::insecure_for_tests()).unwrap();
        let sealed = seal(&key, "devices/a/batches/1.anb", b"hello").unwrap();
        assert_eq!(&sealed[..4], MAGIC);
        assert!(!sealed.windows(5).any(|w| w == b"hello"), "ciphertext only");
        assert_eq!(
            open(&key, "devices/a/batches/1.anb", &sealed).unwrap(),
            b"hello"
        );
        // moved to another path
        assert!(matches!(
            open(&key, "devices/b/batches/1.anb", &sealed),
            Err(SyncError::Tampered(_))
        ));
        // a flipped bit
        let mut bad = sealed.clone();
        let n = bad.len() - 1;
        bad[n] ^= 1;
        assert!(matches!(
            open(&key, "devices/a/batches/1.anb", &bad),
            Err(SyncError::Tampered(_))
        ));
        // the passphrase check
        assert!(vault.unlock("correct horse battery").is_ok());
        assert!(matches!(
            vault.unlock("wrong horse battery"),
            Err(SyncError::WrongPassphrase)
        ));
        let other = KdfParams {
            salt: hex::encode([9u8; 16]),
            ..KdfParams::insecure_for_tests()
        };
        let k2 = other.derive("correct horse battery").unwrap();
        assert_ne!(k2.as_bytes(), key.as_bytes(), "the salt matters");
        assert!(Vault::create("short", KdfParams::insecure_for_tests()).is_err());
        assert_eq!(
            Key::from_hex(&key.to_hex()).unwrap().as_bytes(),
            key.as_bytes()
        );
    }
}
