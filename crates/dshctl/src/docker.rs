// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    ffi::OsString,
    path::PathBuf,
    process::{Command, ExitStatus, Output, Stdio},
};

use anyhow::{Context, Result, bail};
use dsh_config::{Config, ImageMode, MirrorMode};
use serde_json::Value;

pub const RUNTIME_LABEL: &str = "io.github.rin721.dsh-docker.runtime";
pub const RUNTIME_LABEL_VALUE: &str = "2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Prebuilt,
    Local,
    Built,
}

impl Delivery {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prebuilt => "prebuilt",
            Self::Local => "local",
            Self::Built => "built",
        }
    }
}

/// Thin, testable wrapper around the Docker CLI.
///
/// Commands that download or build inherit stdio so progress is never swallowed
/// into a variable (the historical "fake hang" regression).
pub struct Docker {
    root: PathBuf,
    config: Config,
    verbose: bool,
}

impl Docker {
    pub fn new(config: Config, verbose: bool) -> Self {
        Self {
            root: config.project_dir.clone(),
            config,
            verbose,
        }
    }

    pub fn require(&self) -> Result<()> {
        self.run_capture("docker", &[os("--version")], &[])
            .context("docker is not installed or not on PATH")?;
        self.run_capture("docker", &[os("compose"), os("version")], &[])
            .context("Docker Compose v2 (`docker compose`) is required")?;
        self.run_capture("docker", &[os("info")], &[])
            .context("Docker daemon is not reachable; start Docker and check permissions")?;
        Ok(())
    }

    /// Remove containers left by early v1 releases that used fixed names.
    ///
    /// Only containers explicitly labeled as this project (or the historical
    /// `deepseek-harness`/`dsh-docker` projects) are touched.
    pub fn cleanup_legacy(&self) -> Result<()> {
        let own_project = self.project_name();
        for name in [
            "deepseek-harness",
            "dsh-tinyauth",
            "dsh-gateway",
            "dsh-caddy",
            "dsh-edge",
        ] {
            let output = self.run_capture(
                "docker",
                &[
                    os("inspect"),
                    os("--format"),
                    os("{{ index .Config.Labels \"com.docker.compose.project\" }}"),
                    os(name),
                ],
                &[],
            )?;
            if !output.status.success() {
                continue;
            }
            let project = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if project == "deepseek-harness" || project == "dsh-docker" || project == own_project {
                println!("Removing legacy container: {name}");
                let _ = self.run_status("docker", &[os("rm"), os("-f"), os(name)], &[], false);
            }
        }
        Ok(())
    }

    pub fn compose_config_check(&self) -> Result<()> {
        let output = self
            .compose_capture(&[os("config")], false)
            .context("failed to validate Docker Compose configuration")?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            bail!("docker compose config failed:\n{stderr}");
        }
        Ok(())
    }

    pub fn up(&self) -> Result<()> {
        self.compose_status(&[os("up"), os("-d"), os("--remove-orphans")], false)
    }

    pub fn down(&self) -> Result<()> {
        self.compose_status(&[os("down"), os("--remove-orphans")], false)
    }

    pub fn stop(&self) -> Result<()> {
        self.compose_status(&[os("stop")], false)
    }

    pub fn restart(&self) -> Result<()> {
        self.compose_status(&[os("restart")], false)
    }

    pub fn logs(&self, follow: bool, tail: Option<usize>) -> Result<()> {
        let mut args = vec![os("logs")];
        if follow {
            args.push(os("--follow"));
        }
        if let Some(tail) = tail {
            args.push(os("--tail"));
            args.push(os(tail.to_string()));
        }
        args.push(os("dsh"));
        self.compose_status(&args, false)
    }

    pub fn shell(&self) -> Result<()> {
        self.compose_status(&[os("exec"), os("dsh"), os("bash")], false)
    }

    pub fn exec(&self, command: &[String]) -> Result<()> {
        if command.is_empty() {
            bail!("exec requires a command");
        }
        let mut args = vec![os("exec"), os("dsh")];
        args.extend(command.iter().map(os));
        self.compose_status(&args, false)
    }

    pub fn ps(&self) -> Result<Vec<Value>> {
        let output = self
            .compose_capture(&[os("ps"), os("--format"), os("json")], false)
            .context("failed to query compose services")?;
        if !output.status.success() {
            return Ok(Vec::new());
        }
        Ok(parse_json_lines(&String::from_utf8_lossy(&output.stdout)))
    }

    pub fn project_name(&self) -> String {
        self.config.compose_project_name()
    }

    pub fn port_owned(&self, port: u16) -> bool {
        let project = self.project_name();
        let filter = format!("label=com.docker.compose.project={project}");
        let output = match self.run_capture(
            "docker",
            &[
                os("ps"),
                os("-a"),
                os("--filter"),
                os(filter),
                os("--format"),
                os("{{.ID}}"),
            ],
            &[],
        ) {
            Ok(output) => output,
            Err(_) => return false,
        };
        if !output.status.success() {
            return false;
        }
        for id in String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            if let Ok(ports) = self.run_capture("docker", &[os("port"), os(id)], &[]) {
                if String::from_utf8_lossy(&ports.stdout)
                    .lines()
                    .any(|line| line.trim_end().ends_with(&format!(":{port}")))
                {
                    return true;
                }
            }
        }
        false
    }

    pub fn manifest_inspect(&self, image: &str) -> Result<()> {
        let output =
            self.run_capture("docker", &[os("manifest"), os("inspect"), os(image)], &[])?;
        if !output.status.success() {
            bail!(
                "docker manifest inspect failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    pub fn image_exists(&self, image: &str) -> bool {
        self.run_capture("docker", &[os("image"), os("inspect"), os(image)], &[])
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    pub fn image_label(&self, image: &str, label: &str) -> Result<Option<String>> {
        let format = format!("{{{{index .Config.Labels \"{label}\"}}}}");
        let output = self.run_capture(
            "docker",
            &[
                os("image"),
                os("inspect"),
                os("--format"),
                os(format),
                os(image),
            ],
            &[],
        )?;
        if !output.status.success() {
            return Ok(None);
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if value.is_empty() || value == "<no value>" {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    pub fn image_compatible(&self, image: &str) -> bool {
        matches!(
            self.image_label(image, RUNTIME_LABEL),
            Ok(Some(value)) if value == RUNTIME_LABEL_VALUE
        )
    }

    pub fn prepare_image(&self) -> Result<Delivery> {
        match self.config.image_mode {
            ImageMode::Pull => {
                self.pull_with_mirrors()?;
                Ok(Delivery::Prebuilt)
            }
            ImageMode::Build => {
                self.build_image()?;
                Ok(Delivery::Built)
            }
            ImageMode::Auto => {
                println!("Trying prebuilt image: {}", self.config.effective_image());
                match self.pull_with_mirrors() {
                    Ok(()) => Ok(Delivery::Prebuilt),
                    Err(error) => {
                        eprintln!("Prebuilt image is unavailable: {error:#}");
                        if self.image_exists(&self.config.effective_image())
                            && self.image_compatible(&self.config.effective_image())
                        {
                            println!("Reusing compatible local image.");
                            Ok(Delivery::Local)
                        } else if self.can_build() {
                            println!("Falling back to a local build.");
                            self.build_image()?;
                            Ok(Delivery::Built)
                        } else {
                            bail!(
                                "prebuilt image is unavailable and this directory has no Dockerfile/Cargo.toml to build from; \
                                 clone the repository or set DSH_IMAGE to a reachable mirror"
                            );
                        }
                    }
                }
            }
        }
    }

    pub fn pull_with_mirrors(&self) -> Result<()> {
        let image = self.config.effective_image();
        let mut candidates = Vec::new();
        let mirrors = self.mirror_hosts();
        match self.config.image_mirror {
            MirrorMode::Cn => {
                for mirror in &mirrors {
                    if let Some(candidate) = rewrite_ghcr(&image, mirror) {
                        candidates.push(candidate);
                    }
                }
                candidates.push(image.clone());
            }
            MirrorMode::Global => {
                candidates.push(image.clone());
                for mirror in &mirrors {
                    if let Some(candidate) = rewrite_ghcr(&image, mirror) {
                        candidates.push(candidate);
                    }
                }
            }
            MirrorMode::Off => candidates.push(image.clone()),
        }

        let mut errors = Vec::new();
        for candidate in candidates {
            println!("Pulling {candidate}");
            let status = self.run_status("docker", &[os("pull"), os(&candidate)], &[], true);
            match status {
                Ok(status) if status.success() => {
                    if candidate != image {
                        self.run_status(
                            "docker",
                            &[os("tag"), os(&candidate), os(&image)],
                            &[],
                            false,
                        )
                        .with_context(|| format!("failed to tag {candidate} as {image}"))?;
                    }
                    if self.image_compatible(&image) {
                        return Ok(());
                    }
                    errors.push(format!(
                        "{candidate}: image does not carry the v2 runtime label"
                    ));
                }
                Ok(status) => errors.push(format!("{candidate}: docker pull exited with {status}")),
                Err(error) => errors.push(format!("{candidate}: {error:#}")),
            }
        }
        bail!("could not pull a compatible image: {}", errors.join("; "))
    }

    pub fn build_image(&self) -> Result<()> {
        if !self.can_build() {
            bail!(
                "local build requires Dockerfile and Cargo.toml in {}; clone the source repository first",
                self.root.display()
            );
        }
        println!("Building DSH image locally (this reuses BuildKit caches)...");
        self.compose_status(&[os("build"), os("dsh")], true)?;
        if !self.image_compatible(&self.config.effective_image()) {
            bail!(
                "built image {} does not carry the v2 runtime label",
                self.config.effective_image()
            );
        }
        Ok(())
    }

    pub fn can_build(&self) -> bool {
        self.root.join("Dockerfile").is_file() && self.root.join("Cargo.toml").is_file()
    }

    pub fn mirror_hosts(&self) -> Vec<String> {
        if !self.config.image_mirrors.is_empty() {
            return self.config.image_mirrors.clone();
        }
        if self.config.image_mirror == MirrorMode::Cn {
            vec![
                "ghcr.nju.edu.cn".to_owned(),
                "ghcr.m.daocloud.io".to_owned(),
            ]
        } else {
            Vec::new()
        }
    }

    fn compose_files(&self, build: bool) -> Result<Vec<PathBuf>> {
        let mut files = vec![self.root.join("compose.yaml")];
        for override_name in ["compose.override.yaml", "compose.override.yml"] {
            let candidate = self.root.join(override_name);
            if candidate.is_file() {
                files.push(candidate);
                break;
            }
        }
        if build {
            files.push(self.root.join("compose.build.yaml"));
        }
        for file in &files {
            if !file.is_file() {
                bail!("required Compose file is missing: {}", file.display());
            }
        }
        Ok(files)
    }

    fn compose_args(&self, args: &[OsString], build: bool) -> Result<Vec<OsString>> {
        let files = self.compose_files(build)?;
        let mut command = vec![
            os("compose"),
            os("--project-directory"),
            os(&self.root),
            os("--env-file"),
            os(self.root.join(".env")),
        ];
        for file in files {
            command.push(os("-f"));
            command.push(os(file));
        }
        command.extend(args.iter().cloned());
        Ok(command)
    }

    fn compose_status(&self, args: &[OsString], build: bool) -> Result<()> {
        let compose = self.compose_args(args, build)?;
        let status = self.run_status("docker", &compose, &self.compose_env(), true)?;
        if !status.success() {
            bail!("docker compose exited with {status}");
        }
        Ok(())
    }

    fn compose_capture(&self, args: &[OsString], build: bool) -> Result<Output> {
        let compose = self.compose_args(args, build)?;
        self.run_capture("docker", &compose, &self.compose_env())
    }

    fn compose_env(&self) -> Vec<(String, String)> {
        self.config.compose_env()
    }

    fn run_status(
        &self,
        program: &str,
        args: &[OsString],
        envs: &[(String, String)],
        inherit: bool,
    ) -> Result<ExitStatus> {
        let mut command = Command::new(program);
        command.args(args);
        for (key, value) in envs {
            command.env(key, value);
        }
        if inherit {
            command.stdin(Stdio::inherit());
            command.stdout(Stdio::inherit());
            command.stderr(Stdio::inherit());
        }
        if self.verbose {
            eprintln!("+ {} {}", program, display_args(args));
        }
        command
            .status()
            .with_context(|| format!("failed to run {program}"))
    }

    fn run_capture(
        &self,
        program: &str,
        args: &[OsString],
        envs: &[(String, String)],
    ) -> Result<Output> {
        let mut command = Command::new(program);
        command.args(args);
        for (key, value) in envs {
            command.env(key, value);
        }
        if self.verbose {
            eprintln!("+ {} {}", program, display_args(args));
        }
        command
            .output()
            .with_context(|| format!("failed to run {program}"))
    }
}

fn rewrite_ghcr(image: &str, mirror: &str) -> Option<String> {
    image
        .strip_prefix("ghcr.io/")
        .map(|rest| format!("{mirror}/{rest}"))
}

fn parse_json_lines(text: &str) -> Vec<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return match value {
            Value::Array(values) => values,
            Value::Object(_) => vec![value],
            _ => Vec::new(),
        };
    }
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn os(value: impl AsRef<std::ffi::OsStr>) -> OsString {
    value.as_ref().to_owned()
}

fn display_args(args: &[OsString]) -> String {
    args.iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_ghcr_images() {
        assert_eq!(
            rewrite_ghcr("ghcr.io/rin721/dsh-docker:2", "ghcr.nju.edu.cn").as_deref(),
            Some("ghcr.nju.edu.cn/rin721/dsh-docker:2")
        );
        assert!(rewrite_ghcr("docker.io/library/node:24", "ghcr.nju.edu.cn").is_none());
    }

    #[test]
    fn parses_compose_ps_json() {
        let lines = "{\"Name\":\"dsh\"}\n{\"Name\":\"other\"}\n";
        assert_eq!(parse_json_lines(lines).len(), 2);
        let array = "[{\"Name\":\"dsh\"}]";
        assert_eq!(parse_json_lines(array).len(), 1);
    }
}
