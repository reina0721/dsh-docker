// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{fs, path::PathBuf, process::Command};

use anyhow::{Context, Result, bail};
use dsh_config::Config;

use crate::ssh;

pub struct GitConfig {
    path: PathBuf,
    ssh_dir: PathBuf,
}

impl GitConfig {
    pub fn new(config: &Config) -> Self {
        Self {
            path: config.home_dir().join("git/config"),
            ssh_dir: config.home_dir().join("ssh"),
        }
    }

    pub fn setup(
        &self,
        name: Option<&str>,
        email: Option<&str>,
        ssh_key: Option<&str>,
    ) -> Result<()> {
        if name.is_none() && email.is_none() && ssh_key.is_none() {
            bail!("provide at least one of --name, --email, or --key");
        }
        for value in [name, email].into_iter().flatten() {
            if value.trim().is_empty() || value.contains(['\r', '\n']) {
                bail!("Git name/email must be a non-empty single line");
            }
        }
        if let Some(email) = email {
            if !email.contains('@') {
                eprintln!("warning: '{email}' does not look like an email address");
            }
        }
        if let Some(key) = ssh_key {
            ssh::validate_name(key)?;
            let path = self.ssh_dir.join(key);
            if !path.is_file() {
                bail!("SSH key '{}' does not exist", key);
            }
        }

        fs::create_dir_all(
            self.path
                .parent()
                .context("Git config path has no parent directory")?,
        )
        .context("failed to create the Git config directory")?;
        if fs::symlink_metadata(&self.path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            bail!("refusing to write through symlink {}", self.path.display());
        }

        if let Some(name) = name {
            self.set("user.name", name)?;
        }
        if let Some(email) = email {
            self.set("user.email", email)?;
        }
        if let Some(key) = ssh_key {
            let command = format!("ssh -i /home/node/.ssh/{key} -o IdentitiesOnly=yes");
            self.set("core.sshCommand", &command)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600));
        }
        println!("Git config: {}", self.path.display());
        Ok(())
    }

    pub fn show(&self) -> Result<()> {
        if !self.path.is_file() {
            println!("No Git config yet ({}).", self.path.display());
            return Ok(());
        }
        let output = Command::new("git")
            .arg("config")
            .arg("--file")
            .arg(&self.path)
            .arg("--list")
            .output()
            .context("failed to run git; install Git to inspect the config")?;
        if !output.status.success() {
            bail!(
                "git config failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        print!("{}", String::from_utf8_lossy(&output.stdout));
        Ok(())
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        let status = Command::new("git")
            .arg("config")
            .arg("--file")
            .arg(&self.path)
            .arg("--replace-all")
            .arg(key)
            .arg(value)
            .status()
            .context("failed to run git; install Git first")?;
        if !status.success() {
            bail!("git config failed for {key}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_config() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let config = Config::from_map(root.path(), Default::default()).unwrap();
        let git = GitConfig::new(&config);
        git.setup(Some("Alice"), Some("alice@example.com"), None)
            .unwrap();
        let text = fs::read_to_string(&git.path).unwrap();
        assert!(text.contains("Alice"));
        assert!(text.contains("alice@example.com"));
    }
}
