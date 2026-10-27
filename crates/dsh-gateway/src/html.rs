// SPDX-License-Identifier: MIT OR Apache-2.0

/// Path used by the browser compatibility shim.
pub const COMPAT_PATH: &str = "/__dsh_http_compat.js";

/// Browser shim embedded at compile time so the image has no loose JS asset.
pub const COMPAT_SCRIPT: &str = include_str!("../../../assets/dsh-http-compat.js");

/// Inject the compatibility shim into an HTML document.
///
/// The order matches the old Node proxy: before `</head>`, otherwise after the
/// opening `<body ...>` tag, otherwise at the start of the document.
pub fn inject_compat(html: &str) -> String {
    if html.contains(COMPAT_PATH) {
        return html.to_owned();
    }
    let tag = format!("<script src=\"{COMPAT_PATH}\"></script>");

    if let Some(index) = find_ascii_case_insensitive(html, "</head>") {
        let mut out = String::with_capacity(html.len() + tag.len());
        out.push_str(&html[..index]);
        out.push_str(&tag);
        out.push_str(&html[index..]);
        return out;
    }

    if let Some(body) = find_body_tag_end(html) {
        let mut out = String::with_capacity(html.len() + tag.len());
        out.push_str(&html[..body]);
        out.push_str(&tag);
        out.push_str(&html[body..]);
        return out;
    }

    let mut out = String::with_capacity(html.len() + tag.len());
    out.push_str(&tag);
    out.push_str(html);
    out
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

fn find_body_tag_end(html: &str) -> Option<usize> {
    let start = find_ascii_case_insensitive(html, "<body")?;
    let bytes = html.as_bytes();
    let after = start + 5;
    if after < bytes.len() && !(bytes[after] == b'>' || bytes[after].is_ascii_whitespace()) {
        return None;
    }
    let end = html[after..].find('>')?;
    Some(after + end + 1)
}

/// True when the content type names `text/html`.
pub fn is_html_content_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .map(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
        .unwrap_or(false)
}

/// True when the content type has no charset or an explicit UTF-8 charset.
pub fn is_utf8_or_unspecified_charset(value: &str) -> bool {
    for parameter in value.split(';').skip(1) {
        let parameter = parameter.trim();
        if let Some((key, value)) = parameter.split_once('=') {
            if key.trim().eq_ignore_ascii_case("charset") {
                let charset = value.trim().trim_matches('"');
                return charset.eq_ignore_ascii_case("utf-8")
                    || charset.eq_ignore_ascii_case("utf8");
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_before_head() {
        let html = "<html><head><title>x</title></head><body>hi</body></html>";
        let out = inject_compat(html);
        assert!(out.contains(&format!("<script src=\"{COMPAT_PATH}\"></script></head>")));
    }

    #[test]
    fn injects_after_body_when_no_head() {
        let html = "<html><BODY class=\"x\"><p>hi</p></BODY></html>";
        let out = inject_compat(html);
        assert!(out.contains("<BODY class=\"x\"><script"));
    }

    #[test]
    fn prepends_when_no_head_or_body() {
        let out = inject_compat("<p>hi</p>");
        assert!(out.starts_with("<script"));
    }

    #[test]
    fn does_not_duplicate() {
        let html = "<head><script src=\"/__dsh_http_compat.js\"></script></head>";
        assert_eq!(inject_compat(html), html);
    }

    #[test]
    fn handles_case_insensitive_tags() {
        let out = inject_compat("<HTML><HEAD></HEAD><BODY></BODY></HTML>");
        assert!(out.contains("<script"));
        assert!(out.contains("</HEAD>"));
    }

    #[test]
    fn detects_content_types() {
        assert!(is_html_content_type("text/html; charset=utf-8"));
        assert!(is_html_content_type("TEXT/HTML"));
        assert!(!is_html_content_type("application/json"));
        assert!(is_utf8_or_unspecified_charset("text/html"));
        assert!(is_utf8_or_unspecified_charset("text/html; charset=UTF-8"));
        assert!(!is_utf8_or_unspecified_charset("text/html; charset=gbk"));
    }
}
