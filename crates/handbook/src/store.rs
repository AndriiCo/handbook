//! File-store paths and content-addressed blob I/O.
//!
//! Layout (see `docs/design.md` §2):
//!
//! ```text
//! <root>/handbook.sqlite
//! <root>/warc/...
//! <root>/mbox/...
//! <root>/blob/xx/yy/<sha256>
//! <root>/text/<capture_id>.txt
//! ```

use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// A file-store root on local disk.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Attach to a store root (created by [`Store::init`] if missing).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Store root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path of the SQLite metadata database.
    pub fn db_path(&self) -> PathBuf {
        self.root.join("handbook.sqlite")
    }

    /// Create the root and well-known subdirectories (idempotent).
    pub fn init(&self) -> io::Result<()> {
        for dir in ["warc", "mbox", "blob", "text"] {
            std::fs::create_dir_all(self.root.join(dir))?;
        }
        Ok(())
    }

    /// Content-addressed path for a blob: `blob/xx/yy/<rest-of-sha256>`.
    pub fn blob_path(&self, sha_hex: &str) -> PathBuf {
        debug_assert!(sha_hex.len() >= 4);
        self.root
            .join("blob")
            .join(&sha_hex[..2])
            .join(&sha_hex[2..4])
            .join(&sha_hex[4..])
    }

    /// Store bytes under their sha256 address. Returns the hex digest.
    /// Identical bytes are written once (dedup by address).
    pub fn write_blob(&self, bytes: &[u8]) -> io::Result<String> {
        let sha_hex = sha256_hex(bytes);
        let path = self.blob_path(&sha_hex);
        if path.exists() {
            return Ok(sha_hex);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes)?;
        Ok(sha_hex)
    }

    /// Read a blob by its hex digest.
    pub fn read_blob(&self, sha_hex: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.blob_path(sha_hex))
    }
}

/// Lowercase hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_roundtrip_and_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();

        let sha = store.write_blob(b"hello").unwrap();
        assert_eq!(
            sha,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(store.read_blob(&sha).unwrap(), b"hello");

        // Same bytes, same address, written once.
        let again = store.write_blob(b"hello").unwrap();
        assert_eq!(sha, again);
        assert!(store.blob_path(&sha).exists());
    }

    #[test]
    fn blob_path_uses_two_level_prefix() {
        let store = Store::new("/store");
        let path = store.blob_path("abcdef1234");
        assert_eq!(path, PathBuf::from("/store/blob/ab/cd/ef1234"));
    }
}
