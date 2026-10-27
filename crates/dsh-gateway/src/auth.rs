// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};

use dsh_auth::{UserStore, verify_password};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;

const CACHE_TTL: Duration = Duration::from_secs(600);
const CACHE_MAX: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthOutcome {
    Allowed,
    Denied,
    /// No usable users are configured.  The gateway must fail closed.
    NoUsers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileMeta {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    ino: u64,
}

#[derive(Debug)]
struct AuthState {
    store: UserStore,
    meta: Option<FileMeta>,
    last_check: Instant,
    generation: u64,
}

/// Authentication database with bcrypt verification caching and hot reload.
///
/// The cache key is a digest of the credential pair, never the plaintext.  A
/// single async mutex serialises first-time verifications (single flight), which
/// matters because bcrypt at cost 14 takes roughly a second on small VPSes.
pub struct AuthStore {
    primary: PathBuf,
    legacy: PathBuf,
    reload_interval: Duration,
    state: Mutex<AuthState>,
    cache: AsyncMutex<HashMap<[u8; 32], Instant>>,
    verify_lock: AsyncMutex<()>,
}

impl AuthStore {
    pub fn new(
        primary: impl Into<PathBuf>,
        legacy: impl Into<PathBuf>,
        reload_interval: Duration,
    ) -> Self {
        let store = Self {
            primary: primary.into(),
            legacy: legacy.into(),
            reload_interval,
            state: Mutex::new(AuthState {
                store: UserStore::new(),
                meta: None,
                last_check: Instant::now() - reload_interval,
                generation: 0,
            }),
            cache: AsyncMutex::new(HashMap::new()),
            verify_lock: AsyncMutex::new(()),
        };
        store.reload_if_changed(true);
        store
    }

    pub fn user_count(&self) -> usize {
        self.state.lock().expect("auth state poisoned").store.len()
    }

    pub fn auth_path(&self) -> &PathBuf {
        &self.primary
    }

    /// Re-read the database if its metadata changed.  A malformed or unreadable
    /// file keeps the last known-good database; a valid file with zero users
    /// intentionally fails closed.
    pub fn reload_if_changed(&self, force: bool) {
        let now = Instant::now();
        {
            let state = self.state.lock().expect("auth state poisoned");
            if !force && now.duration_since(state.last_check) < self.reload_interval {
                return;
            }
        }

        let (path, meta) = match metadata_of(&self.primary) {
            Ok(Some(meta)) => (self.primary.clone(), Some(meta)),
            Ok(None) => match metadata_of(&self.legacy) {
                Ok(Some(meta)) => (self.legacy.clone(), Some(meta)),
                Ok(None) => (self.primary.clone(), None),
                Err(error) => {
                    tracing::warn!(error = %error, "failed to inspect legacy auth file");
                    return;
                }
            },
            Err(error) => {
                tracing::warn!(error = %error, path = %self.primary.display(), "failed to inspect auth file");
                return;
            }
        };

        let mut state = self.state.lock().expect("auth state poisoned");
        state.last_check = now;
        if !force && state.meta == meta {
            return;
        }

        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                tracing::warn!(error = %error, path = %path.display(), "failed to read auth file; keeping last known-good credentials");
                return;
            }
        };

        let (store, warnings) = UserStore::parse(&content);
        for warning in warnings {
            tracing::warn!(path = %path.display(), "{warning}");
        }
        if store.is_empty() {
            tracing::warn!(
                path = %path.display(),
                "authentication database has no valid users; gateway will reject all requests"
            );
        } else {
            tracing::info!(
                path = %path.display(),
                users = store.len(),
                "loaded authentication database"
            );
        }
        state.store = store;
        state.meta = meta;
        state.generation = state.generation.wrapping_add(1);
        drop(state);

        // A credential that was valid for the previous file must not survive a
        // password change or user removal.
        if let Ok(mut cache) = self.cache.try_lock() {
            cache.clear();
        } else {
            // A concurrent authorization is holding the cache briefly; clearing
            // via a blocking task is not worth the complexity.  The generation
            // is part of the key, so old entries cannot authenticate a request
            // against the new database anyway.
        }
    }

    pub async fn authorize(&self, username: &str, password: &[u8]) -> AuthOutcome {
        self.reload_if_changed(false);

        let (hash, generation, user_count) = {
            let state = self.state.lock().expect("auth state poisoned");
            (
                state.store.get(username).map(ToOwned::to_owned),
                state.generation,
                state.store.len(),
            )
        };

        if user_count == 0 {
            return AuthOutcome::NoUsers;
        }

        let key = cache_key(generation, username, password);
        if self.cache_hit(&key).await {
            return AuthOutcome::Allowed;
        }

        // Single flight: concurrent requests with the same (or different)
        // credentials only run one bcrypt verification at a time.  Re-check the
        // cache after waiting so parallel asset requests become cache hits.
        let _guard = self.verify_lock.lock().await;
        if self.cache_hit(&key).await {
            return AuthOutcome::Allowed;
        }

        let Some(hash) = hash else {
            return AuthOutcome::Denied;
        };
        let password = password.to_vec();
        let verified = tokio::task::spawn_blocking(move || verify_password(&password, &hash))
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(error = %error, "bcrypt verification task failed");
                Ok(false)
            })
            .unwrap_or_else(|error| {
                tracing::warn!(error = %error, "bcrypt verification failed");
                false
            });

        if verified {
            self.cache_insert(key).await;
            AuthOutcome::Allowed
        } else {
            AuthOutcome::Denied
        }
    }

    async fn cache_hit(&self, key: &[u8; 32]) -> bool {
        let mut cache = self.cache.lock().await;
        match cache.get(key) {
            Some(expiry) if *expiry > Instant::now() => true,
            Some(_) => {
                cache.remove(key);
                false
            }
            None => false,
        }
    }

    async fn cache_insert(&self, key: [u8; 32]) {
        let mut cache = self.cache.lock().await;
        if cache.len() >= CACHE_MAX {
            let now = Instant::now();
            cache.retain(|_, expiry| *expiry > now);
            while cache.len() >= CACHE_MAX {
                if let Some(key) = cache.keys().next().copied() {
                    cache.remove(&key);
                } else {
                    break;
                }
            }
        }
        cache.insert(key, Instant::now() + CACHE_TTL);
    }

    #[cfg(test)]
    pub fn cache_len(&self) -> usize {
        self.cache.try_lock().map(|cache| cache.len()).unwrap_or(0)
    }
}

fn cache_key(generation: u64, username: &str, password: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(generation.to_le_bytes());
    hasher.update(username.as_bytes());
    hasher.update([0]);
    hasher.update(password);
    hasher.finalize().into()
}

fn metadata_of(path: &PathBuf) -> std::io::Result<Option<FileMeta>> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(FileMeta {
        len: meta.len(),
        modified: meta.modified().ok(),
        #[cfg(unix)]
        ino: {
            use std::os::unix::fs::MetadataExt;
            meta.ino()
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_auth(path: &std::path::Path, name: &str, password: &str) {
        let hash = dsh_auth::hash_password(password.as_bytes(), 4).unwrap();
        let mut store = UserStore::new();
        store.upsert(name, hash);
        store.write_atomic(path, 0o600).unwrap();
    }

    #[tokio::test]
    async fn authorizes_and_caches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.htpasswd");
        write_auth(&path, "alice", "secret");
        let store = AuthStore::new(&path, dir.path().join("users.caddy"), Duration::ZERO);
        assert_eq!(store.user_count(), 1);
        assert_eq!(
            store.authorize("alice", b"secret").await,
            AuthOutcome::Allowed
        );
        assert_eq!(
            store.authorize("alice", b"secret").await,
            AuthOutcome::Allowed
        );
        assert_eq!(store.cache_len(), 1);
        assert_eq!(
            store.authorize("alice", b"wrong").await,
            AuthOutcome::Denied
        );
        assert_eq!(store.authorize("bob", b"secret").await, AuthOutcome::Denied);
    }

    #[tokio::test]
    async fn reload_clears_old_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.htpasswd");
        write_auth(&path, "alice", "secret");
        let store = AuthStore::new(&path, dir.path().join("users.caddy"), Duration::ZERO);
        assert_eq!(
            store.authorize("alice", b"secret").await,
            AuthOutcome::Allowed
        );
        write_auth(&path, "alice", "changed");
        store.reload_if_changed(true);
        assert_eq!(
            store.authorize("alice", b"secret").await,
            AuthOutcome::Denied
        );
        assert_eq!(
            store.authorize("alice", b"changed").await,
            AuthOutcome::Allowed
        );
    }

    #[tokio::test]
    async fn empty_file_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.htpasswd");
        std::fs::write(&path, b"").unwrap();
        let store = AuthStore::new(&path, dir.path().join("users.caddy"), Duration::ZERO);
        assert_eq!(
            store.authorize("alice", b"secret").await,
            AuthOutcome::NoUsers
        );
    }

    #[tokio::test]
    async fn falls_back_to_legacy_file() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("users.caddy");
        write_auth(&legacy, "alice", "secret");
        let store = AuthStore::new(dir.path().join("users.htpasswd"), &legacy, Duration::ZERO);
        assert_eq!(
            store.authorize("alice", b"secret").await,
            AuthOutcome::Allowed
        );
    }
}
