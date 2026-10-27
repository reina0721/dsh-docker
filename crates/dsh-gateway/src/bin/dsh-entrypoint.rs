// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    env,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use dsh_gateway::{
    config::GatewayConfig,
    home::{self, HomeLayout},
    serve,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
    signal,
    sync::{mpsc, watch},
    time::{sleep, timeout},
};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("dsh-entrypoint: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let mut args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("healthcheck") => return healthcheck(),
        Some("prepare") => {
            home::prepare(&HomeLayout::from_env())?;
            return Ok(());
        }
        Some("version") => {
            println!("dsh-entrypoint {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("serve") => {
            args.remove(0);
        }
        _ => {}
    }

    let config = GatewayConfig::from_env().context("invalid gateway configuration")?;
    init_tracing(&config);
    home::prepare(&HomeLayout::from_env()).context("failed to prepare persistent home")?;

    if env::var("DSH_STARTUP_PREPARE_ONLY")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        info!("persistent home prepared; exiting because DSH_STARTUP_PREPARE_ONLY=true");
        return Ok(());
    }

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let mut command = Command::new("dsh");
    command
        .arg("web")
        .arg("--no-open")
        .arg("--host")
        .arg(&config.upstream_host)
        .arg("--port")
        .arg(config.upstream_port.to_string())
        .arg("--trusted-host")
        .arg("localhost")
        .arg("--trusted-host")
        .arg("127.0.0.1")
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = command.spawn().context("failed to start `dsh web`")?;
    let child_pid = child.id();
    info!(pid = ?child_pid, "started dsh web");

    let stdout = child
        .stdout
        .take()
        .context("dsh web stdout was not captured")?;
    let stderr = child
        .stderr
        .take()
        .context("dsh web stderr was not captured")?;
    let (token_tx, mut token_rx) = mpsc::channel(1);
    tokio::spawn(log_lines(stdout, Some(token_tx.clone())));
    tokio::spawn(log_lines(stderr, Some(token_tx)));

    if let Err(error) = wait_until_ready(&config, &mut child).await {
        let _ = shutdown_tx.send(true);
        let _ = terminate(&mut child).await;
        return Err(error);
    }
    let token = match timeout(Duration::from_secs(20), token_rx.recv()).await {
        Ok(Some(token)) => Some(token),
        _ => None,
    };
    if token.is_none() {
        warn!("could not capture the dsh web token; the UI may require its tokenized URL");
    }

    let mut gateway_config = config.clone();
    gateway_config.web_token = token;
    let mut gateway = tokio::spawn(async move {
        let mut shutdown_rx = shutdown_rx;
        let result = serve::serve(gateway_config, async move {
            while !*shutdown_rx.borrow() {
                if shutdown_rx.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;
        if let Err(error) = &result {
            error!(error = %error, "gateway stopped with an error");
        }
        result
    });
    info!("dsh web is ready");

    let mut shutdown_signal = Box::pin(shutdown_signal());
    loop {
        if let Some(status) = child.try_wait().context("failed to poll dsh web")? {
            warn!(?status, "dsh web exited; stopping gateway");
            let _ = shutdown_tx.send(true);
            let _ = timeout(Duration::from_secs(10), &mut gateway).await;
            if !status.success() {
                bail!("dsh web exited with {status}");
            }
            return Ok(());
        }

        tokio::select! {
            _ = &mut shutdown_signal => {
                info!("received shutdown signal");
                let _ = shutdown_tx.send(true);
                let _ = terminate(&mut child).await;
                let _ = timeout(Duration::from_secs(10), &mut gateway).await;
                return Ok(());
            }
            result = &mut gateway => {
                match result {
                    Ok(Ok(())) => warn!("gateway exited unexpectedly"),
                    Ok(Err(error)) => error!(error = %error, "gateway failed"),
                    Err(error) => error!(error = %error, "gateway task failed"),
                }
                let _ = terminate(&mut child).await;
                bail!("gateway exited; shutting down dsh web");
            }
            _ = sleep(Duration::from_millis(500)) => {}
        }
    }
}

async fn log_lines<R>(reader: R, mut token_tx: Option<mpsc::Sender<String>>)
where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let token = parse_web_token(&line);
        let redacted = match &token {
            Some(token) => line.replace(token, "<redacted>"),
            None => line,
        };
        info!(target: "dsh", "{redacted}");
        if let Some(token) = token {
            if let Some(sender) = token_tx.take() {
                let _ = sender.send(token).await;
            }
        }
    }
}

fn parse_web_token(line: &str) -> Option<String> {
    const MARKER: &str = "?token=";
    let start = line.find(MARKER)? + MARKER.len();
    let token: String = line[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~'))
        .collect();
    (!token.is_empty() && token.len() <= 512).then_some(token)
}

fn init_tracing(config: &GatewayConfig) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.log_level.clone()));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false);
    if config.log_format == "json" {
        let _ = builder.json().try_init();
    } else {
        let _ = builder.try_init();
    }
}

async fn wait_until_ready(config: &GatewayConfig, child: &mut tokio::process::Child) -> Result<()> {
    let addr = config.upstream_addr()?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        if let Some(status) = child.try_wait().context("failed to poll dsh web")? {
            bail!("dsh web exited before becoming ready: {status}");
        }
        if timeout(
            Duration::from_millis(500),
            tokio::net::TcpStream::connect(addr),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .is_some()
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("dsh web did not become ready within 90 seconds");
        }
        sleep(Duration::from_millis(500)).await;
    }
}

async fn terminate(child: &mut tokio::process::Child) -> Result<()> {
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            let pid = nix::unistd::Pid::from_raw(pid as i32);
            let _ = nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGTERM);
        }
    }
    if timeout(Duration::from_secs(15), child.wait())
        .await
        .is_err()
    {
        warn!("dsh web did not stop within 15 seconds; sending SIGKILL");
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal as unix_signal};
        let mut terminate = match unix_signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                warn!(error = %error, "failed to install SIGTERM handler");
                let _ = signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = signal::ctrl_c().await;
    }
}

fn healthcheck() -> Result<()> {
    let config = GatewayConfig::from_env().context("invalid gateway configuration")?;
    let upstream = config.upstream_addr()?;
    let gateway_ip = match config.listen_addr()?.ip() {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        }
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        }
        ip => ip,
    };
    let gateway = SocketAddr::new(gateway_ip, config.listen_port);
    check_tcp(upstream, "dsh web")?;
    check_gateway(gateway)?;
    Ok(())
}

fn check_tcp(addr: SocketAddr, label: &str) -> Result<()> {
    TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .with_context(|| format!("{label} is not reachable at {addr}"))?;
    Ok(())
}

fn check_gateway(addr: SocketAddr) -> Result<()> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .with_context(|| format!("gateway is not reachable at {addr}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .context("failed to write healthcheck request")?;
    let mut buffer = [0u8; 1024];
    let read = stream
        .read(&mut buffer)
        .context("failed to read healthcheck response")?;
    let response = String::from_utf8_lossy(&buffer[..read]);
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .context("gateway returned a malformed HTTP response")?;
    match status {
        200 | 401 => Ok(()),
        other => bail!("gateway healthcheck returned HTTP {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_web_token;

    #[test]
    fn parses_dsh_web_token() {
        assert_eq!(
            parse_web_token("dsh web: http://127.0.0.1:3079/?token=abc_DEF-123"),
            Some("abc_DEF-123".to_owned())
        );
        assert_eq!(
            parse_web_token("dsh web: http://127.0.0.1:3079/?token=abc&next=1"),
            Some("abc".to_owned())
        );
        assert_eq!(parse_web_token("dsh web is ready"), None);
    }
}
