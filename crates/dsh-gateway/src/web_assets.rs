// SPDX-License-Identifier: MIT OR Apache-2.0

use bytes::Bytes;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "web/dist"]
pub struct WebAssets;

/// Retrieve embedded asset mime type and bytes.
pub fn get_asset(path: &str) -> Option<(&'static str, Bytes)> {
    let raw = path.trim_start_matches('/');
    let target = if let Some(stripped) = raw.strip_prefix("_dsh_login/") {
        stripped
    } else if let Some(stripped) = raw.strip_prefix("login/assets/") {
        stripped
    } else if raw.is_empty() {
        "index.html"
    } else {
        raw
    };

    let file = WebAssets::get(target)?;
    let mime = match target.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    };

    let bytes = Bytes::copy_from_slice(&file.data);
    Some((mime, bytes))
}

pub fn get_index_html() -> Option<(&'static str, Bytes)> {
    get_asset("index.html")
}
