// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    collections::BTreeMap,
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use dsh_auth::write_atomic;
use dsh_config::Config;
use serde::Serialize;

/// SSH keypair metadata shown by `dshctl ssh list`.
///
/// Private key material is never included.  `public` is the OpenSSH public line.
#[derive(Debug, Clone, Serialize)]
pub struct KeyInfo {
    pub name: String,
    pub private: bool,
    pub public: Option<String>,
    pub fingerprint: Option<String>,
    pub key_type: Option<String>,
}

pub struct SshManager {
    dir: PathBuf,
}

impl SshManager {
    pub fn new(config: &Config) -> Self {
        Self {
            dir: config.home_dir().join("ssh"),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn list(&self) -> Result<Vec<KeyInfo>> {
        let mut keys: BTreeMap<String, KeyInfo> = BTreeMap::new();
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", self.dir.display()));
            }
        };

        for entry in entries {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if !file_type.is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if ignored_name(&file_name) {
                continue;
            }
            if let Some(name) = file_name.strip_suffix(".pub") {
                if validate_name(name).is_err() {
                    continue;
                }
                let public = read_public(&entry.path()).ok();
                let info = keys.entry(name.to_owned()).or_insert_with(|| KeyInfo {
                    name: name.to_owned(),
                    private: false,
                    public: None,
                    fingerprint: None,
                    key_type: None,
                });
                info.public = public.clone();
                info.key_type = public.as_deref().and_then(public_key_type);
                info.fingerprint = fingerprint(&entry.path());
            } else if validate_name(&file_name).is_ok() {
                let public_path = self.public_path(&file_name);
                let public = read_public(&public_path).ok();
                let info = keys.entry(file_name.clone()).or_insert_with(|| KeyInfo {
                    name: file_name,
                    private: false,
                    public: None,
                    fingerprint: None,
                    key_type: None,
                });
                info.private = true;
                if public.is_some() {
                    info.public = public.clone();
                    info.key_type = public.as_deref().and_then(public_key_type);
                    info.fingerprint = fingerprint(&public_path);
                }
            }
        }
        Ok(keys.into_values().collect())
    }

    pub fn generate(
        &self,
        name: Option<&str>,
        key_type: &str,
        comment: Option<&str>,
        no_passphrase: bool,
        force: bool,
    ) -> Result<KeyInfo> {
        let key_type = normalize_key_type(key_type)?;
        let default_name = match key_type {
            "ed25519" => "id_ed25519",
            "rsa" => "id_rsa",
            "ecdsa" => "id_ecdsa",
            _ => "id_ed25519",
        };
        let name = name.unwrap_or(default_name);
        validate_name(name)?;
        if let Some(comment) = comment {
            if comment.contains(['\r', '\n']) {
                bail!("key comment must be a single line");
            }
        }
        self.ensure_dir()?;

        let private = self.private_path(name);
        let public = self.public_path(name);
        if private.exists() || public.exists() {
            if !force {
                bail!("key '{}' already exists; use --force to replace it", name);
            }
            remove_if_exists(&private)?;
            remove_if_exists(&public)?;
        }

        if !no_passphrase && !io::stdin().is_terminal() {
            bail!(
                "generating an encrypted key requires a terminal for the passphrase prompt; \
                 use --no-passphrase for unattended generation"
            );
        }

        let mut command = Command::new("ssh-keygen");
        command
            .arg("-t")
            .arg(key_type)
            .arg("-f")
            .arg(&private)
            .arg("-C")
            .arg(comment.unwrap_or("dshctl"));
        if no_passphrase {
            command.arg("-N").arg("");
        }
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let status = command.status().context(
            "failed to run ssh-keygen; install the OpenSSH client or add the key with `dshctl ssh import`",
        )?;
        if !status.success() {
            bail!("ssh-keygen exited with {status}");
        }
        self.set_key_modes(&private, &public)?;
        self.describe(name)
    }

    pub fn import(
        &self,
        private_key: &Path,
        public_key: Option<&Path>,
        name: Option<&str>,
        force: bool,
    ) -> Result<KeyInfo> {
        let private_key = expand_home(private_key)?;
        if !private_key.is_file() {
            bail!("private key not found: {}", private_key.display());
        }
        if fs::symlink_metadata(&private_key)?.file_type().is_symlink() {
            bail!("refusing to import a symlink: {}", private_key.display());
        }
        let bytes = fs::read(&private_key)
            .with_context(|| format!("failed to read {}", private_key.display()))?;
        if bytes.len() > 1024 * 1024 {
            bail!("private key is unexpectedly large (limit 1 MiB)");
        }
        let text = std::str::from_utf8(&bytes).context("private key is not UTF-8 text")?;
        if !text.contains("PRIVATE KEY") {
            bail!("file does not look like a PEM/OpenSSH private key");
        }

        let name = name
            .map(ToOwned::to_owned)
            .or_else(|| {
                private_key
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .context("could not derive a key name; pass --name")?;
        validate_name(&name)?;
        self.ensure_dir()?;

        let target = self.private_path(&name);
        let target_public = self.public_path(&name);
        if target.exists() || target_public.exists() {
            if !force {
                bail!("key '{}' already exists; use --force to replace it", name);
            }
            remove_if_exists(&target)?;
            remove_if_exists(&target_public)?;
        }
        write_atomic(&target, &bytes, 0o600)
            .with_context(|| format!("failed to write {}", target.display()))?;

        let public_source = public_key
            .map(expand_home)
            .transpose()?
            .filter(|path| path.is_file())
            .or_else(|| {
                let adjacent = PathBuf::from(format!("{}.pub", private_key.display()));
                adjacent.is_file().then_some(adjacent)
            });

        if let Some(source) = public_source {
            let public = read_public(&source)?;
            write_atomic(&target_public, format!("{public}\n").as_bytes(), 0o644)
                .with_context(|| format!("failed to write {}", target_public.display()))?;
        } else if let Some(public) = derive_public(&target) {
            write_atomic(&target_public, format!("{public}\n").as_bytes(), 0o644)
                .with_context(|| format!("failed to write {}", target_public.display()))?;
        } else {
            eprintln!(
                "warning: could not derive the public key (it may be passphrase-protected); \
                 pass --public-key or run `ssh-keygen -y -f {}`",
                target.display()
            );
        }

        self.set_key_modes(&target, &target_public)?;
        self.describe(&name)
    }

    pub fn remove(&self, name: &str, yes: bool) -> Result<()> {
        validate_name(name)?;
        let private = self.private_path(name);
        let public = self.public_path(name);
        if !private.exists() && !public.exists() {
            bail!("no key named '{}'", name);
        }
        if !yes && !confirm(&format!("Remove SSH key '{name}'? [y/N] "))? {
            println!("Aborted.");
            return Ok(());
        }
        remove_if_exists(&private)?;
        remove_if_exists(&public)?;
        println!("Removed SSH key '{name}'.");
        Ok(())
    }

    pub fn test(&self, target: &str, key: Option<&str>) -> Result<()> {
        let target = match target {
            "github" | "github.com" => "git@github.com".to_owned(),
            "gitlab" | "gitlab.com" => "git@gitlab.com".to_owned(),
            "gitee" | "gitee.com" => "git@gitee.com".to_owned(),
            other if other.contains('@') => other.to_owned(),
            other => format!("git@{other}"),
        };

        let key = match key {
            Some(name) => {
                validate_name(name)?;
                let path = self.private_path(name);
                if !path.is_file() {
                    bail!("no private key named '{}'", name);
                }
                Some(path)
            }
            None => self
                .list()?
                .into_iter()
                .find(|key| key.private)
                .map(|key| self.private_path(&key.name)),
        };

        let mut command = Command::new("ssh");
        command
            .arg("-T")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg("-o")
            .arg("ConnectTimeout=10");
        if let Some(path) = &key {
            command.arg("-i").arg(path);
            command.arg("-o").arg("IdentitiesOnly=yes");
        }
        command
            .arg(&target)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let status = command
            .status()
            .context("failed to run ssh; install the OpenSSH client")?;
        if status.success() {
            println!("SSH connection to {target} succeeded.");
            return Ok(());
        }
        if status.code() == Some(1)
            && (target.contains("github") || target.contains("gitlab") || target.contains("gitee"))
        {
            eprintln!(
                "Note: {target} exits with status 1 even after a successful greeting. \
                 Read the message above; `successfully authenticated` means it worked."
            );
            return Ok(());
        }
        bail!("ssh exited with {status}");
    }

    pub fn describe(&self, name: &str) -> Result<KeyInfo> {
        let private = self.private_path(name);
        let public_path = self.public_path(name);
        let public = read_public(&public_path).ok();
        Ok(KeyInfo {
            name: name.to_owned(),
            private: private.is_file(),
            public: public.clone(),
            fingerprint: fingerprint(&public_path),
            key_type: public.as_deref().and_then(public_key_type),
        })
    }

    pub fn private_path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn public_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.pub"))
    }

    fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("failed to create {}", self.dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700));
        }
        Ok(())
    }

    fn set_key_modes(&self, private: &Path, public: &Path) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(private, fs::Permissions::from_mode(0o600));
            if public.exists() {
                let _ = fs::set_permissions(public, fs::Permissions::from_mode(0o644));
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (private, public);
        }
        Ok(())
    }
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        bail!("key name must be 1-64 characters");
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        bail!("key name may only contain letters, digits, '.', '_' and '-'");
    }
    if matches!(name, "." | "..") || ignored_name(name) || name.ends_with(".pub") {
        bail!("'{name}' is reserved; use the key name without .pub");
    }
    Ok(())
}

fn ignored_name(name: &str) -> bool {
    matches!(
        name,
        "known_hosts"
            | "known_hosts.old"
            | "config"
            | "authorized_keys"
            | "authorized_keys2"
            | "environment"
            | "rc"
    )
}

fn normalize_key_type(value: &str) -> Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "ed25519" => Ok("ed25519"),
        "rsa" => Ok("rsa"),
        "ecdsa" => Ok("ecdsa"),
        other => bail!("unsupported key type {other:?}; use ed25519, rsa, or ecdsa"),
    }
}

fn read_public(path: &Path) -> Result<String> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .context("public key file is empty")?;
    if !is_public_key(line) {
        bail!("{} does not contain an OpenSSH public key", path.display());
    }
    Ok(line.to_owned())
}

fn is_public_key(line: &str) -> bool {
    line.starts_with("ssh-")
        || line.starts_with("ecdsa-")
        || line.starts_with("sk-ssh-")
        || line.starts_with("sk-ecdsa-")
}

fn public_key_type(line: &str) -> Option<String> {
    line.split_whitespace().next().map(ToOwned::to_owned)
}

fn fingerprint(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let output = Command::new("ssh-keygen")
        .arg("-l")
        .arg("-f")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn derive_public(private: &Path) -> Option<String> {
    let output = Command::new("ssh-keygen")
        .arg("-y")
        .arg("-f")
        .arg(private)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    is_public_key(&text).then_some(text)
}

fn expand_home(path: &Path) -> Result<PathBuf> {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is not set")?;
        Ok(home.join(rest))
    } else {
        Ok(path.to_path_buf())
    }
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing to remove symlink {}", path.display())
        }
        Ok(_) => {
            fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    }
    Ok(())
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_names() {
        assert!(validate_name("id_ed25519").is_ok());
        assert!(validate_name("work-2026").is_ok());
        assert!(validate_name("../escape").is_err());
        assert!(validate_name("known_hosts").is_err());
        assert!(validate_name("foo.pub").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn imports_and_lists_keys() {
        let root = tempfile::tempdir().unwrap();
        let config = Config::from_map(root.path(), Default::default()).unwrap();
        let manager = SshManager::new(&config);
        let source = root.path().join("source_key");
        fs::write(
            &source,
            "-----BEGIN OPENSSH PRIVATE KEY-----\ntest\n-----END OPENSSH PRIVATE KEY-----\n",
        )
        .unwrap();
        let public = root.path().join("source_key.pub");
        fs::write(
            &public,
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest dshctl\n",
        )
        .unwrap();

        let info = manager
            .import(&source, Some(&public), Some("work"), false)
            .unwrap();
        assert_eq!(info.name, "work");
        assert_eq!(info.key_type.as_deref(), Some("ssh-ed25519"));
        let listed = manager.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].private);

        manager.remove("work", true).unwrap();
        assert!(manager.list().unwrap().is_empty());
    }

    #[test]
    fn rejects_non_key_material() {
        let root = tempfile::tempdir().unwrap();
        let config = Config::from_map(root.path(), Default::default()).unwrap();
        let manager = SshManager::new(&config);
        let source = root.path().join("not-a-key");
        fs::write(&source, "hello").unwrap();
        assert!(manager.import(&source, None, Some("bad"), false).is_err());
    }
}
