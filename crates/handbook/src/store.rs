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
    ///
    /// `sha_hex` must be a 64-character lowercase hex digest. Anything
    /// else (wrong length, non-hex, uppercase, separators) returns
    /// [`io::ErrorKind::InvalidInput`] instead of panicking, so
    /// untrusted input can never escape the blob directory.
    pub fn blob_path(&self, sha_hex: &str) -> io::Result<PathBuf> {
        let valid = sha_hex.len() == 64
            && sha_hex
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'));
        if !valid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid sha256 digest: {sha_hex:?}"),
            ));
        }
        Ok(self
            .root
            .join("blob")
            .join(&sha_hex[..2])
            .join(&sha_hex[2..4])
            .join(&sha_hex[4..]))
    }

    /// Store bytes under their sha256 address. Returns the hex digest.
    /// Identical bytes are written once (dedup by address).
    ///
    /// The write is atomic: bytes go to a temp file in the same
    /// directory, then `rename` publishes the final address, so a
    /// concurrent reader never sees a torn file.
    pub fn write_blob(&self, bytes: &[u8]) -> io::Result<String> {
        let sha_hex = sha256_hex(bytes);
        let path = self.blob_path(&sha_hex)?;
        if path.exists() {
            return Ok(sha_hex);
        }
        let parent = path
            .parent()
            .expect("blob path always has a parent directory");
        std::fs::create_dir_all(parent)?;
        // A racing writer may have won while we created directories.
        if path.exists() {
            return Ok(sha_hex);
        }
        // Stage in the same directory so the rename stays on one
        // filesystem (atomic on APFS and ext4). `create_new` plus
        // retries keep concurrent writers off one shared temp name.
        let mut staged: Option<PathBuf> = None;
        for attempt in 0..100 {
            let candidate = parent.join(format!(".tmp-{}-{attempt}", std::process::id()));
            let mut file = match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            };
            use std::io::Write as _;
            file.write_all(bytes)?;
            staged = Some(candidate);
            break;
        }
        let Some(tmp) = staged else {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not stage blob temp file",
            ));
        };
        match std::fs::rename(&tmp, &path) {
            Ok(()) => Ok(sha_hex),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                // A racing writer may have published identical bytes
                // first (same bytes hash to the same address): a win.
                if path.exists() { Ok(sha_hex) } else { Err(e) }
            }
        }
    }

    /// Read a blob by its hex digest.
    pub fn read_blob(&self, sha_hex: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.blob_path(sha_hex)?)
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
        assert!(store.blob_path(&sha).unwrap().exists());
    }

    #[test]
    fn blob_path_uses_two_level_prefix() {
        let store = Store::new("/store");
        let sha = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let path = store.blob_path(sha).unwrap();
        assert_eq!(
            path,
            PathBuf::from(
                "/store/blob/2c/f2/4dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
            )
        );
    }

    #[test]
    fn blob_path_rejects_invalid_digests() {
        let store = Store::new("/store");
        let valid = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(store.blob_path(valid).is_ok());
        let (head, tail) = valid.split_at(32);
        let bad_inputs = [
            // Wrong length.
            "abcdef1234".to_string(),
            String::new(),
            // Non-hex (right length).
            "x".repeat(64),
            // Uppercase hex.
            valid.to_uppercase(),
            // Path traversal.
            "../../../../etc".to_string(),
            // Embedded separator.
            format!("{head}/{tail}"),
        ];
        for bad in &bad_inputs {
            let err = store.blob_path(bad).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "input: {bad:?}");
        }
    }
}
