// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    str::FromStr,
};

use anyhow::{Context, Result, bail};

const DEFAULT_AUTH_FILE: &str = "/home/node/.dsh-auth/users.htpasswd";
const DEFAULT_LEGACY_AUTH_FILE: &str = "/home/node/.dsh-auth/users.caddy";

/// Configuration for the in-container gateway.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub listen_host: String,
    pub listen_port: u16,
    pub upstream_host: String,
    pub upstream_port: u16,
    pub auth_file: PathBuf,
    pub legacy_auth_file: PathBuf,
    pub realm: String,
    pub compat: bool,
    pub max_html_bytes: usize,
    pub log_level: String,
    pub log_format: String,
    /// Token printed by `dsh web`, injected into upstream requests so the
    /// browser does not need to reopen the tokenized URL manually.
    pub web_token: Option<String>,
}

impl GatewayConfig {
    pub fn from_env() -> Result<Self> {
        let listen_host = env::var("DSH_PROXY_HOST").unwrap_or_else(|_| "0.0.0.0".to_owned());
        let listen_port = parse_u16("DSH_PROXY_PORT", 3080)?;
        let upstream_host =
            env::var("DSH_INTERNAL_HOST").unwrap_or_else(|_| "127.0.0.1".to_owned());
        let upstream_port = parse_u16("DSH_INTERNAL_PORT", 3079)?;
        let auth_file = env::var("DSH_AUTH_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_AUTH_FILE));
        let legacy_auth_file = auth_file
            .parent()
            .map(|parent| parent.join("users.caddy"))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_LEGACY_AUTH_FILE));
        let realm = env::var("AUTH_REALM").unwrap_or_else(|_| "DeepSeek Harness".to_owned());
        let compat = parse_bool("DSH_HTTP_COMPAT_SHIM", true)?;
        let max_html_bytes = env::var("DSH_GATEWAY_MAX_HTML_BYTES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(8 * 1024 * 1024)
            .max(1024);
        let log_level = env::var("DSH_LOG_LEVEL").unwrap_or_else(|_| "info".to_owned());
        let log_format = env::var("DSH_LOG_FORMAT").unwrap_or_else(|_| "text".to_owned());
        let web_token = env::var("DSH_WEB_TOKEN")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let config = Self {
            listen_host,
            listen_port,
            upstream_host,
            upstream_port,
            auth_file,
            legacy_auth_file,
            realm,
            compat,
            max_html_bytes,
            log_level,
            log_format,
            web_token,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn for_test(
        listen: SocketAddr,
        upstream: SocketAddr,
        auth_file: impl Into<PathBuf>,
    ) -> Self {
        let auth_file = auth_file.into();
        Self {
            listen_host: listen.ip().to_string(),
            listen_port: listen.port(),
            upstream_host: upstream.ip().to_string(),
            upstream_port: upstream.port(),
            legacy_auth_file: auth_file
                .parent()
                .map(|parent| parent.join("users.caddy"))
                .unwrap_or_else(|| PathBuf::from("users.caddy")),
            auth_file,
            realm: "DeepSeek Harness".to_owned(),
            compat: true,
            max_html_bytes: 8 * 1024 * 1024,
            log_level: "info".to_owned(),
            log_format: "text".to_owned(),
            web_token: None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        self.listen_addr()?;
        self.upstream_addr()?;
        if self.realm.len() > 128 || self.realm.contains(['\r', '\n']) {
            bail!("AUTH_REALM must be a single line of at most 128 characters");
        }
        if !matches!(self.log_format.as_str(), "text" | "json") {
            bail!("DSH_LOG_FORMAT must be text or json");
        }
        if let Some(token) = &self.web_token {
            if token.len() > 512
                || !token
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~'))
            {
                bail!("DSH_WEB_TOKEN contains invalid characters");
            }
        }
        Ok(())
    }

    pub fn listen_addr(&self) -> Result<SocketAddr> {
        parse_socket_addr(&self.listen_host, self.listen_port).with_context(|| {
            format!(
                "invalid gateway listen address {}:{}",
                self.listen_host, self.listen_port
            )
        })
    }

    pub fn upstream_addr(&self) -> Result<SocketAddr> {
        parse_socket_addr(&self.upstream_host, self.upstream_port).with_context(|| {
            format!(
                "invalid DSH upstream address {}:{}",
                self.upstream_host, self.upstream_port
            )
        })
    }

    pub fn internal_authority(&self) -> String {
        format!("localhost:{}", self.upstream_port)
    }

    pub fn internal_origin(&self) -> String {
        format!("http://localhost:{}", self.upstream_port)
    }
}

fn parse_u16(name: &str, default: u16) -> Result<u16> {
    match env::var(name) {
        Ok(value) => value
            .trim()
            .parse::<u16>()
            .with_context(|| format!("{name} must be a port number (got {value:?})")),
        Err(_) => Ok(default),
    }
}

fn parse_bool(name: &str, default: bool) -> Result<bool> {
    match env::var(name) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => bail!("{name} must be a boolean (got {other:?})"),
        },
        Err(_) => Ok(default),
    }
}

fn parse_socket_addr(host: &str, port: u16) -> Result<SocketAddr> {
    let host = host.trim();
    if let Ok(ip) = IpAddr::from_str(host) {
        return Ok(SocketAddr::new(ip, port));
    }
    // Accept bracketed IPv6 literals as a convenience.
    let trimmed = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = IpAddr::from_str(trimmed) {
        return Ok(SocketAddr::new(ip, port));
    }
    bail!("expected an IP literal, got {host:?}")
}

/// Default listen address used by tests and standalone runs.
pub fn default_listen_addr() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 3080)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ips_and_ports() {
        assert_eq!(
            parse_socket_addr("127.0.0.1", 3080).unwrap(),
            "127.0.0.1:3080".parse().unwrap()
        );
        assert_eq!(
            parse_socket_addr("[::1]", 3080).unwrap(),
            "[::1]:3080".parse().unwrap()
        );
        assert!(parse_socket_addr("localhost", 3080).is_err());
    }
}
