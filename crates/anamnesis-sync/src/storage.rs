//! Where the sync root lives: a folder (that the user's own cloud drive or syncthing
//! syncs) or an s3-compatible bucket the user configured. Paths are `/`-separated and
//! relative to the root.

use std::path::{Path, PathBuf};

use crate::error::{Result, SyncError};

/// A flat key → bytes store. Implementations must make `put` atomic for readers (a
/// reader sees the old file, the new file, or none; never half of one).
pub trait Storage: Send + Sync {
    /// Files under `prefix` (recursively), as root-relative paths, sorted.
    fn list(&self, prefix: &str) -> Result<Vec<String>>;
    fn get(&self, path: &str) -> Result<Option<Vec<u8>>>;
    fn put(&self, path: &str, bytes: &[u8]) -> Result<()>;
    fn delete(&self, path: &str) -> Result<()>;
    /// Move a file. The default copies and deletes.
    fn rename(&self, from: &str, to: &str) -> Result<()> {
        let b = self
            .get(from)?
            .ok_or_else(|| SyncError::Io(format!("{from}: not found")))?;
        self.put(to, &b)?;
        self.delete(from)
    }
    /// Where it is, for the ui ("~/Dropbox/anamnesis", "s3: bucket/prefix").
    fn describe(&self) -> String;
}

fn check_rel(path: &str) -> Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..")
    {
        return Err(SyncError::Invalid(format!(
            "bad path in sync root: {path:?}"
        )));
    }
    Ok(())
}

/// A folder on this machine.
pub struct FolderStorage {
    root: PathBuf,
}

impl FolderStorage {
    pub fn new(root: impl Into<PathBuf>) -> FolderStorage {
        FolderStorage { root: root.into() }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn abs(&self, rel: &str) -> Result<PathBuf> {
        check_rel(rel)?;
        Ok(self
            .root
            .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR)))
    }
}

/// Files a cloud drive or the os leaves behind, never ours.
fn ignored(name: &str) -> bool {
    name.starts_with(".~")
        || name.starts_with("~$")
        || name == ".DS_Store"
        || name == "desktop.ini"
        || name.ends_with(".tmp")
        || name.starts_with(".anamnesis-tmp-")
}

impl Storage for FolderStorage {
    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let base = if prefix.is_empty() {
            self.root.clone()
        } else {
            self.abs(prefix.trim_end_matches('/'))?
        };
        let mut out = Vec::new();
        let mut stack = vec![base];
        while let Some(dir) = stack.pop() {
            let rd = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            for ent in rd {
                let ent = ent?;
                let name = ent.file_name().to_string_lossy().into_owned();
                if ignored(&name) {
                    continue;
                }
                let ft = ent.file_type()?;
                if ft.is_dir() {
                    stack.push(ent.path());
                } else if ft.is_file() {
                    let rel = ent
                        .path()
                        .strip_prefix(&self.root)
                        .map_err(|_| SyncError::Io("path outside the root".into()))?
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/");
                    out.push(rel);
                }
            }
        }
        out.sort();
        Ok(out)
    }

    fn get(&self, path: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read(self.abs(path)?) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn put(&self, path: &str, bytes: &[u8]) -> Result<()> {
        let dest = self.abs(path)?;
        let dir = dest
            .parent()
            .ok_or_else(|| SyncError::Io("no parent".into()))?;
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(
            ".anamnesis-tmp-{}",
            hex::encode(crate::crypto::random::<6>()?)
        ));
        std::fs::write(&tmp, bytes)?;
        if let Err(e) = std::fs::rename(&tmp, &dest) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        match std::fs::remove_file(self.abs(path)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn rename(&self, from: &str, to: &str) -> Result<()> {
        let (a, b) = (self.abs(from)?, self.abs(to)?);
        if let Some(d) = b.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::rename(a, b)?;
        Ok(())
    }

    fn describe(&self) -> String {
        self.root.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_put_get_list_rename_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let s = FolderStorage::new(tmp.path());
        s.put("devices/a/batches/1.anb", b"x").unwrap();
        s.put("inbox/note.md", b"hi").unwrap();
        std::fs::write(tmp.path().join("inbox/.DS_Store"), b"").unwrap();
        assert_eq!(
            s.list("").unwrap(),
            vec!["devices/a/batches/1.anb", "inbox/note.md"]
        );
        assert_eq!(s.list("inbox/").unwrap(), vec!["inbox/note.md"]);
        assert_eq!(s.list("nothing/").unwrap(), Vec::<String>::new());
        assert_eq!(s.get("inbox/note.md").unwrap().unwrap(), b"hi");
        assert_eq!(s.get("inbox/none.md").unwrap(), None);
        s.rename("inbox/note.md", "inbox/imported/note.md").unwrap();
        assert_eq!(s.list("inbox/").unwrap(), vec!["inbox/imported/note.md"]);
        s.delete("inbox/imported/note.md").unwrap();
        s.delete("inbox/imported/note.md").unwrap();
        for bad in ["../x", "/x", "a//b", "a/./b", ""] {
            assert!(s.put(bad, b"").is_err(), "{bad}");
        }
    }
}
