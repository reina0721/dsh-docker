// SPDX-License-Identifier: MIT OR Apache-2.0

//! Typed configuration for the DSH Docker runtime.
//!
//! The runtime intentionally keeps `.env` as the single source of truth so
//! Docker Compose users can continue to inspect and edit it.  This crate adds
//! validation and typed access without ever sourcing the file as shell code.

use std::{
    collections::BTreeMap,
    env, fmt, fs,
    net::IpAddr,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Default image for local builds and deployment.
pub const DEFAULT_IMAGE: &str = "dsh-docker:local";
/// Default realm used in the Basic Auth challenge.
pub const DEFAULT_REALM: &str = "DeepSeek Harness";
/// Default bcrypt cost used for newly created passwords.
pub const DEFAULT_BCRYPT_COST: u32 = 14;
/// bcrypt only consumes the first 72 bytes of a password.
pub const MAX_PASSWORD_BYTES: usize = 72;

/// How the runtime image is obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageMode {
    /// Pull, then reuse a compatible local image, then build.
    Auto,
    /// Only use the prebuilt image.
    Pull,
    /// Always build locally from the source tree.
    Build,
}

impl FromStr for ImageMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "pull" => Ok(Self::Pull),
            "build" => Ok(Self::Build),
            other => bail!("DSH_IMAGE_MODE must be auto, pull or build (got {other:?})"),
        }
    }
}

impl fmt::Display for ImageMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Pull => "pull",
            Self::Build => "build",
        })
    }
}

/// Image flavour published by the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageVariant {
    Dev,
    Slim,
}

impl FromStr for ImageVariant {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "dev" | "full" => Ok(Self::Dev),
            "slim" | "runtime" => Ok(Self::Slim),
            other => bail!("DSH_IMAGE_VARIANT must be dev or slim (got {other:?})"),
        }
    }
}

impl fmt::Display for ImageVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Dev => "dev",
            Self::Slim => "slim",
        })
    }
}

/// Order in which direct and mirror image sources are tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MirrorMode {
    /// Canonical registry first, then configured mirrors.
    Global,
    /// Configured mirrors first, then the canonical registry.
    Cn,
    /// Canonical registry only.
    Off,
}

impl FromStr for MirrorMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "global" | "auto" => Ok(Self::Global),
            "cn" | "china" => Ok(Self::Cn),
            "off" | "none" => Ok(Self::Off),
            other => bail!("DSH_IMAGE_MIRROR must be global, cn or off (got {other:?})"),
        }
    }
}

impl fmt::Display for MirrorMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Global => "global",
            Self::Cn => "cn",
            Self::Off => "off",
        })
    }
}

/// Network driver mode used by the container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    /// Default bridge networking with published ports.
    Bridge,
    /// Direct IP mode via IPVlan (container holds dedicated IP directly).
    Ipvlan,
    /// Direct IP mode via Macvlan (container gets dedicated virtual MAC and IP).
    Macvlan,
}

impl FromStr for NetworkMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "bridge" | "default" => Ok(Self::Bridge),
            "ipvlan" => Ok(Self::Ipvlan),
            "macvlan" => Ok(Self::Macvlan),
            other => bail!("DSH_NETWORK_MODE must be bridge, ipvlan or macvlan (got {other:?})"),
        }
    }
}

impl fmt::Display for NetworkMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bridge => "bridge",
            Self::Ipvlan => "ipvlan",
            Self::Macvlan => "macvlan",
        })
    }
}

/// Fully validated runtime configuration.
#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub project_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub bind_address: String,
    pub dsh_port: u16,
    pub proxy_port: u16,
    pub internal_host: String,
    pub internal_port: u16,
    pub image: String,
    pub image_mode: ImageMode,
    pub image_variant: ImageVariant,
    pub image_mirror: MirrorMode,
    pub image_mirrors: Vec<String>,
    pub auth_realm: String,
    pub bcrypt_cost: u32,
    pub min_password_len: usize,
    pub compat: bool,
    pub runtime_uid: u32,
    pub runtime_gid: u32,
    pub log_level: String,
    pub log_format: String,
    pub dsh_version: String,
    pub install_go_dev_tools: bool,
    pub node_version: String,
    pub go_version: String,
    pub rust_version: String,
    pub pnpm_version: String,
    pub build_mirror: String,
    /// Explicit Docker container name (or None to use Compose default).
    pub container_name: Option<String>,
    /// Network mode (bridge, ipvlan, macvlan).
    pub network_mode: NetworkMode,
    /// Parent network interface for ipvlan/macvlan (e.g. eth0).
    pub network_parent: Option<String>,
    /// Dedicated IPv4 address assigned to the container in direct network mode.
    pub network_ip: Option<String>,
    /// Subnet CIDR for direct network mode (e.g. 198.51.100.0/24).
    pub network_subnet: Option<String>,
    /// Gateway for direct network mode (e.g. 198.51.100.1).
    pub network_gateway: Option<String>,
    /// IPVlan operating mode (l2 or l3, defaults to l2).
    pub network_ipvlan_mode: String,
    /// Whether to also attach the default bridge network in direct network mode.
    pub network_attach_bridge: bool,
    /// Values read from `.env` plus recognised process-environment overrides.
    pub env: BTreeMap<String, String>,
}

impl Config {
    /// Load `.env` from `project_dir` and overlay recognised process variables.
    pub fn load(project_dir: impl AsRef<Path>) -> Result<Self> {
        let project_dir = project_dir.as_ref();
        let file = project_dir.join(".env");
        let mut map = if file.is_file() {
            let text = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            parse_dotenv(&text)
        } else {
            BTreeMap::new()
        };

        for (key, value) in env::vars() {
            if is_known_key(&key) {
                map.insert(key, value);
            }
        }

        Self::from_map(project_dir, map)
    }

    /// Build configuration from an explicit map (used by tests and by `--env`).
    pub fn from_map(project_dir: &Path, env: BTreeMap<String, String>) -> Result<Self> {
        let project_dir = absolute(project_dir)?;
        let get = |key: &str| env.get(key).map(String::as_str);

        let runtime_dir_raw = get("RUNTIME_DIR").unwrap_or("./.runtime");
        if runtime_dir_raw.trim().is_empty() {
            bail!("RUNTIME_DIR must not be empty");
        }
        let runtime_dir = if Path::new(runtime_dir_raw).is_absolute() {
            PathBuf::from(runtime_dir_raw)
        } else {
            project_dir.join(runtime_dir_raw.trim_start_matches("./"))
        };

        let bind_address = get("BIND_ADDRESS").unwrap_or("127.0.0.1").trim().to_owned();
        let dsh_port = parse_u16(&env, "DSH_PORT", 3080)?;
        let proxy_port = parse_u16(&env, "DSH_PROXY_PORT", 3080)?;
        let internal_host = get("DSH_INTERNAL_HOST")
            .unwrap_or("127.0.0.1")
            .trim()
            .to_owned();
        let internal_port = parse_u16(&env, "DSH_INTERNAL_PORT", 3079)?;
        let image = get("DSH_IMAGE").unwrap_or(DEFAULT_IMAGE).trim().to_owned();
        let image_mode = get("DSH_IMAGE_MODE").unwrap_or("build").parse()?;
        let image_variant = get("DSH_IMAGE_VARIANT").unwrap_or("dev").parse()?;
        let image_mirror = get("DSH_IMAGE_MIRROR").unwrap_or("global").parse()?;
        let image_mirrors = get("DSH_IMAGE_MIRRORS")
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        let auth_realm = get("AUTH_REALM").unwrap_or(DEFAULT_REALM).trim().to_owned();
        let bcrypt_cost = parse_u32(&env, "AUTH_BCRYPT_COST", DEFAULT_BCRYPT_COST)?;
        let min_password_len = parse_usize(&env, "AUTH_MIN_PASSWORD_LENGTH", 12)?;
        let compat = parse_bool(&env, "DSH_HTTP_COMPAT_SHIM", true)?;
        let runtime_uid = parse_u32(&env, "RUNTIME_UID", 1000)?;
        let runtime_gid = parse_u32(&env, "RUNTIME_GID", 1000)?;
        let log_level = get("DSH_LOG_LEVEL").unwrap_or("info").trim().to_owned();
        let log_format = get("DSH_LOG_FORMAT").unwrap_or("text").trim().to_owned();
        let dsh_version = get("DSH_VERSION").unwrap_or("latest").trim().to_owned();
        let install_go_dev_tools = parse_bool(&env, "INSTALL_GO_DEV_TOOLS", true)?;
        let node_version = get("NODE_VERSION").unwrap_or("24.18.0").trim().to_owned();
        let go_version = get("GO_VERSION").unwrap_or("1.26.5").trim().to_owned();
        let rust_version = get("RUST_VERSION").unwrap_or("1.90.0").trim().to_owned();
        let pnpm_version = get("PNPM_VERSION").unwrap_or("11.7.0").trim().to_owned();
        let build_mirror = get("BUILD_MIRROR").unwrap_or("global").trim().to_owned();
        let container_name = get("DSH_CONTAINER_NAME")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let network_ip = get("DSH_NETWORK_IP")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let network_mode = match get("DSH_NETWORK_MODE") {
            Some(v) => v.parse()?,
            None => {
                if network_ip.is_some() {
                    NetworkMode::Ipvlan
                } else {
                    NetworkMode::Bridge
                }
            }
        };
        let network_parent = get("DSH_NETWORK_PARENT")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                if network_mode != NetworkMode::Bridge {
                    Some("eth0".to_owned())
                } else {
                    None
                }
            });
        let network_subnet = get("DSH_NETWORK_SUBNET")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                network_ip.as_ref().and_then(|ip_str| {
                    if let Ok(IpAddr::V4(v4)) = ip_str.parse::<IpAddr>() {
                        let oct = v4.octets();
                        Some(format!("{}.{}.{}.0/24", oct[0], oct[1], oct[2]))
                    } else {
                        None
                    }
                })
            });
        let network_gateway = get("DSH_NETWORK_GATEWAY")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                network_ip.as_ref().and_then(|ip_str| {
                    if let Ok(IpAddr::V4(v4)) = ip_str.parse::<IpAddr>() {
                        let oct = v4.octets();
                        Some(format!("{}.{}.{}.1", oct[0], oct[1], oct[2]))
                    } else {
                        None
                    }
                })
            });
        let network_ipvlan_mode = get("DSH_NETWORK_IPVLAN_MODE")
            .unwrap_or("l2")
            .trim()
            .to_ascii_lowercase();
        let network_attach_bridge = parse_bool(&env, "DSH_NETWORK_ATTACH_BRIDGE", true)?;

        let config = Self {
            project_dir,
            runtime_dir,
            bind_address,
            dsh_port,
            proxy_port,
            internal_host,
            internal_port,
            image,
            image_mode,
            image_variant,
            image_mirror,
            image_mirrors,
            auth_realm,
            bcrypt_cost,
            min_password_len,
            compat,
            runtime_uid,
            runtime_gid,
            log_level,
            log_format,
            dsh_version,
            install_go_dev_tools,
            node_version,
            go_version,
            rust_version,
            pnpm_version,
            build_mirror,
            container_name,
            network_mode,
            network_parent,
            network_ip,
            network_subnet,
            network_gateway,
            network_ipvlan_mode,
            network_attach_bridge,
            env,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.dsh_port == 0 || self.proxy_port == 0 || self.internal_port == 0 {
            bail!("ports must be in the range 1-65535");
        }
        if self.proxy_port == self.internal_port {
            bail!("DSH_PROXY_PORT and DSH_INTERNAL_PORT must differ");
        }
        self.bind_ip().with_context(|| {
            format!(
                "BIND_ADDRESS must be an IP address (got {:?})",
                self.bind_address
            )
        })?;
        if self.internal_host.trim().is_empty() {
            bail!("DSH_INTERNAL_HOST must not be empty");
        }
        if self.image.trim().is_empty() {
            bail!("DSH_IMAGE must not be empty");
        }
        if !(4..=31).contains(&self.bcrypt_cost) {
            bail!(
                "AUTH_BCRYPT_COST must be between 4 and 31 (got {})",
                self.bcrypt_cost
            );
        }
        if !(1..=MAX_PASSWORD_BYTES).contains(&self.min_password_len) {
            bail!(
                "AUTH_MIN_PASSWORD_LENGTH must be between 1 and {MAX_PASSWORD_BYTES} (got {})",
                self.min_password_len
            );
        }
        if self.auth_realm.len() > 128 || self.auth_realm.contains(['\r', '\n']) {
            bail!("AUTH_REALM must be a single line of at most 128 characters");
        }
        if !matches!(
            self.log_level.as_str(),
            "trace" | "debug" | "info" | "warn" | "error"
        ) {
            bail!("DSH_LOG_LEVEL must be trace, debug, info, warn or error");
        }
        if !matches!(self.log_format.as_str(), "text" | "json") {
            bail!("DSH_LOG_FORMAT must be text or json");
        }
        for mirror in &self.image_mirrors {
            if mirror.contains('/') || mirror.contains(':') || mirror.contains("://") {
                bail!("DSH_IMAGE_MIRRORS entries must be registry hosts, got {mirror:?}");
            }
        }
        if let Some(ref name) = self.container_name {
            validate_container_name(name)?;
        }
        if self.network_mode != NetworkMode::Bridge {
            let ip = self.network_ip.as_deref().unwrap_or("").trim();
            if ip.is_empty() {
                bail!("DSH_NETWORK_IP is required when DSH_NETWORK_MODE is not bridge");
            }
            ip.parse::<IpAddr>().with_context(|| {
                format!("DSH_NETWORK_IP must be a valid IP address (got {ip:?})")
            })?;

            let parent = self.network_parent.as_deref().unwrap_or("").trim();
            if parent.is_empty() {
                bail!(
                    "DSH_NETWORK_PARENT (e.g. eth0) is required when DSH_NETWORK_MODE is not bridge"
                );
            }

            let subnet = self.network_subnet.as_deref().unwrap_or("").trim();
            if subnet.is_empty() || !subnet.contains('/') {
                bail!("DSH_NETWORK_SUBNET must be a CIDR subnet (e.g. 198.51.100.0/24)");
            }

            let gateway = self.network_gateway.as_deref().unwrap_or("").trim();
            if gateway.is_empty() {
                bail!("DSH_NETWORK_GATEWAY is required when DSH_NETWORK_MODE is not bridge");
            }
            gateway.parse::<IpAddr>().with_context(|| {
                format!("DSH_NETWORK_GATEWAY must be a valid IP address (got {gateway:?})")
            })?;

            if !matches!(self.network_ipvlan_mode.as_str(), "l2" | "l3") {
                bail!(
                    "DSH_NETWORK_IPVLAN_MODE must be l2 or l3 (got {:?})",
                    self.network_ipvlan_mode
                );
            }
        }
        if self.runtime_uid == 0 {
            // Running the container as uid 0 is allowed, but callers should be
            // explicit about it.  Keep a warning hook available for the CLI.
        }
        Ok(())
    }

    /// Parse `BIND_ADDRESS`, accepting bracketed IPv6 literals for Compose.
    pub fn bind_ip(&self) -> Result<IpAddr> {
        let value = self
            .bind_address
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']');
        value.parse::<IpAddr>().with_context(|| {
            format!(
                "BIND_ADDRESS must be an IP address (got {:?})",
                self.bind_address
            )
        })
    }

    pub fn auth_dir(&self) -> PathBuf {
        self.runtime_dir.join("auth")
    }

    pub fn auth_file(&self) -> PathBuf {
        self.auth_dir().join("users.htpasswd")
    }

    pub fn legacy_auth_file(&self) -> PathBuf {
        self.auth_dir().join("users.caddy")
    }

    pub fn home_dir(&self) -> PathBuf {
        self.runtime_dir.join("home")
    }

    pub fn workspace_dir(&self) -> PathBuf {
        self.runtime_dir.join("workspace")
    }

    pub fn dsh_home(&self) -> PathBuf {
        self.runtime_dir.join("dsh-home")
    }

    pub fn compose_project_name(&self) -> String {
        if let Some(name) = self.env.get("COMPOSE_PROJECT_NAME") {
            if !name.trim().is_empty() {
                return sanitize_project_name(name);
            }
        }
        let name = self
            .project_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("dsh-docker");
        sanitize_project_name(name)
    }

    /// The image reference to use after applying the dev/slim default.
    pub fn effective_image(&self) -> String {
        if self.image == DEFAULT_IMAGE && self.image_variant == ImageVariant::Slim {
            "dsh-docker:local-slim".into()
        } else if self.image == "ghcr.io/reina0721/dsh-docker:2"
            && self.image_variant == ImageVariant::Slim
        {
            "ghcr.io/reina0721/dsh-docker:2-slim".into()
        } else {
            self.image.clone()
        }
    }

    /// Environment values that must be visible to Docker Compose.
    pub fn compose_env(&self) -> Vec<(String, String)> {
        let mut values = self.env.clone();
        values.insert("DSH_IMAGE".into(), self.effective_image());
        values.insert("DSH_IMAGE_MODE".into(), self.image_mode.to_string());
        values.insert("DSH_IMAGE_VARIANT".into(), self.image_variant.to_string());
        values.insert("DSH_IMAGE_MIRROR".into(), self.image_mirror.to_string());
        let bind_for_compose = if self.bind_address.contains(':')
            && !self.bind_address.trim_start().starts_with('[')
        {
            format!("[{}]", self.bind_address.trim())
        } else {
            self.bind_address.trim().to_owned()
        };
        values.insert("BIND_ADDRESS".into(), bind_for_compose);
        values.insert("DSH_PORT".into(), self.dsh_port.to_string());
        values.insert("DSH_PROXY_PORT".into(), self.proxy_port.to_string());
        values.insert("DSH_INTERNAL_HOST".into(), self.internal_host.clone());
        values.insert("DSH_INTERNAL_PORT".into(), self.internal_port.to_string());
        values.insert("AUTH_REALM".into(), self.auth_realm.clone());
        values.insert("AUTH_BCRYPT_COST".into(), self.bcrypt_cost.to_string());
        values.insert(
            "AUTH_MIN_PASSWORD_LENGTH".into(),
            self.min_password_len.to_string(),
        );
        values.insert("DSH_HTTP_COMPAT_SHIM".into(), self.compat.to_string());
        values.insert("RUNTIME_DIR".into(), self.runtime_dir.display().to_string());
        values.insert("RUNTIME_UID".into(), self.runtime_uid.to_string());
        values.insert("RUNTIME_GID".into(), self.runtime_gid.to_string());
        values.insert("COMPOSE_PROJECT_NAME".into(), self.compose_project_name());
        if let Some(ref name) = self.container_name {
            values.insert("DSH_CONTAINER_NAME".into(), name.clone());
        }
        values.insert("DSH_NETWORK_MODE".into(), self.network_mode.to_string());
        if let Some(ref parent) = self.network_parent {
            values.insert("DSH_NETWORK_PARENT".into(), parent.clone());
        }
        if let Some(ref ip) = self.network_ip {
            values.insert("DSH_NETWORK_IP".into(), ip.clone());
        }
        if let Some(ref subnet) = self.network_subnet {
            values.insert("DSH_NETWORK_SUBNET".into(), subnet.clone());
        }
        if let Some(ref gw) = self.network_gateway {
            values.insert("DSH_NETWORK_GATEWAY".into(), gw.clone());
        }
        values.insert(
            "DSH_NETWORK_IPVLAN_MODE".into(),
            self.network_ipvlan_mode.clone(),
        );
        values.insert(
            "DSH_NETWORK_ATTACH_BRIDGE".into(),
            self.network_attach_bridge.to_string(),
        );
        values.into_iter().collect()
    }

    /// Redacted view used by `dshctl config show`.
    pub fn redacted(&self) -> Self {
        let mut clone = self.clone();
        clone.env = clone
            .env
            .into_iter()
            .map(|(key, value)| {
                if is_secret_key(&key) {
                    (key, "<redacted>".to_owned())
                } else {
                    (key, redact_url_userinfo(&value))
                }
            })
            .collect();
        clone
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn parse_u16(env: &BTreeMap<String, String>, key: &str, default: u16) -> Result<u16> {
    match env.get(key) {
        Some(value) => value
            .trim()
            .parse::<u16>()
            .with_context(|| format!("{key} must be a port number (got {value:?})")),
        None => Ok(default),
    }
}

fn parse_u32(env: &BTreeMap<String, String>, key: &str, default: u32) -> Result<u32> {
    match env.get(key) {
        Some(value) => value
            .trim()
            .parse::<u32>()
            .with_context(|| format!("{key} must be an unsigned integer (got {value:?})")),
        None => Ok(default),
    }
}

fn parse_usize(env: &BTreeMap<String, String>, key: &str, default: usize) -> Result<usize> {
    match env.get(key) {
        Some(value) => value
            .trim()
            .parse::<usize>()
            .with_context(|| format!("{key} must be a positive integer (got {value:?})")),
        None => Ok(default),
    }
}

fn parse_bool(env: &BTreeMap<String, String>, key: &str, default: bool) -> Result<bool> {
    match env.get(key) {
        Some(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => bail!("{key} must be a boolean (got {other:?})"),
        },
        None => Ok(default),
    }
}

fn is_known_key(key: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "DSH_", "RUNTIME_", "AUTH_", "COMPOSE_", "BUILD_", "NODE_", "GO_", "RUST_", "PNPM_",
        "INSTALL_", "APT_", "NPM_", "PIP_", "CARGO_", "RUSTUP_", "DOCKER_",
    ];
    key == "BIND_ADDRESS"
        || key == "HTTP_PROXY"
        || key == "HTTPS_PROXY"
        || key == "NO_PROXY"
        || PREFIXES.iter().any(|p| key.starts_with(p))
}

fn is_secret_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.contains("PASSWORD")
        || upper.contains("TOKEN")
        || upper.contains("SECRET")
        || upper.contains("PRIVATE")
}

fn redact_url_userinfo(value: &str) -> String {
    let Some(scheme_end) = value.find("://") else {
        return value.to_owned();
    };
    let after_scheme = &value[scheme_end + 3..];
    let Some(at) = after_scheme.find('@') else {
        return value.to_owned();
    };
    let authority = &after_scheme[..at];
    if authority.contains('.') || authority.contains(':') {
        format!(
            "{}://<redacted>@{}",
            &value[..scheme_end],
            &after_scheme[at + 1..]
        )
    } else {
        value.to_owned()
    }
}

/// Parse a dotenv file without executing it.
///
/// The parser is deliberately conservative: keys must be shell identifiers,
/// surrounding quotes are removed, and no variable interpolation is performed.
pub fn parse_dotenv(input: &str) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for raw_line in input.lines() {
        let line = raw_line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !valid_key(key) {
            continue;
        }
        let value = value.trim();
        let value = strip_quotes(value);
        result.insert(key.to_owned(), value.to_owned());
    }
    result
}

fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn strip_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return &value[1..value.len() - 1];
        }
    }
    value
}

/// Validate Docker container name according to Docker daemon rules:
/// Must start with an alphanumeric character, followed by alphanumeric, `_`, `.`, or `-`.
pub fn validate_container_name(name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        bail!("DSH_CONTAINER_NAME must not be empty");
    }
    if name.len() < 2 {
        bail!("DSH_CONTAINER_NAME must be at least 2 characters long (got {name:?})");
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphanumeric() {
        bail!("DSH_CONTAINER_NAME must start with an alphanumeric character (got {name:?})");
    }
    for c in chars {
        if !c.is_ascii_alphanumeric() && c != '_' && c != '.' && c != '-' {
            bail!(
                "DSH_CONTAINER_NAME can only contain alphanumeric characters, '_', '.', or '-' (got {name:?})"
            );
        }
    }
    Ok(())
}

fn sanitize_project_name(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() {
        "dsh-docker".to_owned()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn parses_quotes_export_and_crlf() {
        let parsed = parse_dotenv(
            "\u{feff}export FOO=\"bar baz\"\r\n# comment\r\nQUX='one'\r\nEMPTY=\r\nBAD-KEY=x\r\n",
        );
        assert_eq!(parsed.get("FOO").map(String::as_str), Some("bar baz"));
        assert_eq!(parsed.get("QUX").map(String::as_str), Some("one"));
        assert_eq!(parsed.get("EMPTY").map(String::as_str), Some(""));
        assert!(!parsed.contains_key("BAD-KEY"));
    }

    #[test]
    fn loads_typed_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::from_map(dir.path(), BTreeMap::new()).unwrap();
        assert_eq!(cfg.dsh_port, 3080);
        assert_eq!(cfg.image, DEFAULT_IMAGE);
        assert_eq!(cfg.image_mode, ImageMode::Build);
        assert_eq!(
            cfg.auth_file(),
            dir.path().join(".runtime/auth/users.htpasswd")
        );
    }

    #[test]
    fn process_values_win() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::from_map(
            dir.path(),
            map(&[("DSH_PORT", "4000"), ("DSH_IMAGE_MODE", "pull")]),
        )
        .unwrap();
        assert_eq!(cfg.dsh_port, 4000);
        assert_eq!(cfg.image_mode, ImageMode::Pull);
    }

    #[test]
    fn rejects_invalid_ports_and_realm() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Config::from_map(dir.path(), map(&[("DSH_PORT", "0")])).is_err());
        assert!(Config::from_map(dir.path(), map(&[("DSH_PORT", "70000")])).is_err());
        assert!(Config::from_map(dir.path(), map(&[("AUTH_REALM", "a\nb")])).is_err());
        assert!(Config::from_map(dir.path(), map(&[("AUTH_MIN_PASSWORD_LENGTH", "0")])).is_err());
        assert!(Config::from_map(dir.path(), map(&[("AUTH_MIN_PASSWORD_LENGTH", "6")])).is_ok());
        assert!(Config::from_map(dir.path(), map(&[("AUTH_MIN_PASSWORD_LENGTH", "73")])).is_err());
    }

    #[test]
    fn redacts_secrets_and_proxy_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::from_map(
            dir.path(),
            map(&[
                ("NPM_TOKEN", "secret"),
                ("HTTPS_PROXY", "http://user:pass@proxy:8080"),
            ]),
        )
        .unwrap();
        let redacted = cfg.redacted();
        assert_eq!(redacted.env.get("NPM_TOKEN").unwrap(), "<redacted>");
        assert_eq!(
            redacted.env.get("HTTPS_PROXY").unwrap(),
            "http://<redacted>@proxy:8080"
        );
    }

    #[test]
    fn sanitizes_project_names() {
        assert_eq!(sanitize_project_name("My Repo!"), "my-repo");
        assert_eq!(sanitize_project_name("..."), "dsh-docker");
    }

    #[test]
    fn validates_container_name() {
        assert!(validate_container_name("my-container").is_ok());
        assert!(validate_container_name("dsh_box.1").is_ok());
        assert!(validate_container_name("a").is_err());
        assert!(validate_container_name("-invalid").is_err());
        assert!(validate_container_name("bad name").is_err());
        assert!(validate_container_name("bad/name").is_err());
    }

    #[test]
    fn container_name_config() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::from_map(dir.path(), BTreeMap::new()).unwrap();
        assert_eq!(cfg.container_name, None);

        let cfg =
            Config::from_map(dir.path(), map(&[("DSH_CONTAINER_NAME", "custom-box")])).unwrap();
        assert_eq!(cfg.container_name.as_deref(), Some("custom-box"));
        let env_map: BTreeMap<String, String> = cfg.compose_env().into_iter().collect();
        assert_eq!(
            env_map.get("DSH_CONTAINER_NAME").map(String::as_str),
            Some("custom-box")
        );
    }

    #[test]
    fn network_mode_defaults_and_validation() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::from_map(dir.path(), BTreeMap::new()).unwrap();
        assert_eq!(cfg.network_mode, NetworkMode::Bridge);
        assert_eq!(cfg.network_ip, None);

        // Missing required fields for ipvlan
        assert!(Config::from_map(dir.path(), map(&[("DSH_NETWORK_MODE", "ipvlan")])).is_err());

        // Valid ipvlan config
        let cfg = Config::from_map(
            dir.path(),
            map(&[
                ("DSH_NETWORK_MODE", "ipvlan"),
                ("DSH_NETWORK_PARENT", "eth0"),
                ("DSH_NETWORK_IP", "198.51.100.2"),
                ("DSH_NETWORK_SUBNET", "198.51.100.0/24"),
                ("DSH_NETWORK_GATEWAY", "198.51.100.1"),
            ]),
        )
        .unwrap();
        assert_eq!(cfg.network_mode, NetworkMode::Ipvlan);
        assert_eq!(cfg.network_ip.as_deref(), Some("198.51.100.2"));
        assert_eq!(cfg.network_parent.as_deref(), Some("eth0"));
        assert_eq!(cfg.network_ipvlan_mode, "l2");
        assert!(cfg.network_attach_bridge);

        let env_map: BTreeMap<String, String> = cfg.compose_env().into_iter().collect();
        assert_eq!(
            env_map.get("DSH_NETWORK_MODE").map(String::as_str),
            Some("ipvlan")
        );
        assert_eq!(
            env_map.get("DSH_NETWORK_IP").map(String::as_str),
            Some("198.51.100.2")
        );

        // Auto-derived parent, subnet and gateway from DSH_NETWORK_IP alone
        let cfg = Config::from_map(dir.path(), map(&[("DSH_NETWORK_IP", "198.51.100.2")])).unwrap();
        assert_eq!(cfg.network_mode, NetworkMode::Ipvlan);
        assert_eq!(cfg.network_ip.as_deref(), Some("198.51.100.2"));
        assert_eq!(cfg.network_parent.as_deref(), Some("eth0"));
        assert_eq!(cfg.network_subnet.as_deref(), Some("198.51.100.0/24"));
        assert_eq!(cfg.network_gateway.as_deref(), Some("198.51.100.1"));

        // Invalid IP or CIDR
        assert!(
            Config::from_map(
                dir.path(),
                map(&[
                    ("DSH_NETWORK_MODE", "ipvlan"),
                    ("DSH_NETWORK_PARENT", "eth0"),
                    ("DSH_NETWORK_IP", "not-an-ip"),
                    ("DSH_NETWORK_SUBNET", "198.51.100.0/24"),
                    ("DSH_NETWORK_GATEWAY", "198.51.100.1"),
                ])
            )
            .is_err()
        );
        assert!(
            Config::from_map(
                dir.path(),
                map(&[
                    ("DSH_NETWORK_MODE", "ipvlan"),
                    ("DSH_NETWORK_PARENT", "eth0"),
                    ("DSH_NETWORK_IP", "198.51.100.2"),
                    ("DSH_NETWORK_SUBNET", "198.51.100.0"), // Missing CIDR slash
                    ("DSH_NETWORK_GATEWAY", "198.51.100.1"),
                ])
            )
            .is_err()
        );
    }
}
