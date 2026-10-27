// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::PathBuf;

/// Paths used to reconstruct the persistent developer home.
#[derive(Debug, Clone)]
pub struct HomeLayout {
    pub home: PathBuf,
    pub workspace: PathBuf,
    pub persist: PathBuf,
}

impl HomeLayout {
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/home/node"));
        let workspace = std::env::var_os("DSH_WORKSPACE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/workspace"));
        let persist = std::env::var_os("DSH_PERSIST_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".persist"));
        Self {
            home,
            workspace,
            persist,
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::{
        fs::{self, File, OpenOptions},
        io,
        os::unix::fs::{FileTypeExt, PermissionsExt},
        path::{Path, PathBuf},
    };

    use anyhow::{Context, Result};
    use tracing::{debug, warn};

    use super::HomeLayout;

    pub fn prepare(layout: &HomeLayout) -> Result<()> {
        fs::create_dir_all(&layout.persist)
            .with_context(|| format!("failed to create {}", layout.persist.display()))?;
        set_mode_warn(&layout.persist, 0o700);

        ensure_dir_link(&layout.home.join("workspace"), &layout.workspace)?;
        set_mode_warn(&layout.workspace, 0o750);

        for (link, target) in [
            (".ssh", "ssh"),
            (".gnupg", "gnupg"),
            (".aws", "aws"),
            (".kube", "kube"),
            (".docker", "docker"),
            (".config", "config"),
            (".local/share", "local/share"),
            (".local/state", "local/state"),
        ] {
            let target = layout.persist.join(target);
            ensure_dir_link(&layout.home.join(link), &target)?;
            set_mode_warn(&target, 0o700);
        }

        for (link, target) in [
            (".gitconfig", "git/config"),
            (".git-credentials", "git/credentials"),
            (".bash_history", "shell/bash_history"),
            (".python_history", "shell/python_history"),
            (".npmrc", "npm/npmrc"),
            (".netrc", "netrc"),
            (".pypirc", "pypirc"),
            (".cargo/config.toml", "cargo/config.toml"),
            (".cargo/credentials.toml", "cargo/credentials.toml"),
        ] {
            let target = layout.persist.join(target);
            ensure_file_link(&layout.home.join(link), &target)?;
            set_mode_warn(&target, 0o600);
        }

        cleanup_gnupg_sockets(&layout.persist.join("gnupg"))?;
        Ok(())
    }

    fn ensure_dir_link(link: &Path, target: &Path) -> Result<()> {
        if let Some(parent) = link.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::create_dir_all(target)
            .with_context(|| format!("failed to create {}", target.display()))?;

        if let Ok(metadata) = fs::symlink_metadata(link) {
            if metadata.file_type().is_symlink() {
                let current = fs::read_link(link)
                    .with_context(|| format!("failed to read link {}", link.display()))?;
                if current == target {
                    return Ok(());
                }
                if let Err(error) = fs::remove_file(link) {
                    warn!(
                        path = %link.display(),
                        error = %error,
                        "could not replace existing link; keeping the in-image path"
                    );
                    return Ok(());
                }
            } else if metadata.is_dir() {
                // Merge first; only remove the source after a successful copy so
                // a failed migration cannot destroy developer data.
                copy_missing_dir(link, target)?;
                if let Err(error) = fs::remove_dir_all(link) {
                    warn!(
                        path = %link.display(),
                        error = %error,
                        "could not remove the in-image directory; keeping it instead of linking"
                    );
                    return Ok(());
                }
            } else {
                warn!(path = %link.display(), "path exists and is not a directory; not replacing");
                return Ok(());
            }
        }

        std::os::unix::fs::symlink(target, link).with_context(|| {
            format!("failed to link {} -> {}", link.display(), target.display())
        })?;
        debug!(link = %link.display(), target = %target.display(), "linked persistent directory");
        Ok(())
    }

    fn ensure_file_link(link: &Path, target: &Path) -> Result<()> {
        if let Some(parent) = link.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(target)
            .with_context(|| format!("failed to create {}", target.display()))?;

        if let Ok(metadata) = fs::symlink_metadata(link) {
            if metadata.file_type().is_symlink() {
                let current = fs::read_link(link)
                    .with_context(|| format!("failed to read link {}", link.display()))?;
                if current == target {
                    return Ok(());
                }
                if let Err(error) = fs::remove_file(link) {
                    warn!(
                        path = %link.display(),
                        error = %error,
                        "could not replace existing link; keeping the in-image path"
                    );
                    return Ok(());
                }
            } else if metadata.is_file() {
                if is_empty(target)? && metadata.len() > 0 {
                    fs::copy(link, target).with_context(|| {
                        format!(
                            "failed to migrate {} to {}",
                            link.display(),
                            target.display()
                        )
                    })?;
                }
                if let Err(error) = fs::remove_file(link) {
                    warn!(
                        path = %link.display(),
                        error = %error,
                        "could not remove the in-image file; keeping it instead of linking"
                    );
                    return Ok(());
                }
            } else {
                warn!(path = %link.display(), "path exists and is not a file; not replacing");
                return Ok(());
            }
        }

        std::os::unix::fs::symlink(target, link).with_context(|| {
            format!("failed to link {} -> {}", link.display(), target.display())
        })?;
        debug!(link = %link.display(), target = %target.display(), "linked persistent file");
        Ok(())
    }

    fn is_empty(path: &Path) -> Result<bool> {
        Ok(fs::metadata(path)
            .with_context(|| format!("failed to inspect {}", path.display()))?
            .len()
            == 0)
    }

    fn copy_missing_dir(source: &Path, target: &Path) -> Result<()> {
        for entry in
            fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
        {
            let entry = entry?;
            let source_path = entry.path();
            let target_path = target.join(entry.file_name());
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                if fs::symlink_metadata(&target_path).is_err() {
                    let link = fs::read_link(&source_path)?;
                    std::os::unix::fs::symlink(link, &target_path).with_context(|| {
                        format!("failed to migrate symlink {}", source_path.display())
                    })?;
                }
            } else if file_type.is_dir() {
                fs::create_dir_all(&target_path)?;
                copy_missing_dir(&source_path, &target_path)?;
                let permissions = fs::metadata(&source_path)?.permissions();
                let _ = fs::set_permissions(&target_path, permissions);
            } else if file_type.is_file() {
                if fs::symlink_metadata(&target_path).is_err() {
                    fs::copy(&source_path, &target_path).with_context(|| {
                        format!(
                            "failed to migrate {} to {}",
                            source_path.display(),
                            target_path.display()
                        )
                    })?;
                    let permissions = fs::metadata(&source_path)?.permissions();
                    let _ = fs::set_permissions(&target_path, permissions);
                }
            } else if file_type.is_socket() {
                // Never copy live sockets into the persistent home.
            }
        }
        Ok(())
    }

    fn cleanup_gnupg_sockets(path: &Path) -> Result<()> {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_socket() {
                fs::remove_file(entry.path())?;
                debug!(path = %entry.path().display(), "removed stale GnuPG socket");
            }
        }
        Ok(())
    }

    fn set_mode(path: &Path, mode: u32) -> Result<()> {
        let mut permissions = fs::metadata(path)
            .with_context(|| format!("failed to inspect {}", path.display()))?
            .permissions();
        permissions.set_mode(mode);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to set mode on {}", path.display()))
    }

    fn set_mode_warn(path: &Path, mode: u32) {
        if let Err(error) = set_mode(path, mode) {
            warn!(error = %error, path = %path.display(), "could not update permissions");
        }
    }

    #[allow(dead_code)]
    fn ensure_file(path: &Path) -> Result<()> {
        if !path.exists() {
            File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn links_and_migrates_without_data_loss() {
            let root = tempfile::tempdir().unwrap();
            let home = root.path().join("home");
            let persist = root.path().join("persist");
            let workspace = root.path().join("workspace");
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(&persist).unwrap();
            fs::create_dir_all(&workspace).unwrap();
            fs::write(workspace.join("keep.txt"), b"workspace").unwrap();
            fs::write(home.join(".gitconfig"), b"[user]\nname = A\n").unwrap();

            let layout = HomeLayout {
                home: home.clone(),
                workspace: workspace.clone(),
                persist: persist.clone(),
            };
            prepare(&layout).unwrap();

            assert!(home.join("workspace").is_symlink());
            assert!(home.join(".ssh").is_symlink());
            assert!(home.join(".gitconfig").is_symlink());
            assert_eq!(
                fs::read_to_string(persist.join("git/config")).unwrap(),
                "[user]\nname = A\n"
            );
            // Running again is idempotent.
            prepare(&layout).unwrap();
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use anyhow::{Result, bail};

    use super::HomeLayout;

    pub fn prepare(_layout: &HomeLayout) -> Result<()> {
        bail!("the DSH entrypoint persistent-home layout is only supported on Unix containers")
    }
}

pub use imp::prepare;
