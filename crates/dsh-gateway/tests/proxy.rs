// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{convert::Infallible, net::SocketAddr, time::Duration};

use base64::Engine;
use bytes::Bytes;
use dsh_auth::UserStore;
use dsh_gateway::{config::GatewayConfig, serve::serve};
use http::{Request, Response, header};
use http_body_util::Full;
use hyper::{body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::{sleep, timeout},
};

async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

async fn wait_ready(addr: SocketAddr) {
    for _ in 0..100 {
        if TcpStream::connect(addr).await.is_ok() {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("gateway did not become ready");
}

async fn request(addr: SocketAddr, raw: String) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(raw.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        match timeout(Duration::from_secs(2), stream.read(&mut buffer)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(read)) => response.extend_from_slice(&buffer[..read]),
            Ok(Err(_)) => break,
        }
    }
    String::from_utf8_lossy(&response).to_string()
}

fn basic() -> String {
    base64::engine::general_purpose::STANDARD.encode("alice:secret")
}

#[tokio::test]
async fn auth_trust_and_html_injection() {
    let auth_dir = tempfile::tempdir().unwrap();
    let auth_path = auth_dir.path().join("users.htpasswd");
    let mut users = UserStore::new();
    users.upsert("alice", dsh_auth::hash_password(b"secret", 4).unwrap());
    users.write_atomic(&auth_path, 0o600).unwrap();

    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();
    let upstream_addr = format!("127.0.0.1:{upstream_port}");
    let upstream = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = upstream_listener.accept().await else {
                break;
            };
            let expected_host = format!("localhost:{upstream_port}");
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| {
                    let expected_host = expected_host.clone();
                    async move {
                        assert_eq!(
                            request
                                .headers()
                                .get(header::HOST)
                                .and_then(|value| value.to_str().ok()),
                            Some(expected_host.as_str())
                        );
                        let body = if request.uri().path() == "/" {
                            "<html><head><title>DSH</title></head><body>hello</body></html>"
                        } else {
                            "plain"
                        };
                        let content_type = if request.uri().path() == "/" {
                            "text/html; charset=utf-8"
                        } else {
                            "text/plain"
                        };
                        let mut response = Response::new(Full::new(Bytes::from(body)));
                        response
                            .headers_mut()
                            .insert(header::CONTENT_TYPE, content_type.parse().unwrap());
                        Ok::<_, Infallible>(response)
                    }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });

    let port = free_port().await;
    let listen = format!("127.0.0.1:{port}").parse().unwrap();
    let upstream_socket = upstream_addr.parse().unwrap();
    let config = GatewayConfig::for_test(listen, upstream_socket, &auth_path);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let gateway = tokio::spawn(serve(config, async move {
        let _ = shutdown_rx.await;
    }));
    let addr: SocketAddr = listen;
    wait_ready(addr).await;

    let unauthorized = request(
        addr,
        "GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n".to_owned(),
    )
    .await;
    assert!(unauthorized.starts_with("HTTP/1.1 401"), "{unauthorized}");
    assert!(
        unauthorized
            .to_ascii_lowercase()
            .contains("www-authenticate")
    );

    let authorized = request(
        addr,
        format!(
            "GET / HTTP/1.1\r\nHost: example.com\r\nAuthorization: Basic {}\r\nConnection: close\r\n\r\n",
            basic()
        ),
    )
    .await;
    assert!(authorized.starts_with("HTTP/1.1 200"), "{authorized}");
    assert!(authorized.contains("/__dsh_http_compat.js"), "{authorized}");
    assert!(
        authorized
            .to_ascii_lowercase()
            .contains("x-dsh-http-compat: active"),
        "{authorized}"
    );

    let bad_origin = request(
        addr,
        format!(
            "GET / HTTP/1.1\r\nHost: example.com\r\nOrigin: https://evil.example\r\nAuthorization: Basic {}\r\nConnection: close\r\n\r\n",
            basic()
        ),
    )
    .await;
    assert!(bad_origin.starts_with("HTTP/1.1 403"), "{bad_origin}");
    assert!(
        bad_origin
            .to_ascii_lowercase()
            .contains("x-dsh-proxy-reject: origin-host-mismatch"),
        "{bad_origin}"
    );

    let asset = request(
        addr,
        format!(
            "GET /__dsh_http_compat.js HTTP/1.1\r\nHost: example.com\r\nAuthorization: Basic {}\r\nConnection: close\r\n\r\n",
            basic()
        ),
    )
    .await;
    assert!(asset.starts_with("HTTP/1.1 200"), "{asset}");
    assert!(
        asset
            .to_ascii_lowercase()
            .contains("content-type: application/javascript"),
        "{asset}"
    );

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(2), gateway).await;
    upstream.abort();
}

#[tokio::test]
async fn fails_closed_without_users() {
    let auth_dir = tempfile::tempdir().unwrap();
    let auth_path = auth_dir.path().join("users.htpasswd");
    std::fs::write(&auth_path, b"").unwrap();

    let port = free_port().await;
    let listen: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let upstream: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let config = GatewayConfig::for_test(listen, upstream, &auth_path);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let gateway = tokio::spawn(serve(config, async move {
        let _ = shutdown_rx.await;
    }));
    wait_ready(listen).await;

    let response = request(
        listen,
        format!(
            "GET / HTTP/1.1\r\nHost: example.com\r\nAuthorization: Basic {}\r\nConnection: close\r\n\r\n",
            basic()
        ),
    )
    .await;
    assert!(
        response.starts_with("HTTP/1.1 503"),
        "expected fail-closed 503, got {response}"
    );

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(2), gateway).await;
}

#[tokio::test]
async fn websocket_upgrade_is_proxied() {
    let auth_dir = tempfile::tempdir().unwrap();
    let auth_path = auth_dir.path().join("users.htpasswd");
    let mut users = UserStore::new();
    users.upsert("alice", dsh_auth::hash_password(b"secret", 4).unwrap());
    users.write_atomic(&auth_path, 0o600).unwrap();

    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();
    let upstream = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = upstream_listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte).await {
                        Ok(0) | Err(_) => return,
                        Ok(_) => request.push(byte[0]),
                    }
                }
                let text = String::from_utf8_lossy(&request).to_ascii_lowercase();
                assert!(
                    text.contains(&format!("host: localhost:{upstream_port}")),
                    "{text}"
                );
                stream
                    .write_all(
                        b"HTTP/1.1 101 Switching Protocols\r\n\
                          Connection: Upgrade\r\n\
                          Upgrade: websocket\r\n\
                          Sec-WebSocket-Accept: test\r\n\r\n",
                    )
                    .await
                    .unwrap();
                let mut buffer = [0u8; 1024];
                loop {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            if stream.write_all(&buffer[..read]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    });

    let port = free_port().await;
    let listen: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let upstream_socket: SocketAddr = format!("127.0.0.1:{upstream_port}").parse().unwrap();
    let config = GatewayConfig::for_test(listen, upstream_socket, &auth_path);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let gateway = tokio::spawn(serve(config, async move {
        let _ = shutdown_rx.await;
    }));
    wait_ready(listen).await;

    let mut client = TcpStream::connect(listen).await.unwrap();
    let handshake = format!(
        "GET /ws HTTP/1.1\r\nHost: example.com\r\nAuthorization: Basic {}\r\n\
         Connection: Upgrade\r\nUpgrade: websocket\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n",
        basic()
    );
    client.write_all(handshake.as_bytes()).await.unwrap();

    let mut headers = Vec::new();
    let mut byte = [0u8; 1];
    while !headers.ends_with(b"\r\n\r\n") {
        let read = timeout(Duration::from_secs(2), client.read(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(read, 0);
        headers.push(byte[0]);
    }
    let headers = String::from_utf8_lossy(&headers);
    assert!(headers.starts_with("HTTP/1.1 101"), "{headers}");

    client.write_all(b"ping").await.unwrap();
    let mut echo = [0u8; 4];
    timeout(Duration::from_secs(2), client.read_exact(&mut echo))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&echo, b"ping");

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(2), gateway).await;
    upstream.abort();
}
