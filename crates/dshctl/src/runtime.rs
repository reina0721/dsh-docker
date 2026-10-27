// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use dsh_auth::UserStore;
use dsh_config::Config;

use crate::templates;

#[derive(Debug, Clone, Copy, Default)]
pub struct InitOptions {
    pub force: bool,
    pub refresh: bool,
    pub quiet: bool,
}

/// Create or refresh the deployment directory and its persistent runtime tree.
pub fn init(config: &Config, options: InitOptions) -> Result<()> {
    fs::create_dir_all(&config.project_dir)
        .with_context(|| format!("failed to create {}", config.project_dir.display()))?;

    let compose = config.project_dir.join("compose.yaml");
    let legacy_compose = fs::read_to_string(&compose)
        .map(|text| {
            text.contains("Caddyfile")
                || text.contains("caddy:")
                || text.contains("start-dsh-web")
                || text.contains("dsh-web-proxy")
                || text.contains("\n  gateway:")
        })
        .unwrap_or(false);
    if options.force || options.refresh || legacy_compose || !compose.exists() {
        if compose.exists() {
            let backup = backup_path(&compose);
            fs::copy(&compose, &backup)
                .with_context(|| format!("failed to back up {}", compose.display()))?;
        }
        write_file(&compose, templates::COMPOSE_YAML.as_bytes(), 0o644)?;
    }

    let env_example = config.project_dir.join(".env.example");
    if legacy_compose && env_example.exists() {
        let backup = backup_path(&env_example);
        fs::copy(&env_example, &backup)
            .with_context(|| format!("failed to back up {}", env_example.display()))?;
    }
    if legacy_compose || !env_example.exists() {
        write_file(&env_example, templates::ENV_EXAMPLE.as_bytes(), 0o644)?;
    }

    let env_file = config.project_dir.join(".env");
    if !env_file.exists() {
        write_file(&env_file, templates::ENV_EXAMPLE.as_bytes(), 0o600)?;
    }

    create_runtime_tree(config)?;
    migrate_legacy_auth(config)?;

    apply_permissions(config)?;
    chown_tree_if_root(config)?;
    warn_owner_mismatch(config);

    if !options.quiet {
        println!("Repository runtime: {}", config.runtime_dir.display());
        println!("Compose file     : {}", compose.display());
        println!("Environment      : {}", env_file.display());
    }
    Ok(())
}

fn create_runtime_tree(config: &Config) -> Result<()> {
    let dirs = [
        config.workspace_dir().join("projects"),
        config.dsh_home(),
        config.auth_dir(),
        config.home_dir().join("ssh"),
        config.home_dir().join("gnupg"),
        config.home_dir().join("aws"),
        config.home_dir().join("kube"),
        config.home_dir().join("docker"),
        config.home_dir().join("config"),
        config.home_dir().join("local/share"),
        config.home_dir().join("local/state"),
        config.home_dir().join("git"),
        config.home_dir().join("shell"),
        config.home_dir().join("npm"),
        config.home_dir().join("cargo"),
    ];
    for dir in dirs {
        fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    }

    let files = [
        config.auth_file(),
        config.home_dir().join("git/config"),
        config.home_dir().join("git/credentials"),
        config.home_dir().join("shell/bash_history"),
        config.home_dir().join("shell/python_history"),
        config.home_dir().join("npm/npmrc"),
        config.home_dir().join("netrc"),
        config.home_dir().join("pypirc"),
        config.home_dir().join("cargo/config.toml"),
        config.home_dir().join("cargo/credentials.toml"),
    ];
    for file in files {
        ensure_file(&file)?;
    }
    Ok(())
}

/// Copy a legacy Caddy htpasswd file to the canonical v2 name.
pub fn migrate_legacy_auth(config: &Config) -> Result<bool> {
    let canonical = config.auth_file();
    let legacy = config.legacy_auth_file();
    if !legacy.is_file() {
        return Ok(false);
    }
    let canonical_empty = fs::metadata(&canonical)
        .map(|meta| meta.len() == 0)
        .unwrap_or(true);
    if canonical.is_file() && !canonical_empty {
        return Ok(false);
    }
    let store = UserStore::load(&legacy)
        .with_context(|| format!("failed to read legacy auth file {}", legacy.display()))?;
    store
        .write_atomic(&canonical, 0o644)
        .with_context(|| format!("failed to migrate auth file to {}", canonical.display()))?;
    Ok(true)
}

pub fn load_users(config: &Config) -> Result<UserStore> {
    UserStore::load_with_legacy(config.auth_file(), config.legacy_auth_file()).with_context(|| {
        format!(
            "failed to load authentication database {}",
            config.auth_file().display()
        )
    })
}

pub fn write_users(config: &Config, store: &UserStore) -> Result<()> {
    fs::create_dir_all(config.auth_dir())
        .with_context(|| format!("failed to create {}", config.auth_dir().display()))?;
    store
        .write_atomic(config.auth_file(), 0o644)
        .with_context(|| format!("failed to write {}", config.auth_file().display()))
}

pub fn has_users(config: &Config) -> Result<bool> {
    Ok(!load_users(config)?.is_empty())
}

pub fn apply_permissions(config: &Config) -> Result<()> {
    set_mode(&config.runtime_dir, 0o750)?;
    set_mode(&config.workspace_dir(), 0o750)?;
    set_mode(&config.dsh_home(), 0o700)?;
    set_mode(&config.auth_dir(), 0o750)?;
    set_mode(&config.home_dir(), 0o700)?;
    set_mode(&config.home_dir().join("ssh"), 0o700)?;
    set_mode(&config.home_dir().join("gnupg"), 0o700)?;
    set_mode(&config.auth_file(), 0o644)?;

    for file in [
        config.home_dir().join("git/config"),
        config.home_dir().join("git/credentials"),
        config.home_dir().join("shell/bash_history"),
        config.home_dir().join("shell/python_history"),
        config.home_dir().join("npm/npmrc"),
        config.home_dir().join("netrc"),
        config.home_dir().join("pypirc"),
        config.home_dir().join("cargo/config.toml"),
        config.home_dir().join("cargo/credentials.toml"),
    ] {
        set_mode(&file, 0o600)?;
    }
    Ok(())
}

pub fn set_mode(path: &Path, mode: u32) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .with_context(|| format!("failed to set mode on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

fn write_file(path: &Path, contents: &[u8], mode: u32) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.sync_all()?;
    set_mode(path, mode)
}

fn ensure_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    Ok(())
}

fn backup_path(path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    path.with_file_name(format!("{name}.bak-{stamp}"))
}

#[cfg(unix)]
fn warn_owner_mismatch(config: &Config) {
    use std::os::unix::fs::MetadataExt;

    if nix::unistd::Uid::effective().is_root() {
        return;
    }
    if let Ok(metadata) = fs::metadata(config.home_dir()) {
        if metadata.uid() != config.runtime_uid {
            eprintln!(
                "warning: {} is owned by uid {}, but the container runs as uid {}; \
                 run `sudo chown -R {}:{} {}` if the container cannot write",
                config.home_dir().display(),
                metadata.uid(),
                config.runtime_uid,
                config.runtime_uid,
                config.runtime_gid,
                config.home_dir().display()
            );
        }
    }
}

#[cfg(not(unix))]
fn warn_owner_mismatch(_config: &Config) {}

#[cfg(unix)]
fn chown_tree_if_root(config: &Config) -> Result<()> {
    use nix::unistd::{Gid, Uid, chown};

    if !Uid::effective().is_root() {
        return Ok(());
    }
    let uid = Uid::from_raw(config.runtime_uid);
    let gid = Gid::from_raw(config.runtime_gid);
    for root in [config.workspace_dir(), config.dsh_home(), config.home_dir()] {
        chown_recursive(&root, uid, gid);
    }
    Ok(())
}

#[cfg(unix)]
fn chown_recursive(path: &Path, uid: nix::unistd::Uid, gid: nix::unistd::Gid) {
    use nix::unistd::chown;

    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return,
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    if let Err(error) = chown(path, Some(uid), Some(gid)) {
        eprintln!("warning: failed to chown {}: {error}", path.display());
    }
    if metadata.is_dir() {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                chown_recursive(&entry.path(), uid, gid);
            }
        }
    }
}

#[cfg(not(unix))]
fn chown_tree_if_root(_config: &Config) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dsh_config::Config;
    use std::collections::BTreeMap;

    fn config(dir: &Path) -> Config {
        Config::from_map(dir, BTreeMap::new()).unwrap()
    }

    #[test]
    fn initializes_runtime_tree() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        init(
            &cfg,
            InitOptions {
                force: true,
                refresh: false,
                quiet: true,
            },
        )
        .unwrap();
        assert!(cfg.auth_file().is_file());
        assert!(cfg.home_dir().join("ssh").is_dir());
        assert!(cfg.project_dir.join("compose.yaml").is_file());
        assert!(cfg.project_dir.join(".env").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&cfg.auth_file()).unwrap().permissions().mode() & 0o777,
                0o644
            );
        }
    }

    #[test]
    fn migrates_legacy_auth() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        fs::create_dir_all(cfg.auth_dir()).unwrap();
        let hash = dsh_auth::hash_password(b"secret", 4).unwrap();
        fs::write(cfg.legacy_auth_file(), format!("alice {hash}\n")).unwrap();
        assert!(migrate_legacy_auth(&cfg).unwrap());
        let store = load_users(&cfg).unwrap();
        assert!(store.contains("alice"));
        assert!(cfg.legacy_auth_file().exists());
    }
}
