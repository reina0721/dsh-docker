# DSH Docker

[简体中文](README.md) | [English](README.en.md)

[![CI](https://github.com/reina0721/dsh-docker/actions/workflows/ci.yml/badge.svg)](https://github.com/reina0721/dsh-docker/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/reina0721/dsh-docker?sort=semver)](https://github.com/reina0721/dsh-docker/releases)
[![GHCR](https://img.shields.io/badge/ghcr.io-reina0721%2Fdsh--docker-blue?logo=github)](https://github.com/reina0721/dsh-docker/pkgs/container/dsh-docker)

A long-lived Docker development environment for DeepSeek Harness (DSH). Version 2
replaces the host shell scripts, Caddy gateway, and Node proxy with Rust:

- `dshctl`: one static host CLI for init, deploy, update, diagnostics, auth, backup,
  restore, and self-update.
- `dsh-entrypoint`: a Rust container entrypoint and reverse proxy (Basic Auth,
  Host/Origin validation, HTML compatibility injection, WebSocket/SSE passthrough).
- One container: DSH listens on loopback only; the Rust gateway is the only exposed
  process.

> v2 is a breaking release. The old `./scripts/*.sh` commands are gone. See
> [migration-v1-to-v2.md](docs/migration-v1-to-v2.md).

## Quick start

Linux / macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.sh | bash
```

With the China mirror policy:

```bash
curl -fsSL https://ghfast.top/https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.sh \
  | bash -s -- --mirror cn
```

Windows PowerShell 5.1+ (Docker Desktop / WSL2 required):

```powershell
irm https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.ps1 | iex
```

The installer verifies the package checksum and launches an interactive wizard guiding you through:
- **Installation Directory** (default: `~/dsh-docker`, completely self-contained)
- **Web Port** (default: `3080`) & **Bind Address** (`127.0.0.1` or `0.0.0.0`)
- **Environment Variant** (`dev` full toolchain or `slim` lightweight)
- **Mirror Policy** (`global` or China domestic acceleration `cn`)
- **Admin Account & Password** (custom or generated)

All installation artifacts, executables (`bin/dshctl`), Compose configurations, and runtime data are placed inside your chosen project directory. Pre-compiled static `dsh-entrypoint` binaries are bundled, allowing local Docker builds to finish in 1-2 minutes without downloading heavy Rust builder images.

Open `http://127.0.0.1:3080/` and sign in with Basic Auth. The gateway captures DSH's
one-time web token (redacted in logs) and injects it until DSH sets its auth cookie;
you do not need to copy the `?token=...` URL manually.

From source (Rust 1.85+, Docker Engine 24+, Compose v2):

```bash
git clone https://github.com/reina0721/dsh-docker.git
cd dsh-docker
cargo build --release -p dshctl
./target/release/dshctl deploy
```

## Architecture

```text
Browser -> TLS reverse proxy (optional) -> 127.0.0.1:3080
  dsh container
    dsh-entrypoint (Rust, tini)
      Rust gateway :3080
        Basic Auth + bcrypt cache/reload
        Host/Origin validation
        HTML compat injection, WebSocket/SSE
      dsh web 127.0.0.1:3079
```

## Images

```dotenv
DSH_IMAGE_MODE=auto
DSH_IMAGE=ghcr.io/reina0721/dsh-docker:2
```

- `:2` / `:latest`: `dev` image with Go, Rust, Node, pnpm, DSH, and common tools.
- `:2-slim`: slim image with Node, DSH, Git/SSH, and build basics; no Go/Rust
  toolchains.

`auto` tries pull, then a compatible local image, then a local build. Pull output is
never captured, so progress stays live. Images must carry
`io.github.reina0721.dsh-docker.runtime=2`; v1 caches are rejected.

China mirror:

```bash
dshctl deploy --mirror cn
```

or in `.env`:

```dotenv
DSH_IMAGE_MIRROR=cn
DSH_IMAGE_MIRRORS=ghcr.nju.edu.cn,ghcr.m.daocloud.io
```

## Commands

| Command | Purpose |
| --- | --- |
| `dshctl init` | Create `.env`, missing `compose.yaml`, `.runtime/`, permissions |
| `dshctl deploy` | Preflight, auth, image, start, health |
| `dshctl up/down/stop/restart` | Lifecycle |
| `dshctl status [--json]` | Services, HTTP, auth, image |
| `dshctl doctor [--fix] [--network]` | Docker, config, port, permissions, image, network |
| `dshctl user add/list/remove` | Manage bcrypt Basic Auth |
| `dshctl ssh list/generate/import/remove/test` | Manage persistent Git SSH keys |
| `dshctl git setup/show` | Configure the persistent Git identity |
| `dshctl backup` / `restore` | Backup `.env` + `.runtime`, restore with rollback |
| `dshctl update` / `rebuild` | Pull latest / build locally |
| `dshctl migrate` | v1 to v2 migration |
| `dshctl self-update` | Update the binary from GitHub Releases |

See the [CLI reference](docs/cli.md) and [configuration](docs/configuration.md).

## Persistence and auth

Everything lives under `.runtime/`: `workspace`, `dsh-home`, developer home
(`ssh`, `gnupg`, `aws`, `kube`, `docker`, XDG, Git, shell, npm, Cargo config), and
`auth/users.htpasswd`. The container links only the paths that must persist, so the
image-managed toolchains stay visible.

Passwords are hashed with bcrypt locally; plaintext never reaches disk or process
arguments. The gateway caches successful credentials and reloads the file without a
restart. With no users configured the gateway fails closed with 503.

## SSH / Git identity

Keys persist in `.runtime/home/ssh`; Git config persists in
`.runtime/home/git/config`. Manage them with:

```bash
dshctl ssh generate work --type ed25519 --comment "you@example.com"
dshctl ssh import ~/.ssh/id_ed25519 --name github
dshctl ssh list
dshctl ssh test github --key work
dshctl git setup --name "Your Name" --email "you@example.com" --key work
```

Private keys are never printed or logged. See [SSH/Git](docs/ssh-git.md).

## Reverse proxy

Keep `Host` and `Origin` intact, support WebSocket upgrades, and disable buffering
for SSE. Examples: [Nginx](examples/nginx.conf), [Caddy](examples/Caddyfile),
[Traefik](examples/traefik.yaml), [1Panel](examples/1panel.md).

## Security

Basic Auth must be behind HTTPS. Backups contain SSH keys and credentials. See
[security.md](docs/security.md).

## License

MIT OR Apache-2.0.
