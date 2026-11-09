# FAQ

## What changed in v2?

Caddy and the Node proxy are gone. A Rust gateway performs Basic Auth, Host/Origin
validation, HTML compatibility injection, and WebSocket/SSE proxying. The host
scripts are replaced by the static `dshctl` binary. Compose has a single `dsh`
service. See [migration](migration-v1-to-v2.md).

## Why remove Caddy?

It was a full Go web server used for only Basic Auth and a reverse proxy, and it
added a second container plus a password-hash subprocess. The Rust gateway has a much
smaller dependency surface and runs next to DSH in the same container. TLS was
already delegated to an external proxy.

## Is TLS supported?

No. Put Nginx/Caddy/Traefik/1Panel in front and terminate TLS there. The gateway is
plain HTTP on loopback by default. Basic Auth without TLS is unsafe.

## Do I need Rust to deploy?

No. The install script downloads a prebuilt static `dshctl`. Rust is only needed to
build from source or to build the gateway inside Docker (the Docker build stage
handles it).

## Does it work without Docker?

`dshctl config`, `user`, `init`, `backup`, `restore`, and `migrate` can work without
Docker (backup `--quiesce` needs it). Deploy/status/logs need Docker.

## Which platforms are supported?

- Linux amd64/arm64: primary.
- macOS amd64/arm64: CLI + Docker Desktop.
- Windows 10/11 with Docker Desktop/WSL2: CLI; WSL2 is recommended for symlinks and
  file permissions.

## How large is the image?

The `dev` image intentionally includes Go, Rust, Node, pnpm, DSH, and development
tools. Use `DSH_IMAGE=ghcr.io/reina0721/dsh-docker:2-slim` for a smaller runtime without
Go/Rust toolchains. Exact sizes are recorded in release notes.

## Can I use a China mirror?

Yes:

```bash
dshctl deploy --mirror cn
```

or set `DSH_IMAGE_MIRROR=cn` and optionally `DSH_IMAGE_MIRRORS`. Mirrors are
third-party; the CLI tags the result back to the canonical image and checks the v2
label. Checksums/signatures are the trust root.

## Why does the first login take about a second?

bcrypt cost 14 is deliberately slow. The gateway caches successful credentials and
uses single-flight verification, so only the first request pays the cost. Lower
`AUTH_BCRYPT_COST` to 12 on a small VPS if needed.

## What happens if I remove all users?

The gateway returns 503 for every request (fail closed). The container stays up so
you can add a user without losing the session. Run `dshctl user add`.

## How do I change the port?

Set `DSH_PORT` in `.env` (or pass `--port <PORT>` to `install.sh`).
In Bridge mode, it changes the published host port. In IPVlan mode, `DSH_PROXY_PORT` automatically defaults to `DSH_PORT`.
Then run `dshctl deploy` to apply.

## How do I update?

To upgrade DSH (`@deepseek-ai/dsh`) to the latest upstream release:

```bash
dshctl upgrade
```

To update the Docker image and recreate the container:

```bash
dshctl update --backup
```

To update the `dshctl` CLI binary itself from GitHub Releases:

```bash
dshctl self-update
```

Source checkouts can use `dshctl update --source`, which refuses dirty trees.

## Where are my SSH keys?

`.runtime/home/ssh`, linked to `~/.ssh` inside the container. Backups include them.

## How do I directly bind a public IP to run development projects on arbitrary ports?

Pass `--network-ip <IP>` to `install.sh` (or set `DSH_NETWORK_MODE=ipvlan` and `DSH_NETWORK_IP=<IP>` in `.env`). The network interface, subnet, and gateway are automatically detected and derived.
In this mode, Docker port publishing is omitted completely: the container directly holds the public IP, so any project or port you run inside the container (e.g. 3000, 5173, 8080) is reachable directly from outside.
See [networking-ipvlan.md](networking-ipvlan.md) for details.
