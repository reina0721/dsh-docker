// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{borrow::Cow, convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};

use base64::Engine;
use bytes::Bytes;
use http::{
    HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, Uri,
    header::{
        ACCEPT_ENCODING, AUTHORIZATION, CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE,
        COOKIE, ETAG, HOST, LAST_MODIFIED, ORIGIN, PROXY_AUTHORIZATION, SET_COOKIE, UPGRADE,
    },
};
use http_body_util::{BodyExt, Full, Limited, combinators::UnsyncBoxBody};
use hyper::body::Incoming;
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use tokio::{io::copy_bidirectional, sync::RwLock};
use tracing::{debug, warn};

use crate::{
    auth::{AuthOutcome, AuthStore},
    config::GatewayConfig,
    html, trust,
};

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type ProxyBody = UnsyncBoxBody<Bytes, BoxError>;

const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// Shared gateway state.
pub struct Gateway {
    config: GatewayConfig,
    auth: AuthStore,
    client: Client<HttpConnector, ProxyBody>,
    /// Stable DSH web cookie captured after the token exchange.
    web_cookie: RwLock<Option<String>>,
}

impl Gateway {
    pub fn new(config: GatewayConfig) -> Self {
        let auth = AuthStore::new(
            config.auth_file.clone(),
            config.legacy_auth_file.clone(),
            Duration::from_secs(1),
        );
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(Duration::from_secs(5)));
        connector.set_nodelay(true);
        let client = Client::builder(TokioExecutor::new()).build(connector);
        tracing::info!(
            listen = %format!("{}:{}", config.listen_host, config.listen_port),
            upstream = %format!("{}:{}", config.upstream_host, config.upstream_port),
            users = auth.user_count(),
            compat = config.compat,
            "gateway initialized"
        );
        Self {
            config,
            auth,
            client,
            web_cookie: RwLock::new(None),
        }
    }

    pub fn config(&self) -> &GatewayConfig {
        &self.config
    }

    pub async fn handle(
        self: Arc<Self>,
        req: Request<Incoming>,
        peer: SocketAddr,
    ) -> Response<ProxyBody> {
        let method = req.method().clone();
        let path = req.uri().path().to_owned();

        if method == Method::CONNECT {
            return text_response(
                StatusCode::METHOD_NOT_ALLOWED,
                "method not allowed\n",
                HeaderMap::new(),
            );
        }
        if req.uri().scheme().is_some() || req.uri().authority().is_some() {
            return text_response(StatusCode::BAD_REQUEST, "bad request\n", HeaderMap::new());
        }

        // OAuth callback endpoint from upstream identity providers (e.g. platform.deepseek.com).
        // Identity provider redirects are top-level cross-site navigations that do not carry Basic Auth headers.
        // Upstream DSH strictly validates PKCE code challenge and cryptographically random state.
        let is_oauth_callback = method == Method::GET && path == "/oauth/callback";

        if !is_oauth_callback {
            let outcome = match basic_credentials(req.headers()) {
                Some((username, password)) => self.auth.authorize(&username, &password).await,
                None => AuthOutcome::Denied,
            };
            match outcome {
                AuthOutcome::Allowed => {}
                AuthOutcome::Denied => return self.unauthorized(),
                AuthOutcome::NoUsers => {
                    return text_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "no authentication users configured\n",
                        HeaderMap::new(),
                    );
                }
            }
        }

        let external_host = match trust::validate_with_policy(req.headers(), is_oauth_callback) {
            Ok(host) => host,
            Err(reject) => return self.forbidden(reject),
        };

        if path == html::COMPAT_PATH {
            if !self.config.compat {
                return text_response(
                    StatusCode::NOT_FOUND,
                    "compat shim disabled\n",
                    HeaderMap::new(),
                );
            }
            let mut headers = HeaderMap::new();
            headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/javascript; charset=utf-8"),
            );
            headers.insert(
                http::header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            );
            headers.insert(
                http::header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            return with_headers(
                StatusCode::OK,
                headers,
                Bytes::from_static(html::COMPAT_SCRIPT.as_bytes()),
            );
        }

        let is_upgrade = is_upgrade_request(req.headers());
        let response = if is_upgrade {
            self.handle_upgrade(req, peer, &external_host).await
        } else {
            self.handle_http(req, peer, &external_host).await
        };
        debug!(
            method = %method,
            path = %path,
            status = response.status().as_u16(),
            peer = %peer,
            "request completed"
        );
        response
    }

    async fn handle_http(
        &self,
        req: Request<Incoming>,
        peer: SocketAddr,
        external_host: &str,
    ) -> Response<ProxyBody> {
        let (parts, body) = req.into_parts();
        let stored_cookie = self.web_cookie.read().await.clone();
        let path_and_query = self.with_web_token(
            parts
                .uri
                .path_and_query()
                .map(|value| value.as_str())
                .unwrap_or("/"),
            stored_cookie.is_none(),
        );
        let upstream_uri = match self.upstream_uri(&path_and_query) {
            Ok(uri) => uri,
            Err(()) => {
                return text_response(StatusCode::BAD_REQUEST, "bad request\n", HeaderMap::new());
            }
        };
        let mut headers = self.internalize(&parts, peer, external_host, false);
        apply_dsh_cookie(&mut headers, stored_cookie.as_deref());
        let body = body
            .map_err(|error| -> BoxError { Box::new(error) })
            .boxed_unsync();
        let mut upstream_request = Request::new(body);
        *upstream_request.method_mut() = parts.method.clone();
        *upstream_request.uri_mut() = upstream_uri;
        *upstream_request.headers_mut() = headers;

        let response = match self.client.request(upstream_request).await {
            Ok(response) => response,
            Err(error) => {
                warn!(error = %error, "upstream request failed");
                return text_response(
                    StatusCode::BAD_GATEWAY,
                    "upstream request failed\n",
                    HeaderMap::new(),
                );
            }
        };
        if let Some(cookie) = dsh_cookie_from_headers(response.headers()) {
            *self.web_cookie.write().await = Some(cookie);
        } else if response.status() == StatusCode::UNAUTHORIZED && stored_cookie.is_some() {
            // The cached DSH cookie expired or DSH restarted underneath us;
            // force the next reload back through the token exchange.
            *self.web_cookie.write().await = None;
        }

        self.rewrite_response(parts.method, response).await
    }

    async fn rewrite_response(
        &self,
        method: Method,
        response: Response<Incoming>,
    ) -> Response<ProxyBody> {
        let (mut parts, body) = response.into_parts();
        strip_hop_by_hop(&mut parts.headers);

        let candidate = method == Method::GET
            && self.config.compat
            && !matches!(
                parts.status,
                StatusCode::NO_CONTENT
                    | StatusCode::NOT_MODIFIED
                    | StatusCode::PARTIAL_CONTENT
                    | StatusCode::SWITCHING_PROTOCOLS
            )
            && html_candidate(&parts.headers);

        if !candidate {
            return Response::from_parts(
                parts,
                body.map_err(|error| -> BoxError { Box::new(error) })
                    .boxed_unsync(),
            );
        }

        let content_type = parts
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let limited = Limited::new(body, self.config.max_html_bytes);
        let collected = match limited.collect().await {
            Ok(collected) => collected,
            Err(error) => {
                warn!(error = %error, "HTML response exceeded the compatibility proxy limit");
                return text_response(
                    StatusCode::BAD_GATEWAY,
                    "upstream HTML response exceeded the compatibility proxy limit\n",
                    HeaderMap::new(),
                );
            }
        };
        let bytes = collected.to_bytes();

        // A non-UTF-8 or explicitly non-UTF-8 document must pass through
        // untouched; injecting into a different encoding would corrupt it.
        if !html::is_utf8_or_unspecified_charset(&content_type) {
            return Response::from_parts(parts, full_body(bytes));
        }
        let html = match std::str::from_utf8(&bytes) {
            Ok(html) => html,
            Err(_) => return Response::from_parts(parts, full_body(bytes)),
        };

        let injected = html::inject_compat(html);
        let length = injected.len();
        parts.headers.remove(CONTENT_LENGTH);
        parts.headers.remove(CONTENT_ENCODING);
        parts.headers.remove(ETAG);
        parts.headers.remove(LAST_MODIFIED);
        if let Ok(value) = HeaderValue::from_str(&length.to_string()) {
            parts.headers.insert(CONTENT_LENGTH, value);
        }
        parts
            .headers
            .insert("x-dsh-http-compat", HeaderValue::from_static("active"));
        Response::from_parts(parts, full_body(Bytes::from(injected)))
    }

    async fn handle_upgrade(
        &self,
        mut req: Request<Incoming>,
        peer: SocketAddr,
        external_host: &str,
    ) -> Response<ProxyBody> {
        let client_upgrade = hyper::upgrade::on(&mut req);
        let (parts, _body) = req.into_parts();
        let stored_cookie = self.web_cookie.read().await.clone();
        let path_and_query = self.with_web_token(
            parts
                .uri
                .path_and_query()
                .map(|value| value.as_str())
                .unwrap_or("/"),
            stored_cookie.is_none(),
        );
        let upstream_uri = match self.upstream_uri(&path_and_query) {
            Ok(uri) => uri,
            Err(()) => {
                return text_response(StatusCode::BAD_REQUEST, "bad request\n", HeaderMap::new());
            }
        };
        let mut headers = self.internalize(&parts, peer, external_host, true);
        apply_dsh_cookie(&mut headers, stored_cookie.as_deref());

        let mut upstream_request = Request::new(full_body(Bytes::new()));
        *upstream_request.method_mut() = parts.method.clone();
        *upstream_request.uri_mut() = upstream_uri;
        *upstream_request.headers_mut() = headers;

        let mut upstream_response = match self.client.request(upstream_request).await {
            Ok(response) => response,
            Err(error) => {
                warn!(error = %error, "upstream WebSocket request failed");
                return text_response(
                    StatusCode::BAD_GATEWAY,
                    "upstream upgrade failed\n",
                    HeaderMap::new(),
                );
            }
        };
        if let Some(cookie) = dsh_cookie_from_headers(upstream_response.headers()) {
            *self.web_cookie.write().await = Some(cookie);
        }

        if upstream_response.status() != StatusCode::SWITCHING_PROTOCOLS {
            let (mut parts, body) = upstream_response.into_parts();
            strip_hop_by_hop(&mut parts.headers);
            return Response::from_parts(
                parts,
                body.map_err(|error| -> BoxError { Box::new(error) })
                    .boxed_unsync(),
            );
        }

        let upstream_upgrade = hyper::upgrade::on(&mut upstream_response);
        let (parts, _body) = upstream_response.into_parts();
        tokio::spawn(async move {
            match tokio::try_join!(client_upgrade, upstream_upgrade) {
                Ok((client, upstream)) => {
                    let mut client = hyper_util::rt::TokioIo::new(client);
                    let mut upstream = hyper_util::rt::TokioIo::new(upstream);
                    if let Err(error) = copy_bidirectional(&mut client, &mut upstream).await {
                        debug!(error = %error, "WebSocket copy ended with an error");
                    }
                }
                Err(error) => debug!(error = %error, "WebSocket upgrade failed"),
            }
        });

        Response::from_parts(parts, full_body(Bytes::new()))
    }

    /// `dsh web` protects its UI with a per-process token. The supervisor
    /// captures it from the startup line and the gateway appends it until the
    /// token exchange returns a stable `dsh-auth-*` cookie.
    fn with_web_token<'a>(&'a self, path_and_query: &'a str, allow_token: bool) -> Cow<'a, str> {
        let Some(token) = self.config.web_token.as_deref() else {
            return Cow::Borrowed(path_and_query);
        };
        if !allow_token {
            return Cow::Borrowed(path_and_query);
        }
        let (_, query) = path_and_query
            .split_once('?')
            .unwrap_or((path_and_query, ""));
        if query
            .split('&')
            .any(|part| part.split('=').next() == Some("token"))
        {
            return Cow::Borrowed(path_and_query);
        }
        let separator = if query.is_empty() {
            if path_and_query.ends_with('?') {
                ""
            } else {
                "?"
            }
        } else {
            "&"
        };
        Cow::Owned(format!("{path_and_query}{separator}token={token}"))
    }

    fn upstream_uri(&self, path_and_query: &str) -> Result<Uri, ()> {
        let authority = if self.config.upstream_host.contains(':') {
            format!(
                "[{}]:{}",
                self.config.upstream_host, self.config.upstream_port
            )
        } else {
            format!(
                "{}:{}",
                self.config.upstream_host, self.config.upstream_port
            )
        };
        Uri::builder()
            .scheme("http")
            .authority(authority)
            .path_and_query(path_and_query)
            .build()
            .map_err(|_| ())
    }

    fn internalize(
        &self,
        parts: &http::request::Parts,
        peer: SocketAddr,
        external_host: &str,
        upgrade: bool,
    ) -> HeaderMap {
        let connection_tokens = connection_tokens(&parts.headers);
        let mut headers = HeaderMap::new();

        for (name, value) in parts.headers.iter() {
            if name == HOST
                || name == ORIGIN
                || name == AUTHORIZATION
                || name == PROXY_AUTHORIZATION
            {
                continue;
            }
            if name == ACCEPT_ENCODING && self.config.compat && !upgrade {
                continue;
            }
            let dynamic_hop = connection_tokens
                .iter()
                .any(|token| name.as_str().eq_ignore_ascii_case(token));
            if is_hop_by_hop(name.as_str()) || dynamic_hop {
                if upgrade && (name == CONNECTION || name == UPGRADE) {
                    headers.append(name, value.clone());
                }
                continue;
            }
            headers.append(name, value.clone());
        }

        if let Ok(value) = HeaderValue::from_str(&self.config.internal_authority()) {
            headers.insert(HOST, value);
        }
        if parts.headers.contains_key(ORIGIN) {
            if let Ok(value) = HeaderValue::from_str(&self.config.internal_origin()) {
                headers.insert(ORIGIN, value);
            }
        }
        if self.config.compat
            && !upgrade
            && (parts.method == Method::GET || parts.method == Method::HEAD)
        {
            headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        }

        if let Ok(value) = HeaderValue::from_str(external_host) {
            headers.insert("x-forwarded-host", value);
        }
        let proto = parts
            .headers
            .get("x-forwarded-proto")
            .and_then(|value| value.to_str().ok())
            .filter(|value| *value == "http" || *value == "https")
            .unwrap_or("http");
        if let Ok(value) = HeaderValue::from_str(proto) {
            headers.insert("x-forwarded-proto", value);
        }
        let forwarded_for = match parts
            .headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
        {
            Some(existing) if !existing.is_empty() => format!("{existing}, {}", peer.ip()),
            _ => peer.ip().to_string(),
        };
        if let Ok(value) = HeaderValue::from_str(&forwarded_for) {
            headers.insert("x-forwarded-for", value);
        }

        if upgrade {
            headers.insert(CONNECTION, HeaderValue::from_static("upgrade"));
            if let Some(value) = parts.headers.get(UPGRADE) {
                headers.insert(UPGRADE, value.clone());
            }
        }

        headers
    }

    fn unauthorized(&self) -> Response<ProxyBody> {
        let mut headers = HeaderMap::new();
        let challenge = format!(
            "Basic realm=\"{}\", charset=\"UTF-8\"",
            self.config.realm.replace('"', "")
        );
        if let Ok(value) = HeaderValue::from_str(&challenge) {
            headers.insert(http::header::WWW_AUTHENTICATE, value);
        }
        text_response(StatusCode::UNAUTHORIZED, "unauthorized\n", headers)
    }

    fn forbidden(&self, reject: trust::Reject) -> Response<ProxyBody> {
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(reject.reason()) {
            headers.insert("x-dsh-proxy-reject", value);
        }
        text_response(StatusCode::FORBIDDEN, "forbidden\n", headers)
    }
}

fn basic_credentials(headers: &HeaderMap) -> Option<(String, Vec<u8>)> {
    let value = trust::single_header(headers, "authorization")?;
    let encoded = value
        .strip_prefix("Basic ")
        .or_else(|| value.strip_prefix("basic "))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    if decoded.len() > 4096 {
        return None;
    }
    let colon = decoded.iter().position(|byte| *byte == b':')?;
    let username = String::from_utf8(decoded[..colon].to_vec()).ok()?;
    Some((username, decoded[colon + 1..].to_vec()))
}

fn is_upgrade_request(headers: &HeaderMap) -> bool {
    if !headers.contains_key(UPGRADE) {
        return false;
    }
    connection_tokens(headers)
        .iter()
        .any(|token| token.eq_ignore_ascii_case("upgrade"))
}

fn connection_tokens(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
        .collect()
}

fn is_hop_by_hop(name: &str) -> bool {
    HOP_BY_HOP
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let tokens = connection_tokens(headers);
    for name in HOP_BY_HOP {
        headers.remove(name);
    }
    for token in tokens {
        if let Ok(name) = HeaderName::from_bytes(token.as_bytes()) {
            headers.remove(name);
        }
    }
}

fn html_candidate(headers: &HeaderMap) -> bool {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !html::is_html_content_type(content_type)
        || !html::is_utf8_or_unspecified_charset(content_type)
    {
        return false;
    }
    match headers
        .get(CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
    {
        None => true,
        Some(value) => value.trim().eq_ignore_ascii_case("identity"),
    }
}

fn full_body(bytes: impl Into<Bytes>) -> ProxyBody {
    Full::new(bytes.into())
        .map_err(|error: Infallible| -> BoxError { match error {} })
        .boxed_unsync()
}

fn with_headers(status: StatusCode, headers: HeaderMap, body: Bytes) -> Response<ProxyBody> {
    let mut response = Response::new(full_body(body));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

fn text_response(
    status: StatusCode,
    body: &'static str,
    mut headers: HeaderMap,
) -> Response<ProxyBody> {
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(
        http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    headers.insert(
        http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    with_headers(status, headers, Bytes::from_static(body.as_bytes()))
}

/// Replace any client `dsh-auth-*` cookie with the one captured by the gateway.
/// Old cookies become invalid when DSH restarts with a new token; ignoring them
/// also prevents a redirect loop after a restart.
fn apply_dsh_cookie(headers: &mut HeaderMap, stored: Option<&str>) {
    let mut others: Vec<String> = Vec::new();
    for value in headers.get_all(COOKIE).iter() {
        let Ok(cookie) = value.to_str() else {
            continue;
        };
        for part in cookie.split(';') {
            let part = part.trim();
            if !part.is_empty() && !part.starts_with("dsh-auth-") {
                others.push(part.to_owned());
            }
        }
    }
    if let Some(cookie) = stored {
        others.push(cookie.to_owned());
    }
    headers.remove(COOKIE);
    if !others.is_empty() {
        if let Ok(value) = HeaderValue::from_str(&others.join("; ")) {
            headers.insert(COOKIE, value);
        }
    }
}

fn dsh_cookie_from_headers(headers: &HeaderMap) -> Option<String> {
    for value in headers.get_all(SET_COOKIE).iter() {
        let Ok(cookie) = value.to_str() else {
            continue;
        };
        for part in cookie.split(';') {
            let part = part.trim();
            if part.starts_with("dsh-auth-") && part.contains('=') {
                return Some(part.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_credentials() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Basic YWxpY2U6c2VjcmV0"),
        );
        let (user, password) = basic_credentials(&headers).unwrap();
        assert_eq!(user, "alice");
        assert_eq!(password, b"secret");
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Basic !!!"));
        assert!(basic_credentials(&headers).is_none());
    }

    #[test]
    fn detects_upgrade() {
        let mut headers = HeaderMap::new();
        headers.insert(CONNECTION, HeaderValue::from_static("keep-alive, Upgrade"));
        headers.insert(UPGRADE, HeaderValue::from_static("websocket"));
        assert!(is_upgrade_request(&headers));
        headers.remove(UPGRADE);
        assert!(!is_upgrade_request(&headers));
    }

    #[test]
    fn injects_the_dsh_web_token_once() {
        let mut config = GatewayConfig::for_test(
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1:1".parse().unwrap(),
            "missing-auth-file",
        );
        config.web_token = Some("token_123".to_owned());
        let gateway = Gateway::new(config);
        assert_eq!(
            gateway.with_web_token("/", true).as_ref(),
            "/?token=token_123"
        );
        assert_eq!(
            gateway.with_web_token("/?x=1", true).as_ref(),
            "/?x=1&token=token_123"
        );
        assert_eq!(
            gateway.with_web_token("/?token=user", true).as_ref(),
            "/?token=user"
        );
        assert_eq!(gateway.with_web_token("/", false).as_ref(), "/");

        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("a=b; dsh-auth-old=v1.stale"),
        );
        apply_dsh_cookie(&mut headers, None);
        assert_eq!(headers.get(COOKIE).unwrap(), "a=b");
        apply_dsh_cookie(&mut headers, Some("dsh-auth-new=v1.fresh"));
        assert_eq!(headers.get(COOKIE).unwrap(), "a=b; dsh-auth-new=v1.fresh");

        let mut response = HeaderMap::new();
        response.insert(
            SET_COOKIE,
            HeaderValue::from_static("dsh-auth-new=v1.fresh; Path=/; HttpOnly"),
        );
        assert_eq!(
            dsh_cookie_from_headers(&response).as_deref(),
            Some("dsh-auth-new=v1.fresh")
        );
    }

    #[test]
    fn strips_hop_by_hop_and_connection_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(CONNECTION, HeaderValue::from_static("x-secret, close"));
        headers.insert("x-secret", HeaderValue::from_static("1"));
        headers.insert("x-keep", HeaderValue::from_static("1"));
        strip_hop_by_hop(&mut headers);
        assert!(!headers.contains_key("connection"));
        assert!(!headers.contains_key("x-secret"));
        assert!(headers.contains_key("x-keep"));
    }
}
