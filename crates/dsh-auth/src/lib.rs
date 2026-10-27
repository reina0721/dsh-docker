// SPDX-License-Identifier: MIT OR Apache-2.0

//! Authentication primitives shared by `dshctl` and the container gateway.
//!
//! The on-disk format is the same `username bcrypt-hash` format Caddy used, so
//! existing `users.caddy` files can be migrated without re-hashing passwords.

use std::{
    borrow::Cow,
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use tempfile::NamedTempFile;

/// Maximum username length accepted by the CLI and gateway.
pub const MAX_USERNAME_LEN: usize = 64;
/// bcrypt only uses the first 72 bytes of a password.
pub const MAX_PASSWORD_BYTES: usize = 72;

/// A single htpasswd entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub hash: String,
}

/// Parsed htpasswd database.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserStore {
    users: BTreeMap<String, String>,
}

impl UserStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse htpasswd content.  Invalid lines are reported to the caller and
    /// skipped; the gateway keeps serving the last known-good database when a
    /// reload cannot be parsed.
    pub fn parse(content: &str) -> (Self, Vec<String>) {
        let mut store = Self::new();
        let mut warnings = Vec::new();
        for (index, raw) in content.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let Some(name) = parts.next() else {
                continue;
            };
            let Some(hash) = parts.next() else {
                warnings.push(format!("line {}: missing password hash", index + 1));
                continue;
            };
            if parts.next().is_some() {
                warnings.push(format!("line {}: extra fields ignored", index + 1));
            }
            if let Err(error) = validate_username(name) {
                warnings.push(format!("line {}: {error}", index + 1));
                continue;
            }
            if !validate_hash(hash) {
                warnings.push(format!(
                    "line {}: invalid bcrypt hash for {name}",
                    index + 1
                ));
                continue;
            }
            if store
                .users
                .insert(name.to_owned(), hash.to_owned())
                .is_some()
            {
                warnings.push(format!(
                    "line {}: duplicate user {name} (last wins)",
                    index + 1
                ));
            }
        }
        (store, warnings)
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(content) => Ok(Self::parse(&content).0),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::new()),
            Err(error) => Err(error),
        }
    }

    /// Load the canonical file, falling back to the legacy Caddy filename.
    pub fn load_with_legacy(
        primary: impl AsRef<Path>,
        legacy: impl AsRef<Path>,
    ) -> io::Result<Self> {
        match fs::read_to_string(primary) {
            Ok(content) => Ok(Self::parse(&content).0),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::load(legacy),
            Err(error) => Err(error),
        }
    }

    pub fn len(&self) -> usize {
        self.users.len()
    }

    pub fn is_empty(&self) -> bool {
        self.users.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.users.get(name).map(String::as_str)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.users.contains_key(name)
    }

    /// Insert or replace a user.
    pub fn upsert(&mut self, name: impl Into<String>, hash: impl Into<String>) {
        self.users.insert(name.into(), hash.into());
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.users.remove(name).is_some()
    }

    pub fn users(&self) -> impl Iterator<Item = User> + '_ {
        self.users.iter().map(|(name, hash)| User {
            name: name.clone(),
            hash: hash.clone(),
        })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> + '_ {
        self.users.keys().map(String::as_str)
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for (name, hash) in &self.users {
            out.push_str(name);
            out.push(' ');
            out.push_str(hash);
            out.push('\n');
        }
        out
    }

    /// Atomically replace the htpasswd file.
    ///
    /// The directory is mounted into the container, so a rename is visible to
    /// the gateway even if the file is bind-mounted read-only.
    pub fn write_atomic(&self, path: impl AsRef<Path>, mode: u32) -> Result<()> {
        let path = path.as_ref();
        write_atomic(path, self.render().as_bytes(), mode)
    }
}

/// Validate a username accepted by the CLI and gateway.
pub fn validate_username(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("username must not be empty");
    }
    if name.len() > MAX_USERNAME_LEN {
        bail!("username must be at most {MAX_USERNAME_LEN} characters");
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '@' | '-'))
    {
        bail!("username may only contain letters, digits, '.', '_', '@' and '-'");
    }
    Ok(())
}

/// Check that a hash looks like a bcrypt hash understood by Caddy/htpasswd.
pub fn validate_hash(hash: &str) -> bool {
    let bytes = hash.as_bytes();
    if bytes.len() != 60 || !hash.starts_with("$2") {
        return false;
    }
    if !matches!(bytes[2], b'a' | b'b' | b'y') || bytes[3] != b'$' {
        return false;
    }
    if !bytes[4].is_ascii_digit() || !bytes[5].is_ascii_digit() || bytes[6] != b'$' {
        return false;
    }
    let cost = (bytes[4] - b'0') as u32 * 10 + (bytes[5] - b'0') as u32;
    if !(4..=31).contains(&cost) {
        return false;
    }
    bytes[7..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'/'))
}

/// Generate a bcrypt hash for a new password.
pub fn hash_password(password: &[u8], cost: u32) -> Result<String> {
    if password.is_empty() {
        bail!("password must not be empty");
    }
    if password.len() > MAX_PASSWORD_BYTES {
        bail!("password must be at most {MAX_PASSWORD_BYTES} bytes");
    }
    if !(4..=31).contains(&cost) {
        bail!("bcrypt cost must be between 4 and 31");
    }
    let hash = bcrypt::hash(password, cost).context("bcrypt hashing failed")?;
    if !validate_hash(&hash) {
        bail!("bcrypt produced an unexpected hash format");
    }
    Ok(hash)
}

/// Verify a password against a bcrypt hash.
pub fn verify_password(password: &[u8], hash: &str) -> Result<bool> {
    if password.len() > MAX_PASSWORD_BYTES {
        return Ok(false);
    }
    let normalized = normalize_hash(hash);
    if !validate_hash(&normalized) {
        return Ok(false);
    }
    bcrypt::verify(password, normalized.as_ref()).context("bcrypt verification failed")
}

fn normalize_hash(hash: &str) -> Cow<'_, str> {
    // The Rust bcrypt crate understands $2a$ and $2b$; PHP/Caddy may emit $2y$.
    if let Some(rest) = hash.strip_prefix("$2y$") {
        let mut owned = String::with_capacity(hash.len());
        owned.push_str("$2b$");
        owned.push_str(rest);
        Cow::Owned(owned)
    } else {
        Cow::Borrowed(hash)
    }
}

/// Atomically write `contents` to `path`.
pub fn write_atomic(path: impl AsRef<Path>, contents: &[u8], mode: u32) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            bail!("refusing to write through symlink {}", path.display());
        }
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp = NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    temp.write_all(contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    temp.flush()?;
    temp.as_file().sync_all()?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))
            .with_context(|| format!("failed to set mode on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
    }

    #[cfg(unix)]
    {
        temp.persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("failed to replace {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        // Windows cannot reliably rename over an existing file, so keep the
        // operation best-effort.  The auth file is not normally edited on
        // Windows hosts running a Linux container.
        if path.exists() {
            fs::remove_file(path)
                .with_context(|| format!("failed to replace {}", path.display()))?;
        }
        temp.persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("failed to replace {}", path.display()))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }

    Ok(())
}

/// Ensure a file exists with the given mode, without truncating existing data.
pub fn ensure_file(path: impl AsRef<Path>, mode: u32) -> Result<()> {
    let path = path.as_ref();
    if path.exists() {
        return Ok(());
    }
    write_atomic(path, b"", mode)
}

/// Resolve the canonical auth file, migrating the legacy Caddy file if needed.
pub fn canonical_auth_path(runtime_dir: impl AsRef<Path>) -> PathBuf {
    runtime_dir.as_ref().join("auth/users.htpasswd")
}

/// Resolve the legacy Caddy auth file.
pub fn legacy_auth_path(runtime_dir: impl AsRef<Path>) -> PathBuf {
    runtime_dir.as_ref().join("auth/users.caddy")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_sorts_users() {
        let zed = hash_password(b"zed", 4).unwrap();
        let alice = format!("$2a${}", &hash_password(b"alice", 4).unwrap()[4..]);
        let content = format!("# comment\n\nzed {zed}\nalice {alice}\nbroken nope\n");
        let (store, warnings) = UserStore::parse(&content);
        assert_eq!(store.len(), 2);
        assert!(warnings.iter().any(|warning| warning.contains("broken")));
        let names: Vec<_> = store.names().collect();
        assert_eq!(names, vec!["alice", "zed"]);
    }

    #[test]
    fn hashes_and_verifies_passwords() {
        let hash = hash_password(b"correct horse battery staple", 4).unwrap();
        assert!(validate_hash(&hash));
        assert!(verify_password(b"correct horse battery staple", &hash).unwrap());
        assert!(!verify_password(b"wrong", &hash).unwrap());
    }

    #[test]
    fn accepts_caddy_prefixes() {
        let hash = hash_password(b"secret", 4).unwrap();
        let as_2a = format!("$2a${}", &hash[4..]);
        let as_2y = format!("$2y${}", &hash[4..]);
        assert!(verify_password(b"secret", &as_2a).unwrap());
        assert!(verify_password(b"secret", &as_2y).unwrap());
    }

    #[test]
    fn rejects_bad_usernames_and_long_passwords() {
        assert!(validate_username("alice").is_ok());
        assert!(validate_username("bad name").is_err());
        assert!(validate_username("").is_err());
        assert!(hash_password(&[b'x'; 73], 4).is_err());
    }

    #[test]
    fn writes_atomic_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.htpasswd");
        let mut store = UserStore::new();
        store.upsert("alice", hash_password(b"secret", 4).unwrap());
        store.write_atomic(&path, 0o600).unwrap();
        let loaded = UserStore::load(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains("alice"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
