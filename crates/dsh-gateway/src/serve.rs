// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{convert::Infallible, future::Future, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

use crate::{config::GatewayConfig, proxy::Gateway};

/// Bind and serve the gateway until `shutdown` completes.
pub async fn serve<F>(config: GatewayConfig, shutdown: F) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let addr = config.listen_addr()?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind gateway on {addr}"))?;
    info!(%addr, "gateway listening");

    let gateway = Arc::new(Gateway::new(config));
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                info!("gateway shutting down");
                return Ok(());
            }
            accepted = listener.accept() => {
                let (stream, peer) = match accepted {
                    Ok(value) => value,
                    Err(error) => {
                        warn!(error = %error, "gateway accept failed");
                        continue;
                    }
                };
                let gateway = gateway.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request| {
                        let gateway = gateway.clone();
                        async move {
                            Ok::<_, Infallible>(gateway.handle(request, peer).await)
                        }
                    });
                    let connection = http1::Builder::new()
                        .timer(TokioTimer::new())
                        .header_read_timeout(Duration::from_secs(30))
                        .keep_alive(true)
                        .serve_connection(TokioIo::new(stream), service)
                        .with_upgrades();
                    if let Err(error) = connection.await {
                        debug!(error = %error, peer = %peer, "connection closed with an error");
                    }
                });
            }
        }
    }
}
