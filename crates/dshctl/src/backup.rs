// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    fs::{self, File},
    io::{self, BufReader, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use dsh_config::Config;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde::Serialize;
use tar::{Archive, Builder, EntryType, Header};

use crate::docker::Docker;

#[derive(Debug, Serialize)]
struct Manifest {
    tool: &'static str,
    version: &'static str,
    created_unix: u64,
    runtime: String,
    includes: Vec<String>,
}

/// Create a backup archive.  Returns the absolute output path.
pub fn create(
    config: &Config,
    output: Option<PathBuf>,
    quiesce: bool,
    docker: Option<&Docker>,
) -> Result<PathBuf> {
    let output = output.unwrap_or_else(|| {
        config
            .project_dir
            .join(format!("dsh-docker-backup-{}.tar.gz", unix_timestamp()))
    });
    let output = if output.is_absolute() {
        output
    } else {
        config.project_dir.join(output)
    };
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if fs::symlink_metadata(&output)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
    {
        bail!(
            "refusing to write a backup through symlink {}",
            output.display()
        );
    }

    if quiesce {
        let docker = docker.context("--quiesce requires Docker access")?;
        docker
            .stop()
            .context("failed to stop services for a consistent backup")?;
    }

    let result = create_inner(config, &output);
    if quiesce {
        if let Some(docker) = docker {
            let start = docker.up();
            if result.is_ok() {
                start.context("failed to restart services after backup")?;
            } else {
                let _ = start;
            }
        }
    }
    result.map(|()| output)
}

fn create_inner(config: &Config, output: &Path) -> Result<()> {
    let temp = tempfile::Builder::new()
        .prefix(".dsh-backup-")
        .suffix(".tar.gz")
        .tempfile_in(output.parent().unwrap_or_else(|| Path::new(".")))
        .context("failed to create temporary backup")?;
    {
        let encoder = GzEncoder::new(temp.as_file(), Compression::default());
        let mut builder = Builder::new(encoder);
        builder.mode(tar::HeaderMode::Complete);

        let env_file = config.project_dir.join(".env");
        if env_file.is_file() {
            builder
                .append_path_with_name(&env_file, Path::new(".env"))
                .context("failed to archive .env")?;
        }

        let manifest = Manifest {
            tool: "dshctl",
            version: env!("CARGO_PKG_VERSION"),
            created_unix: unix_timestamp(),
            runtime: config
                .runtime_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(".runtime")
                .to_owned(),
            includes: vec![
                ".env".to_owned(),
                config
                    .runtime_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(".runtime")
                    .to_owned(),
            ],
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        let mut header = Header::new_gnu();
        header.set_size(manifest_bytes.len() as u64);
        header.set_mode(0o600);
        header.set_mtime(unix_timestamp());
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                "dsh-docker-backup.json",
                manifest_bytes.as_slice(),
            )
            .context("failed to archive manifest")?;

        if config.runtime_dir.exists() {
            let runtime_name = config
                .runtime_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(".runtime");
            append_tree(&mut builder, &config.runtime_dir, Path::new(runtime_name))?;
        }

        let encoder = builder
            .into_inner()
            .context("failed to finish tar archive")?;
        let mut file = encoder.finish().context("failed to finish gzip stream")?;
        file.flush()?;
        file.sync_all()?;
    }
    temp.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    if output.exists() {
        fs::remove_file(output)
            .with_context(|| format!("failed to replace {}", output.display()))?;
    }
    temp.persist(output)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to write {}", output.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(output, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn append_tree(
    builder: &mut Builder<GzEncoder<&File>>,
    source: &Path,
    archive: &Path,
) -> Result<()> {
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", source.display()));
        }
    };
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Directory);
        header.set_size(0);
        header.set_mode(0o700);
        header.set_mtime(unix_timestamp());
        header.set_cksum();
        builder
            .append_data(&mut header, archive, io::empty())
            .with_context(|| format!("failed to archive directory {}", source.display()))?;
        for entry in
            fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
        {
            let entry = entry?;
            append_tree(builder, &entry.path(), &archive.join(entry.file_name()))?;
        }
    } else if file_type.is_file() {
        builder
            .append_path_with_name(source, archive)
            .with_context(|| format!("failed to archive {}", source.display()))?;
    } else if file_type.is_symlink() {
        let target = fs::read_link(source)
            .with_context(|| format!("failed to read link {}", source.display()))?;
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_mtime(unix_timestamp());
        header.set_link_name(&target)?;
        header.set_cksum();
        builder
            .append_data(&mut header, archive, io::empty())
            .with_context(|| format!("failed to archive symlink {}", source.display()))?;
    }
    // Sockets, FIFOs and device nodes are intentionally skipped.
    Ok(())
}

/// Restore a backup into the current project.
pub fn restore(
    config: &Config,
    archive_path: &Path,
    assume_yes: bool,
    docker: Option<&Docker>,
) -> Result<()> {
    if !assume_yes {
        bail!(
            "restore replaces .env and the runtime directory; re-run with --yes after taking a backup"
        );
    }
    let archive_path = if archive_path.is_absolute() {
        archive_path.to_path_buf()
    } else {
        config.project_dir.join(archive_path)
    };
    let file = File::open(&archive_path)
        .with_context(|| format!("failed to open {}", archive_path.display()))?;
    let decoder = GzDecoder::new(BufReader::new(file));
    let mut archive = Archive::new(decoder);
    let stage = tempfile::Builder::new()
        .prefix(".dsh-restore-")
        .tempdir_in(&config.project_dir)
        .context("failed to create restore staging directory")?;

    let mut has_env = false;
    let mut top_levels = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        validate_archive_path(&path)?;
        if path == Path::new(".env") {
            has_env = true;
        }
        if let Some(Component::Normal(name)) = path.components().next() {
            let name = name.to_string_lossy().to_string();
            if !top_levels.contains(&name) {
                top_levels.push(name);
            }
        }
        entry
            .unpack_in(stage.path())
            .context("failed to unpack archive entry")?;
    }

    let runtime_name = config
        .runtime_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(".runtime")
        .to_owned();
    let staged_runtime = stage.path().join(&runtime_name);
    if !has_env && !staged_runtime.exists() {
        bail!("archive does not contain .env or the {runtime_name} runtime directory");
    }

    if let Some(docker) = docker {
        docker
            .stop()
            .context("failed to stop services before restore")?;
    }

    let stamp = unix_timestamp();
    let env_path = config.project_dir.join(".env");
    let env_backup = config.project_dir.join(format!(".env.bak-{stamp}"));
    let runtime_backup = config
        .project_dir
        .join(format!("{runtime_name}.bak-{stamp}"));
    let mut moved_env = false;
    let mut moved_runtime = false;
    if env_path.exists() {
        fs::rename(&env_path, &env_backup).context("failed to move current .env aside")?;
        moved_env = true;
    }
    if config.runtime_dir.exists() {
        fs::rename(&config.runtime_dir, &runtime_backup)
            .context("failed to move current runtime directory aside")?;
        moved_runtime = true;
    }

    let apply = (|| -> Result<()> {
        if stage.path().join(".env").is_file() {
            fs::copy(stage.path().join(".env"), &env_path).context("failed to restore .env")?;
        }
        if staged_runtime.exists() {
            fs::rename(&staged_runtime, &config.runtime_dir)
                .or_else(|_| copy_dir(&staged_runtime, &config.runtime_dir))
                .context("failed to restore runtime directory")?;
        }
        crate::runtime::apply_permissions(config)?;
        Ok(())
    })();

    if let Err(error) = apply {
        let _ = fs::remove_dir_all(&config.runtime_dir);
        let _ = fs::remove_file(&env_path);
        if moved_runtime {
            let _ = fs::rename(&runtime_backup, &config.runtime_dir);
        }
        if moved_env {
            let _ = fs::rename(&env_backup, &env_path);
        }
        if let Some(docker) = docker {
            let _ = docker.up();
        }
        return Err(error).context("restore failed; the previous deployment was restored");
    }

    if let Some(docker) = docker {
        docker
            .up()
            .context("restore completed but services failed to start")?;
    }
    Ok(())
}

fn validate_archive_path(path: &Path) -> Result<()> {
    if path.is_absolute() {
        bail!("archive contains an absolute path: {}", path.display());
    }
    for component in path.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("archive contains an unsafe path: {}", path.display())
            }
            _ => {}
        }
    }
    Ok(())
}

fn copy_dir(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target_path = target.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir(&entry.path(), &target_path)?;
        } else if file_type.is_symlink() {
            let link = fs::read_link(entry.path())?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(link, &target_path)?;
            #[cfg(not(unix))]
            let _ = link;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target_path)?;
        }
    }
    Ok(())
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn config(dir: &Path) -> Config {
        Config::from_map(dir, BTreeMap::new()).unwrap()
    }

    #[test]
    fn backup_and_restore_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        fs::create_dir_all(cfg.runtime_dir.join("workspace")).unwrap();
        fs::write(cfg.project_dir.join(".env"), b"DSH_PORT=3080\n").unwrap();
        fs::write(cfg.runtime_dir.join("workspace/marker"), b"keep me").unwrap();

        let archive = create(&cfg, None, false, None).unwrap();
        assert!(archive.is_file());
        fs::write(cfg.runtime_dir.join("workspace/marker"), b"changed").unwrap();
        restore(&cfg, &archive, true, None).unwrap();
        assert_eq!(
            fs::read(cfg.runtime_dir.join("workspace/marker")).unwrap(),
            b"keep me"
        );
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(validate_archive_path(Path::new("../evil")).is_err());
        assert!(validate_archive_path(Path::new("/etc/passwd")).is_err());
        assert!(validate_archive_path(Path::new(".env")).is_ok());
    }
}
