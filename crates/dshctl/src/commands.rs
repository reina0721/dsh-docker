// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    fs::{self, File},
    io::{self, IsTerminal, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, ToSocketAddrs},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use base64::Engine;
use clap::CommandFactory;
use dsh_auth::{hash_password, validate_username};
use dsh_config::{Config, ImageMode, ImageVariant};
use getrandom::getrandom;
use serde::Serialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{
    AccountArgs, AccountSubcommand, Cli, Commands, CompletionArgs, ConfigCommands, DeployArgs,
    DoctorArgs, GitCommands, ImageCommands, InitArgs, LogsArgs, MigrateArgs, RebuildArgs,
    RestoreArgs, SelfUpdateArgs, SshCommands, StatusArgs, UpdateArgs, UpgradeArgs, UserCommands,
    backup,
    docker::{Docker, RUNTIME_LABEL, RUNTIME_LABEL_VALUE},
    git::GitConfig,
    runtime,
    ssh::SshManager,
};

const RELEASE_API: &str = "https://api.github.com/repos/reina0721/dsh-docker/releases/latest";
const RELEASE_DOWNLOAD: &str = "https://github.com/reina0721/dsh-docker/releases/download";

pub fn run(cli: Cli) -> Result<()> {
    let dir = resolve_dir(&cli.dir)?;
    let mut config = Config::load(&dir)?;
    apply_overrides(&mut config, &cli.command)?;

    match cli.command {
        Commands::Init(args) => cmd_init(&config, args),
        Commands::Deploy(args) => cmd_deploy(&config, cli.verbose, args),
        Commands::Up => cmd_up(&config, cli.verbose),
        Commands::Down => {
            let docker = docker(&config, cli.verbose);
            docker.require()?;
            docker.down()
        }
        Commands::Stop => {
            let docker = docker(&config, cli.verbose);
            docker.require()?;
            docker.stop()
        }
        Commands::Restart => {
            let docker = docker(&config, cli.verbose);
            docker.require()?;
            docker.restart()?;
            wait_health(&config)
        }
        Commands::Status(args) => cmd_status(&config, cli.verbose, args),
        Commands::Doctor(args) => cmd_doctor(&config, cli.verbose, args),
        Commands::Logs(args) => cmd_logs(&config, cli.verbose, args),
        Commands::Shell => {
            let docker = docker(&config, cli.verbose);
            docker.require()?;
            docker.shell()
        }
        Commands::Exec(args) => {
            let docker = docker(&config, cli.verbose);
            docker.require()?;
            docker.exec(&args.command)
        }
        Commands::Update(args) => cmd_update(&config, cli.verbose, args),
        Commands::Upgrade(args) => cmd_upgrade(&config, cli.verbose, args),
        Commands::SelfUpdate(args) => cmd_self_update(args),
        Commands::Rebuild(args) => cmd_rebuild(&config, cli.verbose, args),
        Commands::User(args) => match args.command {
            UserCommands::Add(args) => cmd_user_add(&config, args),
            UserCommands::List(args) => cmd_user_list(&config, args.json),
            UserCommands::Remove(args) => cmd_user_remove(&config, args),
        },
        Commands::Ssh(args) => cmd_ssh(&config, args.command),
        Commands::Git(args) => cmd_git(&config, args.command),
        Commands::Backup(args) => {
            let docker = docker(&config, cli.verbose);
            let path = backup::create(&config, args.output, args.quiesce, Some(&docker))?;
            println!("Backup: {}", path.display());
            print_archive_info(&path)?;
            println!(
                "WARNING: backups contain SSH keys, Git/CLI credentials, and DSH credentials."
            );
            Ok(())
        }
        Commands::Restore(args) => cmd_restore(&config, cli.verbose, args),
        Commands::Image(args) => match args.command {
            ImageCommands::Check(args) => cmd_image_check(&config, cli.verbose, args.network),
            ImageCommands::Pull(_) => {
                let docker = docker(&config, cli.verbose);
                docker.require()?;
                docker.pull_with_mirrors()
            }
        },
        Commands::Config(args) => match args.command {
            ConfigCommands::Validate => {
                config.validate()?;
                println!("configuration is valid");
                Ok(())
            }
            ConfigCommands::Show(_) => {
                println!("{}", serde_json::to_string_pretty(&config.redacted())?);
                Ok(())
            }
        },
        Commands::Migrate(args) => cmd_migrate(&config, cli.verbose, args),
        Commands::Account(args) => cmd_account(&config, cli.verbose, args),
        Commands::Completion(args) => cmd_completion(args),
        Commands::Version => {
            println!(
                "dshctl {} ({}-{})",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            Ok(())
        }
    }
}

fn apply_overrides(config: &mut Config, command: &Commands) -> Result<()> {
    match command {
        Commands::Deploy(DeployArgs {
            mode, mirror, slim, ..
        }) => {
            if let Some(mode) = mode {
                config.image_mode = mode.parse()?;
            }
            if let Some(mirror) = mirror {
                config.image_mirror = mirror.parse()?;
            }
            if *slim {
                config.image_variant = ImageVariant::Slim;
            }
        }
        Commands::Rebuild(RebuildArgs { slim, .. }) => {
            if *slim {
                config.image_variant = ImageVariant::Slim;
            }
        }
        Commands::Image(args) => {
            if let ImageCommands::Pull(pull) = &args.command {
                if let Some(mirror) = &pull.mirror {
                    config.image_mirror = mirror.parse()?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn resolve_dir(input: &Path) -> Result<PathBuf> {
    let input = if input.is_absolute() {
        input.to_path_buf()
    } else {
        std::env::current_dir()?.join(input)
    };
    let input = if input.exists() {
        fs::canonicalize(&input).unwrap_or(input)
    } else {
        input
    };
    let input = strip_verbatim(input);
    if input.join("compose.yaml").exists() || input.join("Cargo.toml").exists() {
        return Ok(input);
    }
    let mut cursor = input.as_path();
    while let Some(parent) = cursor.parent() {
        if parent.join("compose.yaml").exists() {
            return Ok(strip_verbatim(parent.to_path_buf()));
        }
        cursor = parent;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(exe_canon) = fs::canonicalize(exe) {
            let exe_canon = strip_verbatim(exe_canon);
            if let Some(parent) = exe_canon.parent() {
                if parent.join("compose.yaml").exists() {
                    return Ok(parent.to_path_buf());
                }
                if let Some(grandparent) = parent.parent() {
                    if grandparent.join("compose.yaml").exists() {
                        return Ok(grandparent.to_path_buf());
                    }
                }
            }
        }
    }
    Ok(input)
}

fn strip_verbatim(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path
}

fn docker(config: &Config, verbose: bool) -> Docker {
    Docker::new(config.clone(), verbose)
}

fn cmd_init(config: &Config, args: InitArgs) -> Result<()> {
    runtime::init(
        config,
        runtime::InitOptions {
            force: args.force,
            refresh: args.refresh,
            quiet: false,
        },
    )
}

fn cmd_deploy(config: &Config, verbose: bool, args: DeployArgs) -> Result<()> {
    let docker = docker(config, verbose);
    docker.require()?;
    docker.cleanup_legacy()?;
    runtime::init(config, runtime::InitOptions::default())?;
    ensure_auth(config, args.yes)?;
    if let Ok(ip) = config.bind_ip() {
        if !ip.is_loopback() {
            eprintln!(
                "warning: BIND_ADDRESS={} exposes the gateway beyond loopback; terminate TLS in a front proxy",
                config.bind_address
            );
        }
    }
    docker.compose_config_check()?;
    preflight_port(config, &docker)?;
    let delivery = docker.prepare_image()?;
    println!("Image delivery: {}", delivery.as_str());
    docker.up()?;
    wait_health(config)?;
    status_text(config, &docker, true)?;
    Ok(())
}

fn cmd_up(config: &Config, verbose: bool) -> Result<()> {
    let docker = docker(config, verbose);
    docker.require()?;
    docker.compose_config_check()?;
    docker.up()?;
    wait_health(config)?;
    status_text(config, &docker, true)
}

fn cmd_status(config: &Config, verbose: bool, args: StatusArgs) -> Result<()> {
    let docker = docker(config, verbose);
    if args.json {
        let http = probe_http(config);
        let users = runtime::load_users(config)
            .map(|store| store.len())
            .unwrap_or(0);
        let services = docker.ps().unwrap_or_default();
        let image = config.effective_image();
        let dsh_version = detect_current_dsh_version(&docker, config);
        let payload = serde_json::json!({
            "cli_version": env!("CARGO_PKG_VERSION"),
            "project_dir": config.project_dir,
            "runtime_dir": config.runtime_dir,
            "image": image,
            "variant": config.image_variant.to_string(),
            "dsh_version": dsh_version.as_deref().unwrap_or(&config.dsh_version),
            "image_label": docker.image_label(&image, RUNTIME_LABEL).ok().flatten(),
            "services": services,
            "http": {
                "url": config_url(config),
                "status": http.as_ref().ok(),
                "error": http.as_ref().err().map(|error| format!("{error:#}")),
            },
            "auth_users": users,
            "compat": config.compat,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        status_text(config, &docker, false)?;
    }
    Ok(())
}

fn status_text(config: &Config, docker: &Docker, verbose: bool) -> Result<()> {
    let services = docker.ps().unwrap_or_default();
    println!("\nServices:");
    if services.is_empty() {
        println!("  (none)");
    } else {
        for service in &services {
            let name = service
                .get("Service")
                .or_else(|| service.get("Name"))
                .and_then(|value| value.as_str())
                .unwrap_or("?");
            let state = service
                .get("State")
                .and_then(|value| value.as_str())
                .unwrap_or("?");
            let health = service
                .get("Health")
                .and_then(|value| value.as_str())
                .unwrap_or("-");
            println!("  {name}: {state} (health: {health})");
        }
    }

    let users = runtime::load_users(config)
        .map(|store| store.len())
        .unwrap_or(0);
    let image = config.effective_image();
    println!("Core listen  : {}:{}", config.bind_address, config.dsh_port);
    println!("Proxy URL    : {}", config_url(config));
    println!("Runtime      : {}", config.runtime_dir.display());
    println!(
        "Workspace    : {} <-> /workspace",
        config.workspace_dir().display()
    );
    println!(
        "Developer home: {} <-> /home/node/.persist",
        config.home_dir().display()
    );
    println!("Auth users   : {users}");
    println!("Image        : {image}");
    println!("Variant      : {}", config.image_variant);
    let dsh_ver =
        detect_current_dsh_version(docker, config).unwrap_or_else(|| config.dsh_version.clone());
    println!("DSH version  : {dsh_ver}");
    println!("Account      : {}", detect_account_status(config));
    println!("HTTP compat  : {}", config.compat);
    if verbose {
        println!("Project      : {}", config.project_dir.display());
    }

    match probe_http(config) {
        Ok(401) => println!("HTTP check   : 401 (normal: Basic Auth is active)"),
        Ok(code) => println!("HTTP check   : {code}"),
        Err(error) => println!("HTTP check   : unavailable ({error:#})"),
    }
    Ok(())
}

pub fn detect_account_status(config: &Config) -> String {
    let primary = config.dsh_home().join(".credentials.yaml");
    let fallback = config.home_dir().join("dsh").join(".credentials.yaml");
    for path in [&primary, &fallback] {
        if let Ok(content) = fs::read_to_string(path) {
            if content.contains("account:platform") {
                return "logged in (DeepSeek official)".to_string();
            }
        }
    }
    format!(
        "not logged in (use SSH tunnel to sign in: ssh -N -L {port}:127.0.0.1:{port} user@host)",
        port = config.dsh_port
    )
}

fn cmd_account(config: &Config, _verbose: bool, args: AccountArgs) -> Result<()> {
    let creds_file = if config.dsh_home().join(".credentials.yaml").exists() {
        config.dsh_home().join(".credentials.yaml")
    } else {
        config.home_dir().join("dsh").join(".credentials.yaml")
    };
    let is_logged_in = fs::read_to_string(&creds_file)
        .map(|content| content.contains("account:platform"))
        .unwrap_or(false)
        || fs::read_to_string(config.home_dir().join("dsh").join(".credentials.yaml"))
            .map(|content| content.contains("account:platform"))
            .unwrap_or(false);

    let subcmd = args.command.unwrap_or(AccountSubcommand::Status);
    match subcmd {
        AccountSubcommand::Status => {
            println!("\nDeepSeek Official Account Status:");
            if is_logged_in {
                println!("  Status     : Logged in (official account token active)");
                println!("  Platform   : https://platform.deepseek.com");
                println!("  Credentials: {}", creds_file.display());
            } else {
                println!("  Status     : Not logged in");
                println!(
                    "  Note       : Run 'dshctl account login' for instructions on signing in."
                );
            }
            println!();
            Ok(())
        }
        AccountSubcommand::Where => {
            println!("\nCredentials location:");
            println!("  Host path      : {}", creds_file.display());
            println!("  Container path : /home/node/.dsh/.credentials.yaml");
            println!("  DSH home dir   : {}", config.dsh_home().display());
            println!();
            Ok(())
        }
        AccountSubcommand::Login => {
            println!("\n=== DeepSeek 官网账户登录指南 ===");
            println!();
            println!("DeepSeek 官方平台（platform.deepseek.com）OAuth 登录规范：");
            println!(
                "官方认证服务器严格限定 OAuth 回调仅信任本地回环地址（http://localhost:<端口>/oauth/callback）。"
            );
            println!();
            println!(
                "如果您在远程 Linux 服务器上部署了 dsh-docker，请通过以下步骤进行官网账户登录："
            );
            println!();
            println!(
                "1. 在您的本地电脑终端（Windows PowerShell / macOS Terminal / Linux Shell）建立 SSH 端口转发隧道："
            );
            println!(
                "   ssh -N -L {port}:127.0.0.1:{port} <SSH用户名>@<服务器地址> -p <SSH端口>",
                port = config.dsh_port
            );
            println!();
            println!("2. 在您的本地浏览器中访问：");
            println!("   http://localhost:{}/", config.dsh_port);
            println!();
            println!("3. 使用 Basic Auth 管理员账号密码登录进入 DSH Web UI。");
            println!(
                "   点击右上角设置图标（Settings）-> 账号（Account）-> 点击「登录 (Sign in)」按钮。"
            );
            println!();
            println!("4. 浏览器将弹出 DeepSeek 官方登录页面，完成登录授权。");
            println!(
                "   平台会自动回调至 http://localhost:{}/oauth/callback 完成凭据存盘。",
                config.dsh_port
            );
            println!();
            println!("5. 凭据将持久化保存在 host 目录：");
            println!("   {}", creds_file.display());
            println!("   后续即使容器重启或升级，登录状态均持久保留。");
            println!();
            Ok(())
        }
    }
}

fn cmd_doctor(config: &Config, verbose: bool, args: DoctorArgs) -> Result<()> {
    let mut findings: Vec<Finding> = Vec::new();
    macro_rules! add {
        ($level:expr, $code:expr, $message:expr, $hint:expr $(,)?) => {
            findings.push(Finding {
                level: $level,
                code: $code.to_owned(),
                message: $message.to_string(),
                hint: $hint.map(|value: &str| value.to_owned()),
            })
        };
    }

    if args.fix {
        match runtime::init(config, runtime::InitOptions::default()) {
            Ok(()) => add!(
                Level::Ok,
                "runtime.fixed",
                "runtime directories and permissions refreshed",
                None
            ),
            Err(error) => add!(
                Level::Fail,
                "runtime.fix",
                format!("failed to repair runtime: {error:#}"),
                None
            ),
        }
    }

    let docker = docker(config, verbose);
    match docker.require() {
        Ok(()) => add!(
            Level::Ok,
            "docker",
            "docker, Compose v2, and daemon are available",
            None
        ),
        Err(error) => add!(
            Level::Fail,
            "docker",
            format!("{error:#}"),
            Some("install Docker Engine and Compose v2")
        ),
    }

    if config.project_dir.join(".env").is_file() {
        add!(Level::Ok, "env", ".env exists", None);
    } else {
        add!(
            Level::Warn,
            "env",
            ".env is missing",
            Some("run `dshctl init` or `dshctl deploy`")
        );
    }
    if config.env.contains_key("GATEWAY_CADDY_VERSION") {
        add!(
            Level::Warn,
            "env.legacy",
            "GATEWAY_CADDY_VERSION is obsolete in v2",
            Some("remove it; the Rust gateway is built into the image")
        );
    }
    if config.image.ends_with(":latest") {
        add!(
            Level::Warn,
            "env.image",
            "DSH_IMAGE uses the v1-style :latest tag",
            Some("run `dshctl migrate` or set DSH_IMAGE=ghcr.io/reina0721/dsh-docker:2")
        );
    }

    match config.validate() {
        Ok(()) => add!(Level::Ok, "config", "typed configuration is valid", None),
        Err(error) => add!(
            Level::Fail,
            "config",
            format!("{error:#}"),
            Some("run `dshctl config show`")
        ),
    }

    match docker.compose_config_check() {
        Ok(()) => add!(Level::Ok, "compose", "compose configuration is valid", None),
        Err(error) => add!(Level::Fail, "compose", format!("{error:#}"), None),
    }

    match runtime::load_users(config) {
        Ok(store) if !store.is_empty() => add!(
            Level::Ok,
            "auth",
            format!("{} Basic Auth user(s) configured", store.len()),
            None,
        ),
        Ok(_) => add!(
            Level::Warn,
            "auth",
            "no Basic Auth users configured",
            Some("run `dshctl user add <name>`")
        ),
        Err(error) => add!(Level::Fail, "auth", format!("{error:#}"), None),
    }

    match port_available(config) {
        Ok(()) => add!(
            Level::Ok,
            "port",
            format!("port {} is available", config.dsh_port),
            None
        ),
        Err(_error) if docker.port_owned(config.dsh_port) => add!(
            Level::Ok,
            "port",
            format!("port {} is owned by this project", config.dsh_port),
            None,
        ),
        Err(error) => add!(
            Level::Fail,
            "port",
            format!("{error:#}"),
            Some("change DSH_PORT or stop the other process")
        ),
    }

    if config.runtime_dir.is_dir() {
        add!(Level::Ok, "runtime", "runtime directory exists", None);
    } else {
        add!(
            Level::Fail,
            "runtime",
            "runtime directory is missing",
            Some("run `dshctl init`")
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(config.home_dir().join("ssh")) {
            let mode = metadata.permissions().mode() & 0o777;
            if mode == 0o700 {
                add!(Level::Ok, "permissions", "SSH directory mode is 0700", None);
            } else {
                add!(
                    Level::Fail,
                    "permissions",
                    format!("SSH directory mode is {mode:o}, expected 700"),
                    Some("run `dshctl doctor --fix`")
                );
            }
        }
    }

    if docker.require().is_ok() {
        match docker.image_label(&config.effective_image(), RUNTIME_LABEL) {
            Ok(Some(value)) if value == RUNTIME_LABEL_VALUE => {
                add!(
                    Level::Ok,
                    "image",
                    "runtime image carries the v2 label",
                    None
                )
            }
            Ok(Some(value)) => add!(
                Level::Fail,
                "image",
                format!("runtime image label is {value:?}, expected {RUNTIME_LABEL_VALUE:?}"),
                Some("run `dshctl migrate` and pull the v2 image")
            ),
            Ok(None) => add!(
                Level::Warn,
                "image",
                "image is not present locally",
                Some("run `dshctl deploy`")
            ),
            Err(error) => add!(
                Level::Warn,
                "image",
                format!("failed to inspect image: {error:#}"),
                None
            ),
        }
        if let Ok(services) = docker.ps() {
            if services.is_empty() {
                add!(
                    Level::Warn,
                    "health",
                    "no running services",
                    Some("run `dshctl up`")
                );
            } else {
                add!(
                    Level::Ok,
                    "health",
                    "compose reports at least one service",
                    None
                );
            }
        }
    }

    if args.network {
        findings.extend(network_findings(config));
    }

    let failed = findings
        .iter()
        .filter(|finding| finding.level == Level::Fail)
        .count();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&findings)?);
    } else {
        for finding in &findings {
            println!(
                "[{}] {}: {}",
                finding.level.as_str(),
                finding.code,
                finding.message
            );
            if let Some(hint) = &finding.hint {
                println!("       hint: {hint}");
            }
        }
    }
    if failed > 0 {
        bail!("doctor found {failed} failure(s)");
    }
    Ok(())
}

fn cmd_logs(config: &Config, verbose: bool, args: LogsArgs) -> Result<()> {
    let docker = docker(config, verbose);
    docker.require()?;
    docker.logs(args.follow, args.tail)
}

fn cmd_update(config: &Config, verbose: bool, args: UpdateArgs) -> Result<()> {
    let mut config = config.clone();
    if args.source {
        git_pull(&config.project_dir)?;
        config = Config::load(&config.project_dir)?;
    }
    let docker_client = docker(&config, verbose);
    docker_client.require()?;
    docker_client.cleanup_legacy()?;
    if args.backup {
        let path = backup::create(&config, None, true, Some(&docker_client))?;
        println!("Backup: {}", path.display());
    }
    runtime::init(&config, runtime::InitOptions::default())?;
    preflight_port(&config, &docker_client)?;
    config.image_mode = ImageMode::Pull;
    let docker = docker(&config, verbose);
    docker.compose_config_check()?;
    docker.prepare_image()?;
    docker.up()?;
    wait_health(&config)?;
    status_text(&config, &docker, true)
}

pub fn set_env_key(path: &Path, key: &str, val: &str) -> Result<()> {
    let content = if path.is_file() {
        fs::read_to_string(path).context("failed to read .env")?
    } else {
        String::new()
    };
    let mut replaced = false;
    let mut lines: Vec<String> = content
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with(&format!("{key}="))
                || (trimmed.starts_with('#')
                    && trimmed
                        .trim_start_matches('#')
                        .trim_start()
                        .starts_with(&format!("{key}=")))
            {
                replaced = true;
                format!("{key}={val}")
            } else {
                line.to_string()
            }
        })
        .collect();

    if !replaced {
        lines.push(format!("{key}={val}"));
    }

    let mut output = lines.join("\n");
    output.push('\n');
    fs::write(path, output).context("failed to write .env")?;
    Ok(())
}

fn fetch_latest_dsh_version(config: &Config) -> Result<String> {
    let mut urls = Vec::new();
    if config.build_mirror == "cn" || config.image_mirror == dsh_config::MirrorMode::Cn {
        urls.push("https://registry.npmmirror.com/@deepseek-ai/dsh/latest");
    }
    urls.push("https://registry.npmjs.org/@deepseek-ai%2Fdsh/latest");

    let agent = http_agent();
    for url in urls {
        match agent.get(url).call() {
            Ok(response) => {
                let json: serde_json::Value = response
                    .into_json()
                    .with_context(|| format!("failed to parse JSON from {url}"))?;
                if let Some(version) = json.get("version").and_then(|v| v.as_str()) {
                    return Ok(version.to_string());
                }
            }
            Err(e) => {
                if config.build_mirror == "cn" || config.image_mirror == dsh_config::MirrorMode::Cn
                {
                    continue;
                }
                return Err(anyhow::anyhow!(
                    "failed to query npm registry at {url}: {e}"
                ));
            }
        }
    }
    bail!("could not determine latest @deepseek-ai/dsh version from npm registry");
}

fn detect_current_dsh_version(docker: &Docker, config: &Config) -> Option<String> {
    if let Ok(ver) = docker.exec_capture(&["dsh".to_string(), "--version".to_string()]) {
        let trimmed = ver.trim();
        if !trimmed.is_empty() && !trimmed.contains("error") && !trimmed.contains("not found") {
            return Some(trimmed.to_string());
        }
    }
    if !config.dsh_version.is_empty() && config.dsh_version != "latest" {
        return Some(config.dsh_version.clone());
    }
    None
}

fn cmd_upgrade(config: &Config, verbose: bool, args: UpgradeArgs) -> Result<()> {
    let docker = docker(config, verbose);
    docker.require()?;

    println!("Checking upstream DSH releases on npm registry...");
    let target = match args.version {
        Some(v) => v,
        None => fetch_latest_dsh_version(config)?,
    };

    let current = detect_current_dsh_version(&docker, config);

    println!(
        "Current DSH version : {}",
        current.as_deref().unwrap_or("unknown")
    );
    println!("Upstream DSH version: {target}");

    if args.check {
        if let Some(curr) = &current {
            if curr == &target {
                println!("Status              : Up to date");
            } else {
                println!("Status              : Upgrade available (run `dshctl upgrade` to apply)");
            }
        }
        return Ok(());
    }

    if let Some(curr) = &current {
        if curr == &target && !args.rebuild {
            println!(
                "DSH is already up to date ({curr}). Use --rebuild if you want to rebuild the container image."
            );
            return Ok(());
        }
    }

    if !args.yes && io::stdin().is_terminal() {
        print!("Upgrade DSH to {target}? [Y/n]: ");
        io::stdout().flush()?;
        let mut response = String::new();
        io::stdin().read_line(&mut response)?;
        let trimmed = response.trim();
        if !trimmed.is_empty()
            && !trimmed.eq_ignore_ascii_case("y")
            && !trimmed.eq_ignore_ascii_case("yes")
        {
            println!("Aborted.");
            return Ok(());
        }
    }

    set_env_key(&config.project_dir.join(".env"), "DSH_VERSION", &target)?;
    let mut updated_config = Config::load(&config.project_dir)?;
    updated_config.dsh_version = target.clone();

    let is_running = docker.container_running()?;

    if args.rebuild || !is_running {
        println!("Rebuilding DSH image with DSH_VERSION={target}...");
        docker.compose_config_check()?;
        docker.build_image()?;
        if is_running {
            docker.up()?;
            wait_health(&updated_config)?;
        }
    } else {
        println!("Performing live in-container upgrade to @deepseek-ai/dsh@{target}...");
        let mut install_cmd = vec![
            "npm".to_string(),
            "install".to_string(),
            "-g".to_string(),
            "--prefix".to_string(),
            "/opt/dsh".to_string(),
        ];
        if config.build_mirror == "cn" || config.image_mirror == dsh_config::MirrorMode::Cn {
            install_cmd.push("--registry=https://registry.npmmirror.com".to_string());
        }
        install_cmd.push(format!("@deepseek-ai/dsh@{target}"));

        let status = docker.exec_as_user("node", &install_cmd)?;
        if !status.success() {
            bail!("npm install inside container failed with {status}");
        }

        println!("Restarting container to apply changes...");
        docker.restart()?;
        wait_health(&updated_config)?;
    }

    if let Some(final_ver) = detect_current_dsh_version(&docker, &updated_config) {
        println!("Successfully upgraded DSH to {final_ver}!");
    } else {
        println!("Upgraded DSH to {target}.");
    }

    Ok(())
}

fn cmd_rebuild(config: &Config, verbose: bool, args: RebuildArgs) -> Result<()> {
    let mut config = config.clone();
    config.image_mode = ImageMode::Build;
    if args.slim {
        config.image_variant = ImageVariant::Slim;
    }
    let docker = docker(&config, verbose);
    docker.require()?;
    docker.cleanup_legacy()?;
    runtime::init(&config, runtime::InitOptions::default())?;
    ensure_auth(&config, args.yes)?;
    docker.compose_config_check()?;
    preflight_port(&config, &docker)?;
    docker.build_image()?;
    docker.up()?;
    wait_health(&config)?;
    status_text(&config, &docker, true)
}

fn cmd_user_add(config: &Config, args: crate::UserAddArgs) -> Result<()> {
    validate_username(&args.name)?;
    let mut store = runtime::load_users(config)?;
    let existing = store.contains(&args.name);
    if args.password_stdin && args.password_file.is_some() {
        bail!("--password-stdin and --password-file are mutually exclusive");
    }
    let generated = args.generate;
    let password = if generated {
        let password = Zeroizing::new(random_password()?);
        eprintln!(
            "Generated password for '{}': {}",
            args.name,
            password.as_str()
        );
        eprintln!("Save it now; it will not be shown again.");
        password
    } else if args.password_stdin {
        let mut value = String::new();
        io::stdin()
            .read_line(&mut value)
            .context("failed to read password from stdin")?;
        while value.ends_with(['\n', '\r']) {
            value.pop();
        }
        Zeroizing::new(value)
    } else if let Some(path) = &args.password_file {
        let value = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Zeroizing::new(value.trim_end_matches(['\n', '\r']).to_owned())
    } else {
        prompt_password_pair()?
    };
    if args.insecure_password {
        if password.is_empty() {
            bail!("password must not be empty");
        }
        if password.len() > dsh_auth::MAX_PASSWORD_BYTES {
            bail!(
                "password must be at most {} bytes",
                dsh_auth::MAX_PASSWORD_BYTES
            );
        }
    } else {
        validate_password(config, &password)?;
    }
    let cost = args.cost.unwrap_or(config.bcrypt_cost);
    let hash = hash_password(password.as_bytes(), cost)?;
    store.upsert(&args.name, hash);
    runtime::write_users(config, &store)?;

    if existing {
        println!("User '{}' updated.", args.name);
    } else {
        println!("User '{}' created.", args.name);
    }
    println!("Auth file: {}", config.auth_file().display());
    println!("Plaintext password was not written to disk.");
    Ok(())
}

fn cmd_user_list(config: &Config, json: bool) -> Result<()> {
    let store = runtime::load_users(config)?;
    if json {
        let names: Vec<_> = store.names().collect();
        println!("{}", serde_json::to_string_pretty(&names)?);
    } else if store.is_empty() {
        println!("No users configured.");
    } else {
        for name in store.names() {
            println!("{name}");
        }
    }
    Ok(())
}

fn cmd_user_remove(config: &Config, args: crate::UserRemoveArgs) -> Result<()> {
    validate_username(&args.name)?;
    let mut store = runtime::load_users(config)?;
    if !store.contains(&args.name) {
        bail!("user '{}' does not exist", args.name);
    }
    if !args.yes && !confirm(&format!("Remove user '{}'? [y/N] ", args.name))? {
        println!("Aborted.");
        return Ok(());
    }
    store.remove(&args.name);
    runtime::write_users(config, &store)?;
    println!("User '{}' removed.", args.name);
    if store.is_empty() {
        eprintln!(
            "WARNING: no users remain; the gateway will reject all requests until a user is created."
        );
    }
    Ok(())
}

fn cmd_ssh(config: &Config, command: SshCommands) -> Result<()> {
    let manager = SshManager::new(config);
    match command {
        SshCommands::List(args) => {
            let keys = manager.list()?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&keys)?);
            } else if keys.is_empty() {
                println!("No SSH keys in {}", manager.dir().display());
            } else {
                println!(
                    "{:<24} {:<16} {:<10} FINGERPRINT",
                    "NAME", "TYPE", "PRIVATE"
                );
                for key in keys {
                    println!(
                        "{:<24} {:<16} {:<10} {}",
                        key.name,
                        key.key_type.as_deref().unwrap_or("-"),
                        if key.private { "yes" } else { "no" },
                        key.fingerprint.as_deref().unwrap_or("-")
                    );
                }
            }
        }
        SshCommands::Generate(args) => {
            let info = manager.generate(
                args.name.as_deref(),
                &args.key_type,
                args.comment.as_deref(),
                args.no_passphrase,
                args.force,
            )?;
            print_key_info(&info);
            println!(
                "Private key: {} (not printed)",
                manager.private_path(&info.name).display()
            );
        }
        SshCommands::Import(args) => {
            let info = manager.import(
                &args.private_key,
                args.public_key.as_deref(),
                args.name.as_deref(),
                args.force,
            )?;
            print_key_info(&info);
            println!(
                "Private key: {} (not printed)",
                manager.private_path(&info.name).display()
            );
        }
        SshCommands::Remove(args) => manager.remove(&args.name, args.yes)?,
        SshCommands::Test(args) => manager.test(&args.host, args.key.as_deref())?,
    }
    Ok(())
}

fn print_key_info(info: &crate::ssh::KeyInfo) {
    println!("Name       : {}", info.name);
    println!("Type       : {}", info.key_type.as_deref().unwrap_or("-"));
    println!(
        "Fingerprint: {}",
        info.fingerprint.as_deref().unwrap_or("-")
    );
    if let Some(public) = &info.public {
        println!("Public key : {public}");
    }
}

fn cmd_git(config: &Config, command: GitCommands) -> Result<()> {
    let git = GitConfig::new(config);
    match command {
        GitCommands::Setup(args) => git.setup(
            args.name.as_deref(),
            args.email.as_deref(),
            args.key.as_deref(),
        ),
        GitCommands::Show => git.show(),
    }
}

fn cmd_restore(config: &Config, verbose: bool, args: RestoreArgs) -> Result<()> {
    let docker = docker(config, verbose);
    let path = if args.archive.is_absolute() {
        args.archive
    } else {
        config.project_dir.join(args.archive)
    };
    backup::restore(config, &path, args.yes, Some(&docker))?;
    println!("Restored from {}", path.display());
    wait_health(config)
}

fn cmd_image_check(config: &Config, verbose: bool, network: bool) -> Result<()> {
    let docker = docker(config, verbose);
    docker.require()?;
    let image = config.effective_image();
    let mut failed = false;
    println!("Checking image: {image}");
    match docker.manifest_inspect(&image) {
        Ok(()) => println!("[OK]   manifest is accessible"),
        Err(error) => {
            failed = true;
            println!("[FAIL] manifest: {error:#}");
        }
    }

    if network {
        match resolve_host("ghcr.io") {
            Ok(addresses) if !addresses.is_empty() => {
                println!("[OK]   ghcr.io resolves to {:?}", addresses)
            }
            Ok(_) => {
                failed = true;
                println!("[FAIL] ghcr.io did not resolve");
            }
            Err(error) => {
                failed = true;
                println!("[FAIL] ghcr.io DNS: {error:#}");
            }
        }
        match resolve_host("pkg-containers.githubusercontent.com") {
            Ok(addresses) if !addresses.is_empty() => {
                println!("[OK]   layer CDN resolves to {:?}", addresses)
            }
            Ok(_) => {
                failed = true;
                println!("[FAIL] layer CDN did not resolve");
            }
            Err(error) => {
                failed = true;
                println!("[FAIL] layer CDN DNS: {error:#}");
            }
        }
        let agent = http_agent();
        match agent.get("https://ghcr.io/v2/").call() {
            Ok(response) => println!("[INFO] GHCR registry returned HTTP {}", response.status()),
            Err(ureq::Error::Status(401, _)) => {
                println!("[OK]   GHCR registry reachable (HTTP 401 is normal before auth)")
            }
            Err(ureq::Error::Status(code, _)) => {
                println!("[INFO] GHCR registry returned HTTP {code}")
            }
            Err(ureq::Error::Transport(error)) => {
                failed = true;
                println!("[FAIL] GHCR registry: {error}");
            }
        }
    }

    if failed {
        bail!("image check failed");
    }
    println!(
        "\nIf the manifest is fast but pulls stall on Downloading/Waiting, the problem is usually the layer CDN or outbound bandwidth."
    );
    Ok(())
}

fn cmd_migrate(config: &Config, verbose: bool, args: MigrateArgs) -> Result<()> {
    let dest_dir = args.to.clone().or_else(|| args.dest.clone());
    if let Some(target) = dest_dir {
        return cmd_migrate_dir(config, verbose, target, args);
    }

    let actions = [
        "copy .runtime/auth/users.caddy to users.htpasswd".to_owned(),
        "update DSH_IMAGE from the v1 latest tag to :2".to_owned(),
        "create missing runtime directories and permissions".to_owned(),
        "run `dshctl deploy` afterwards to remove the old Caddy service".to_owned(),
    ];
    println!("Migration plan:");
    for action in &actions {
        println!("  - {action}");
    }
    if args.dry_run {
        println!("Dry run: no files were changed.");
        return Ok(());
    }

    let migrated = runtime::migrate_legacy_auth(config)?;
    runtime::init(config, runtime::InitOptions::default())?;
    if migrated {
        println!("Migrated users.caddy to users.htpasswd (legacy file kept).");
    } else {
        println!("No legacy auth migration was needed.");
    }
    if update_env_image(config)? {
        println!("Updated DSH_IMAGE to the v2 tag (previous .env backed up).");
    }
    println!("\nNext: dshctl deploy");
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)
            .with_context(|| format!("failed to create directory {}", dst.display()))?;
    }
    for entry in
        fs::read_dir(src).with_context(|| format!("failed to read directory {}", src.display()))?
    {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if entry_type.is_symlink() {
            #[cfg(unix)]
            {
                let link_target = fs::read_link(&src_path)?;
                let _ = fs::remove_file(&dst_path);
                std::os::unix::fs::symlink(&link_target, &dst_path)?;
            }
            #[cfg(windows)]
            {
                let _ = fs::copy(&src_path, &dst_path);
            }
        } else if entry_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = fs::metadata(&src_path) {
                    let _ = fs::set_permissions(&dst_path, metadata.permissions());
                }
            }
        }
    }
    Ok(())
}

fn cmd_migrate_dir(
    config: &Config,
    verbose: bool,
    target: PathBuf,
    args: MigrateArgs,
) -> Result<()> {
    let source = config
        .project_dir
        .canonicalize()
        .unwrap_or_else(|_| config.project_dir.clone());
    let target = if target.starts_with("~") {
        if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(format!("{}{}", home, &target.to_string_lossy()[1..]))
        } else {
            target
        }
    } else if target.is_absolute() {
        target
    } else {
        std::env::current_dir()?.join(target)
    };

    if source == target {
        bail!(
            "source directory and target directory are the same: {}",
            source.display()
        );
    }

    println!("================================================================");
    println!("             DSH 生产部署目录迁移 (Migration)                   ");
    println!("================================================================");
    println!("当前目录 (Source) : {}", source.display());
    println!("目标目录 (Target) : {}", target.display());
    println!(
        "迁移模式 (Mode)   : {}",
        if args.copy {
            "复制 (Copy)"
        } else {
            "移动 (Move / Relocate)"
        }
    );
    println!(
        "自动启动 (Deploy) : {}",
        if args.no_deploy {
            "否 (No)"
        } else {
            "是 (Yes)"
        }
    );

    if target.exists() && !args.force {
        let is_empty = fs::read_dir(&target)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !is_empty {
            bail!(
                "目标目录 '{}' 已存在且非空；若要覆盖请加上 --force",
                target.display()
            );
        }
    }

    if args.dry_run {
        println!("\nDry run: 演练完成，未进行任何实际文件操作。");
        return Ok(());
    }

    if !args.yes && io::stdin().is_terminal() {
        print!("\n确认执行迁移操作吗? [Y/n]: ");
        io::stdout().flush()?;
        let mut resp = String::new();
        io::stdin().read_line(&mut resp)?;
        let t = resp.trim();
        if !t.is_empty() && !t.eq_ignore_ascii_case("y") && !t.eq_ignore_ascii_case("yes") {
            println!("已取消操作。");
            return Ok(());
        }
    }

    // 1. 停止原目录正在运行的容器服务
    let docker_client = docker(config, verbose);
    if docker_client.require().is_ok() {
        println!("正在安全停止源目录中运行的容器服务...");
        let _ = docker_client.down();
    }

    // 2. 确保目标目录的上级目录存在
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建目标父目录失败: {}", parent.display()))?;
    }

    // 3. 复制所有文件与运行时持久化数据到目标目录
    println!("正在同步所有文件与持久化数据至 {}...", target.display());
    copy_dir_recursive(&source, &target)?;

    // 4. 更新系统软链接 (如 /usr/local/bin/dshctl)
    #[cfg(unix)]
    {
        let usr_local_bin = Path::new("/usr/local/bin/dshctl");
        if let Ok(link_target) = fs::read_link(usr_local_bin) {
            if link_target.starts_with(&source) {
                let new_target = target.join("bin/dshctl");
                let _ = fs::remove_file(usr_local_bin);
                let _ = std::os::unix::fs::symlink(&new_target, usr_local_bin);
                println!(
                    "已更新系统全局软链接: /usr/local/bin/dshctl -> {}",
                    new_target.display()
                );
            }
        }
    }

    // 5. 移动模式下清理源目录
    if !args.copy {
        println!("正在清理原安装目录 {}...", source.display());
        fs::remove_dir_all(&source)
            .with_context(|| format!("删除原目录失败: {}", source.display()))?;
    }

    // 6. 恢复目标目录的执行权限与入口软链接
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let target_dshctl = target.join("bin/dshctl");
        if target_dshctl.is_file() {
            let _ = fs::set_permissions(&target_dshctl, fs::Permissions::from_mode(0o755));
        }
        let target_entrypoint = target.join("bin/dsh-entrypoint");
        if target_entrypoint.is_file() {
            let _ = fs::set_permissions(&target_entrypoint, fs::Permissions::from_mode(0o755));
        }
        let root_launcher = target.join("dshctl");
        let _ = fs::remove_file(&root_launcher);
        let _ = std::os::unix::fs::symlink("bin/dshctl", &root_launcher);
    }

    // 7. 在新目录下启动服务并完成健康检查
    if !args.no_deploy {
        println!("正在从新目录启动 DSH 服务...");
        let new_config = Config::load(&target)?;
        let new_docker = Docker::new(new_config.clone(), verbose);
        new_docker.require()?;
        new_docker.compose_config_check()?;
        new_docker.up()?;
        wait_health(&new_config)?;
        status_text(&new_config, &new_docker, true)?;
    }

    println!("\n🎉 成功将项目完整迁移至新目录: {}", target.display());
    println!(
        "后续管理请进入新目录执行: cd {} && ./dshctl status",
        target.display()
    );
    Ok(())
}

fn cmd_completion(args: CompletionArgs) -> Result<()> {
    let mut command = Cli::command();
    clap_complete::generate(args.shell, &mut command, "dshctl", &mut io::stdout());
    Ok(())
}

fn cmd_self_update(args: SelfUpdateArgs) -> Result<()> {
    let tag = latest_release_tag()?;
    let current = env!("CARGO_PKG_VERSION");
    println!("Current: {current}; latest: {tag}");
    if args.check {
        return Ok(());
    }
    let is_current = match (
        semver::Version::parse(current),
        semver::Version::parse(tag.trim_start_matches('v')),
    ) {
        (Ok(current), Ok(latest)) => latest <= current,
        _ => tag.trim_start_matches('v') == current,
    };
    if is_current {
        println!("dshctl is already up to date.");
        return Ok(());
    }
    let (target, extension) = target_asset()?;
    let asset = format!("dshctl-{target}.{extension}");
    let base = format!("{RELEASE_DOWNLOAD}/{tag}");
    let checksums = download_text(&format!("{base}/SHA256SUMS"))?;
    let expected = checksums
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?;
            (name == asset).then(|| hash.to_owned())
        })
        .with_context(|| format!("SHA256SUMS does not contain {asset}"))?;

    let work = tempfile::tempdir().context("failed to create update directory")?;
    let archive = work.path().join(&asset);
    download_to(&format!("{base}/{asset}"), &archive)?;
    let actual = sha256_file(&archive)?;
    if !actual.eq_ignore_ascii_case(&expected) {
        bail!("checksum mismatch for {asset}: expected {expected}, got {actual}");
    }

    if cfg!(windows) {
        let destination = std::env::current_dir()?.join(&asset);
        fs::copy(&archive, &destination)?;
        bail!(
            "downloaded and verified {}; close dshctl and replace the executable manually",
            destination.display()
        );
    }

    let unpack = work.path().join("unpack");
    fs::create_dir_all(&unpack)?;
    let file = File::open(&archive)?;
    let decoder = flate2::read::GzDecoder::new(io::BufReader::new(file));
    let mut tar = tar::Archive::new(decoder);
    tar.unpack(&unpack)?;
    let binary = find_file(&unpack, "dshctl").context("archive does not contain dshctl")?;
    let current_exe = std::env::current_exe()?;
    let staged = current_exe.with_extension("new");
    fs::copy(&binary, &staged)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
    }
    fs::rename(&staged, &current_exe)?;
    println!("Updated dshctl to {tag}.");
    Ok(())
}

fn ensure_auth(config: &Config, yes: bool) -> Result<()> {
    if runtime::has_users(config)? {
        return Ok(());
    }
    if yes || !io::stdin().is_terminal() {
        bail!(
            "no Basic Auth users are configured; run `dshctl user add <name>` first \
             or deploy interactively"
        );
    }
    println!("No authentication users found. Create one now.");
    let name = prompt("Username: ")?;
    validate_username(&name)?;
    let password = prompt_password_pair()?;
    validate_password(config, &password)?;
    let hash = hash_password(password.as_bytes(), config.bcrypt_cost)?;
    let mut store = runtime::load_users(config)?;
    store.upsert(&name, hash);
    runtime::write_users(config, &store)?;
    println!("User '{name}' created.");
    Ok(())
}

fn prompt_password_pair() -> Result<Zeroizing<String>> {
    let password = Zeroizing::new(rpassword::prompt_password("Password: ")?);
    let confirmation = Zeroizing::new(rpassword::prompt_password("Confirm password: ")?);
    if password.as_str() != confirmation.as_str() {
        bail!("passwords do not match");
    }
    Ok(password)
}

fn validate_password(config: &Config, password: &str) -> Result<()> {
    if password.is_empty() {
        bail!("password must not be empty");
    }
    if password.len() < config.min_password_len {
        bail!(
            "password must be at least {} characters (AUTH_MIN_PASSWORD_LENGTH)",
            config.min_password_len
        );
    }
    if password.len() > dsh_auth::MAX_PASSWORD_BYTES {
        bail!(
            "password must be at most {} bytes",
            dsh_auth::MAX_PASSWORD_BYTES
        );
    }
    Ok(())
}

fn random_password() -> Result<String> {
    let mut bytes = [0u8; 24];
    getrandom(&mut bytes).map_err(|error| anyhow::anyhow!("failed to read randomness: {error}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn preflight_port(config: &Config, docker: &Docker) -> Result<()> {
    match port_available(config) {
        Ok(()) => Ok(()),
        Err(_) if docker.port_owned(config.dsh_port) => {
            println!(
                "Port {} is owned by this project; it will be recreated.",
                config.dsh_port
            );
            Ok(())
        }
        Err(error) => Err(error).context(format!(
            "host port {} is already in use; change DSH_PORT or stop the other process",
            config.dsh_port
        )),
    }
}

fn port_available(config: &Config) -> Result<()> {
    let ip: IpAddr = config.bind_ip()?;
    let address = SocketAddr::new(ip, config.dsh_port);
    TcpListener::bind(address)
        .map(drop)
        .with_context(|| format!("cannot bind {address}"))
}

fn wait_health(config: &Config) -> Result<()> {
    let url = config_url(config);
    let mut last_error = None;
    for _ in 0..90 {
        match probe_http(config) {
            Ok(200 | 401) => {
                println!("Health check OK: {url}");
                return Ok(());
            }
            Ok(code) => last_error = Some(anyhow::anyhow!("HTTP {code}")),
            Err(error) => last_error = Some(error),
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("health check timed out")))
        .with_context(|| format!("{url} did not become healthy"))
}

fn probe_http(config: &Config) -> Result<u16> {
    let host = if config.bind_address == "0.0.0.0"
        || config.bind_address == "::"
        || config.bind_address == "[::]"
    {
        "127.0.0.1".to_owned()
    } else if config.bind_address.starts_with('[') {
        config.bind_address.clone()
    } else if config.bind_address.contains(':') {
        format!("[{}]", config.bind_address)
    } else {
        config.bind_address.clone()
    };
    let url = format!("http://{host}:{}/", config.dsh_port);
    match http_agent().get(&url).call() {
        Ok(response) => Ok(response.status()),
        Err(ureq::Error::Status(code, _)) => Ok(code),
        Err(ureq::Error::Transport(error)) => Err(anyhow::anyhow!("{error}")),
    }
}

fn config_url(config: &Config) -> String {
    let raw = if config.bind_address == "0.0.0.0"
        || config.bind_address == "::"
        || config.bind_address == "[::]"
    {
        "127.0.0.1"
    } else {
        config.bind_address.as_str()
    };
    let host = if raw.starts_with('[') || !raw.contains(':') {
        raw.to_owned()
    } else {
        format!("[{raw}]")
    };
    format!("http://{host}:{}", config.dsh_port)
}

fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build()
}

fn resolve_host(host: &str) -> Result<Vec<SocketAddr>> {
    Ok((host, 443)
        .to_socket_addrs()
        .with_context(|| format!("failed to resolve {host}"))?
        .collect())
}

fn network_findings(config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    match resolve_host("ghcr.io") {
        Ok(addresses) if !addresses.is_empty() => findings.push(Finding::ok(
            "network.ghcr",
            format!("ghcr.io resolves to {:?}", addresses),
        )),
        _ => findings.push(Finding::fail(
            "network.ghcr",
            "ghcr.io does not resolve",
            Some("check DNS or use DSH_IMAGE_MIRROR=cn"),
        )),
    }
    match resolve_host("pkg-containers.githubusercontent.com") {
        Ok(addresses) if !addresses.is_empty() => findings.push(Finding::ok(
            "network.cdn",
            format!("layer CDN resolves to {:?}", addresses),
        )),
        _ => findings.push(Finding::fail(
            "network.cdn",
            "layer CDN does not resolve",
            Some("image layer downloads will stall; configure a registry mirror"),
        )),
    }
    if config.image.is_empty() {
        findings.push(Finding::fail("image", "DSH_IMAGE is empty", None));
    }
    findings
}

fn git_pull(project_dir: &Path) -> Result<()> {
    let status = ProcessCommand::new("git")
        .arg("-C")
        .arg(project_dir)
        .args(["status", "--porcelain"])
        .output()
        .context("failed to run git")?;
    if !status.status.success() {
        bail!("{} is not a git checkout", project_dir.display());
    }
    if !status.stdout.is_empty() {
        bail!(
            "source checkout has uncommitted changes; commit/stash them before `update --source`"
        );
    }
    for args in [vec!["fetch", "--prune"], vec!["pull", "--ff-only"]] {
        let status = ProcessCommand::new("git")
            .arg("-C")
            .arg(project_dir)
            .args(args)
            .status()
            .context("failed to run git")?;
        if !status.success() {
            bail!("git command failed");
        }
    }
    Ok(())
}

fn update_env_image(config: &Config) -> Result<bool> {
    let path = config.project_dir.join(".env");
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(false);
    };
    let updated = text
        .replace(
            "DSH_IMAGE=ghcr.io/rin721/dsh-docker:latest",
            "DSH_IMAGE=ghcr.io/reina0721/dsh-docker:2",
        )
        .replace(
            "DSH_IMAGE=\"ghcr.io/rin721/dsh-docker:latest\"",
            "DSH_IMAGE=\"ghcr.io/reina0721/dsh-docker:2\"",
        )
        .replace(
            "DSH_IMAGE=ghcr.io/rin721/dsh-docker:2",
            "DSH_IMAGE=ghcr.io/reina0721/dsh-docker:2",
        )
        .replace(
            "DSH_IMAGE=\"ghcr.io/rin721/dsh-docker:2\"",
            "DSH_IMAGE=\"ghcr.io/reina0721/dsh-docker:2\"",
        )
        .replace(
            "DSH_IMAGE=ghcr.io/reina0721/dsh-docker:latest",
            "DSH_IMAGE=ghcr.io/reina0721/dsh-docker:2",
        )
        .replace(
            "DSH_IMAGE=\"ghcr.io/reina0721/dsh-docker:latest\"",
            "DSH_IMAGE=\"ghcr.io/reina0721/dsh-docker:2\"",
        );
    if updated == text {
        return Ok(false);
    }
    let backup = path.with_extension(format!("bak-{}", unix_timestamp()));
    fs::copy(&path, &backup)?;
    let mut file = File::create(&path)?;
    file.write_all(updated.as_bytes())?;
    file.sync_all()?;
    runtime::set_mode(&path, 0o600)?;
    Ok(true)
}

fn print_archive_info(path: &Path) -> Result<()> {
    let size = fs::metadata(path)?.len();
    let hash = sha256_file(path)?;
    println!("Size: {} bytes", size);
    println!("SHA256: {hash}");
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn latest_release_tag() -> Result<String> {
    let agent = http_agent();
    let response = agent
        .get(RELEASE_API)
        .set("User-Agent", "dshctl")
        .call()
        .context("failed to query GitHub Releases")?;
    let value: serde_json::Value = response.into_json()?;
    value
        .get("tag_name")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .context("GitHub release response has no tag_name")
}

fn target_asset() -> Result<(&'static str, &'static str)> {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("x86_64-unknown-linux-musl", "tar.gz"),
        ("linux", "aarch64") => ("aarch64-unknown-linux-musl", "tar.gz"),
        ("macos", "x86_64") => ("x86_64-apple-darwin", "tar.gz"),
        ("macos", "aarch64") => ("aarch64-apple-darwin", "tar.gz"),
        ("windows", "x86_64") => ("x86_64-pc-windows-msvc", "zip"),
        (os, arch) => bail!("no prebuilt dshctl for {os}/{arch}"),
    };
    Ok(target)
}

fn download_text(url: &str) -> Result<String> {
    let response = http_agent()
        .get(url)
        .set("User-Agent", "dshctl")
        .call()
        .with_context(|| format!("failed to download {url}"))?;
    response.into_string().context("failed to read response")
}

fn download_to(url: &str, path: &Path) -> Result<()> {
    let response = http_agent()
        .get(url)
        .set("User-Agent", "dshctl")
        .call()
        .with_context(|| format!("failed to download {url}"))?;
    let mut reader = response.into_reader();
    let mut file = File::create(path)?;
    io::copy(&mut reader, &mut file)?;
    file.sync_all()?;
    Ok(())
}

fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(root).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.file_name().and_then(|value| value.to_str()) == Some(name) {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        }
    }
    None
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim().to_owned())
}

fn confirm(label: &str) -> Result<bool> {
    let answer = prompt(label)?;
    Ok(matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes"))
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Ok,
    Warn,
    Fail,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Serialize)]
struct Finding {
    level: Level,
    code: String,
    message: String,
    hint: Option<String>,
}

impl Finding {
    fn ok(code: &str, message: impl Into<String>) -> Self {
        Self {
            level: Level::Ok,
            code: code.to_owned(),
            message: message.into(),
            hint: None,
        }
    }

    fn fail(code: &str, message: impl Into<String>, hint: Option<&str>) -> Self {
        Self {
            level: Level::Fail,
            code: code.to_owned(),
            message: message.into(),
            hint: hint.map(ToOwned::to_owned),
        }
    }
}

impl Serialize for Level {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_env_key_updates_existing_and_commented() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = dir.path().join(".env");
        fs::write(&env_file, "PORT=3080\n# DSH_VERSION=0.2.0\nOTHER=test\n").unwrap();

        set_env_key(&env_file, "DSH_VERSION", "0.2.1-rc.5").unwrap();
        let content = fs::read_to_string(&env_file).unwrap();
        assert!(content.contains("DSH_VERSION=0.2.1-rc.5"));
        assert!(!content.contains("# DSH_VERSION=0.2.0"));

        set_env_key(&env_file, "NEW_KEY", "value").unwrap();
        let content2 = fs::read_to_string(&env_file).unwrap();
        assert!(content2.contains("NEW_KEY=value"));
    }

    #[test]
    fn test_copy_dir_recursive_copies_files_and_subdirs() {
        let src_dir = tempfile::tempdir().unwrap();
        let dst_dir = tempfile::tempdir().unwrap();

        let sub = src_dir.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(src_dir.path().join("file1.txt"), b"hello").unwrap();
        fs::write(sub.join("file2.txt"), b"world").unwrap();

        let target = dst_dir.path().join("migrated");
        copy_dir_recursive(src_dir.path(), &target).unwrap();

        assert_eq!(fs::read(target.join("file1.txt")).unwrap(), b"hello");
        assert_eq!(fs::read(target.join("sub/file2.txt")).unwrap(), b"world");
    }
}
