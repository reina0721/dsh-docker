// SPDX-License-Identifier: MIT OR Apache-2.0

use std::str::FromStr;

use http::{HeaderMap, Uri, uri::Authority};

/// Reason a request was rejected before it reached DSH.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    InvalidHost,
    CrossSite,
    OriginHostMismatch,
    InvalidOrigin,
}

impl Reject {
    pub fn reason(self) -> &'static str {
        match self {
            Self::InvalidHost => "invalid-host",
            Self::CrossSite => "cross-site",
            Self::OriginHostMismatch => "origin-host-mismatch",
            Self::InvalidOrigin => "invalid-origin",
        }
    }
}

/// Validate the external Host/Origin pair and return the external authority.
///
/// This mirrors the old Node proxy contract exactly: Host is a bare authority;
/// `Sec-Fetch-Site: cross-site` is rejected; if Origin is present, its authority
/// must equal Host's authority.  The scheme is intentionally not compared.
pub fn validate(headers: &HeaderMap) -> Result<String, Reject> {
    validate_with_policy(headers, false)
}

/// Validate with an option to permit cross-site navigation (specifically for OAuth callback redirects).
pub fn validate_with_policy(headers: &HeaderMap, allow_cross_site: bool) -> Result<String, Reject> {
    let host = single_header(headers, "host").ok_or(Reject::InvalidHost)?;
    let host_authority = parse_authority(host).ok_or(Reject::InvalidHost)?;

    if !allow_cross_site {
        if let Some(site) = single_header(headers, "sec-fetch-site") {
            if site.eq_ignore_ascii_case("cross-site") {
                return Err(Reject::CrossSite);
            }
        }
    }

    if let Some(origin) = single_header(headers, "origin") {
        let origin_uri = origin.parse::<Uri>().map_err(|_| Reject::InvalidOrigin)?;
        if origin_uri.scheme().is_none() || origin_uri.authority().is_none() {
            return Err(Reject::InvalidOrigin);
        }
        let origin_authority = origin_uri.authority().expect("checked above");
        if !origin_authority
            .as_str()
            .eq_ignore_ascii_case(host_authority.as_str())
        {
            return Err(Reject::OriginHostMismatch);
        }
    }

    Ok(host.to_owned())
}

/// Extract a single header value as UTF-8.
pub fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    value.to_str().ok()
}

fn parse_authority(value: &str) -> Option<Authority> {
    if value.is_empty()
        || value.len() > 255
        || value.contains(['@', '/', '?', '#', '\\'])
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return None;
    }
    let authority = Authority::from_str(value).ok()?;
    if authority.host().is_empty() {
        return None;
    }
    Some(authority)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::HeaderValue;

    fn headers(host: &str, origin: Option<&str>, site: Option<&str>) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert("host", HeaderValue::from_str(host).unwrap());
        if let Some(origin) = origin {
            map.insert("origin", HeaderValue::from_str(origin).unwrap());
        }
        if let Some(site) = site {
            map.insert("sec-fetch-site", HeaderValue::from_str(site).unwrap());
        }
        map
    }

    #[test]
    fn accepts_dynamic_authority() {
        assert_eq!(
            validate(&headers(
                "example.com:8443",
                Some("https://example.com:8443"),
                None
            ))
            .unwrap(),
            "example.com:8443"
        );
        assert_eq!(
            validate(&headers("127.0.0.1:3080", None, None)).unwrap(),
            "127.0.0.1:3080"
        );
    }

    #[test]
    fn rejects_bad_hosts() {
        for host in ["", "user@host", "host/path", "host?x", "host#x", "a b"] {
            assert_eq!(
                validate(&headers(host, None, None)),
                Err(Reject::InvalidHost),
                "{host}"
            );
        }
        let mut map = HeaderMap::new();
        assert_eq!(validate(&map), Err(Reject::InvalidHost));
        map.insert("host", HeaderValue::from_static("example.com"));
        assert!(validate(&map).is_ok());
    }

    #[test]
    fn rejects_cross_site_and_mismatched_origins() {
        assert_eq!(
            validate(&headers("example.com", None, Some("cross-site"))),
            Err(Reject::CrossSite)
        );
        assert_eq!(
            validate_with_policy(&headers("example.com", None, Some("cross-site")), true).unwrap(),
            "example.com"
        );
        assert_eq!(
            validate(&headers("example.com", Some("https://other.com"), None)),
            Err(Reject::OriginHostMismatch)
        );
        assert_eq!(
            validate(&headers("example.com", Some("not a uri"), None)),
            Err(Reject::InvalidOrigin)
        );
        assert_eq!(
            validate(&headers("example.com", Some("null"), None)),
            Err(Reject::InvalidOrigin)
        );
    }

    #[test]
    fn compares_authority_not_scheme() {
        assert!(validate(&headers("example.com", Some("http://example.com"), None)).is_ok());
        assert!(validate(&headers("example.com", Some("https://example.com"), None)).is_ok());
    }
}
