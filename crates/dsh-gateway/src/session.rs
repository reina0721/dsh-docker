// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tracing::{debug, info, warn};

pub const SESSION_COOKIE_NAME: &str = "dsh_session";
pub const REMEMBER_DURATION_SECS: u64 = 30 * 24 * 60 * 60; // 30 days
pub const STANDARD_DURATION_SECS: u64 = 7 * 24 * 60 * 60; // 7 days
pub const SECRET_KEY_LEN: usize = 32;

/// Payload encoded inside the session token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionClaims {
    pub sub: String,
    pub exp: u64,
    pub created_at: u64,
}

#[derive(Clone)]
pub struct SessionManager {
    secret_key: [u8; SECRET_KEY_LEN],
    key_path: PathBuf,
}

impl SessionManager {
    /// Load existing persistent session key or generate and save a new one.
    pub fn new(auth_dir: &Path) -> Result<Self> {
        let mut candidate_paths = vec![auth_dir.join("session_secret.key")];
        if let Ok(dsh_home) = std::env::var("DSH_HOME") {
            let p = PathBuf::from(dsh_home).join("session_secret.key");
            if !candidate_paths.contains(&p) {
                candidate_paths.push(p);
            }
        }
        let fallback_dsh = PathBuf::from("/home/node/.dsh/session_secret.key");
        if !candidate_paths.contains(&fallback_dsh) {
            candidate_paths.push(fallback_dsh);
        }

        // 1. Try to load an existing key from any candidate path
        for path in &candidate_paths {
            if path.exists() {
                if let Ok(key) = Self::load_key(path) {
                    debug!(path = %path.display(), "loaded persistent session key");
                    return Ok(Self {
                        secret_key: key,
                        key_path: path.clone(),
                    });
                }
            }
        }

        // 2. Generate a new key and attempt to persist to the first writable candidate path
        let key = Self::generate_key()?;
        let mut chosen_path = candidate_paths[0].clone();
        let mut saved = false;

        for path in &candidate_paths {
            if let Some(parent) = path.parent() {
                if !parent.exists() {
                    let _ = fs::create_dir_all(parent);
                }
            }
            if Self::save_key(path, &key).is_ok() {
                info!(path = %path.display(), "saved persistent session key (30-day session ready)");
                chosen_path = path.clone();
                saved = true;
                break;
            }
        }

        if !saved {
            warn!(
                primary_path = %candidate_paths[0].display(),
                "could not persist session key to disk; sessions will reset on process restart"
            );
        }

        Ok(Self {
            secret_key: key,
            key_path: chosen_path,
        })
    }

    /// Construct a SessionManager with an explicit in-memory key (for tests).
    pub fn from_key(key: [u8; SECRET_KEY_LEN]) -> Self {
        Self {
            secret_key: key,
            key_path: PathBuf::from("in-memory"),
        }
    }

    pub fn key_path(&self) -> &Path {
        &self.key_path
    }

    fn generate_key() -> Result<[u8; SECRET_KEY_LEN]> {
        let mut key = [0u8; SECRET_KEY_LEN];
        getrandom::getrandom(&mut key)
            .map_err(|e| anyhow::anyhow!("failed to generate secure random key: {e}"))?;
        Ok(key)
    }

    fn load_key(path: &Path) -> Result<[u8; SECRET_KEY_LEN]> {
        let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        if bytes.len() != SECRET_KEY_LEN {
            bail!(
                "invalid session key length in {} (expected {}, got {})",
                path.display(),
                SECRET_KEY_LEN,
                bytes.len()
            );
        }
        let mut key = [0u8; SECRET_KEY_LEN];
        key.copy_from_slice(&bytes);
        Ok(key)
    }

    fn save_key(path: &Path, key: &[u8; SECRET_KEY_LEN]) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        let temp_path = path.with_extension("tmp");
        let mut file = File::create(&temp_path)
            .with_context(|| format!("failed to create temp key file {}", temp_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
        }

        file.write_all(key).context("failed to write key bytes")?;
        file.sync_all().context("failed to sync key file")?;
        drop(file);

        fs::rename(&temp_path, path).with_context(|| {
            format!(
                "failed to rename {} -> {}",
                temp_path.display(),
                path.display()
            )
        })?;
        Ok(())
    }

    /// Sign a new session token for the user.
    pub fn create_token(&self, username: &str, remember: bool) -> Result<(String, u64)> {
        let now = now_secs();
        let duration = if remember {
            REMEMBER_DURATION_SECS
        } else {
            STANDARD_DURATION_SECS
        };
        let exp = now + duration;

        let claims = SessionClaims {
            sub: username.to_string(),
            exp,
            created_at: now,
        };

        let payload_json = serde_json::to_string(&claims)?;
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
        let signature = hmac_sha256(&self.secret_key, payload_b64.as_bytes());
        let signature_b64 = URL_SAFE_NO_PAD.encode(signature);

        let token = format!("{payload_b64}.{signature_b64}");
        Ok((token, duration))
    }

    /// Verify a session token. Returns claims if valid and not expired.
    pub fn verify_token(&self, token: &str) -> Result<SessionClaims> {
        let (payload_b64, sig_b64) = token
            .split_once('.')
            .context("invalid token format; missing signature delimiter")?;

        let expected_sig = hmac_sha256(&self.secret_key, payload_b64.as_bytes());
        let provided_sig = URL_SAFE_NO_PAD
            .decode(sig_b64)
            .context("invalid signature base64")?;

        if expected_sig.ct_eq(&provided_sig).unwrap_u8() != 1 {
            bail!("session token signature mismatch");
        }

        let payload_bytes = URL_SAFE_NO_PAD
            .decode(payload_b64)
            .context("invalid payload base64")?;

        let claims: SessionClaims =
            serde_json::from_slice(&payload_bytes).context("malformed session claims JSON")?;

        if claims.exp < now_secs() {
            bail!("session token expired");
        }

        Ok(claims)
    }

    /// Format a `Set-Cookie` header value for logging in.
    pub fn format_cookie(token: &str, max_age: u64) -> String {
        format!("{SESSION_COOKIE_NAME}={token}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax")
    }

    /// Format a `Set-Cookie` header value for logging out.
    pub fn format_logout_cookie() -> String {
        format!(
            "{SESSION_COOKIE_NAME}=; Path=/; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; SameSite=Lax"
        )
    }

    /// Extract `dsh_session` cookie value from a `Cookie` header string.
    pub fn extract_cookie(cookie_header: &str) -> Option<&str> {
        for item in cookie_header.split(';') {
            let trimmed = item.trim();
            if let Some((name, value)) = trimmed.split_once('=') {
                if name.trim() == SESSION_COOKIE_NAME {
                    let val = value.trim();
                    if !val.is_empty() {
                        return Some(val);
                    }
                }
            }
        }
        None
    }
}

/// Compute HMAC-SHA256.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0u8; BLOCK_SIZE];

    if key.len() > BLOCK_SIZE {
        let hash = Sha256::digest(key);
        key_block[..hash.len()].copy_from_slice(&hash);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0u8; BLOCK_SIZE];
    let mut opad = [0u8; BLOCK_SIZE];

    for i in 0..BLOCK_SIZE {
        ipad[i] = key_block[i] ^ 0x36;
        opad[i] = key_block[i] ^ 0x5c;
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    let outer_hash = outer.finalize();

    let mut output = [0u8; 32];
    output.copy_from_slice(&outer_hash);
    output
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_known_vector() {
        // RFC 4231 test case 1
        let key = [0x0bu8; 20];
        let data = b"Hi There";
        let mac = hmac_sha256(&key, data);
        let expected =
            hex::decode("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7")
                .unwrap();
        assert_eq!(&mac[..], expected.as_slice());
    }

    #[test]
    fn session_token_lifecycle() {
        let key = [42u8; 32];
        let manager = SessionManager::from_key(key);

        let (token, duration) = manager.create_token("alice", true).unwrap();
        assert_eq!(duration, REMEMBER_DURATION_SECS);

        let claims = manager.verify_token(&token).unwrap();
        assert_eq!(claims.sub, "alice");
        assert!(claims.exp > claims.created_at);

        // Tampered token fails
        let tampered = format!("x{token}");
        assert!(manager.verify_token(&tampered).is_err());

        // Cookie extraction
        let cookie_hdr = format!("foo=bar; {SESSION_COOKIE_NAME}={token}; baz=qux");
        assert_eq!(
            SessionManager::extract_cookie(&cookie_hdr),
            Some(token.as_str())
        );
    }

    #[test]
    fn key_persistence() {
        let temp = tempfile::tempdir().unwrap();
        let auth_dir = temp.path();

        let mgr1 = SessionManager::new(auth_dir).unwrap();
        let (token, _) = mgr1.create_token("bob", false).unwrap();

        // New manager reloads same key from disk
        let mgr2 = SessionManager::new(auth_dir).unwrap();
        assert_eq!(mgr1.secret_key, mgr2.secret_key);

        let claims = mgr2.verify_token(&token).unwrap();
        assert_eq!(claims.sub, "bob");
    }
}
