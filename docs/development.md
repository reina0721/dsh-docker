# Development

## Prerequisites

- Rust 1.85+ (the repository pins 1.90.0 in `rust-toolchain.toml`)
- Docker Engine 24+ and Compose v2 for image/deployment tests
- Git

## Layout

```text
crates/dsh-config   typed .env parsing and validation
crates/dsh-auth     htpasswd + bcrypt
crates/dsh-gateway  Rust proxy + dsh-entrypoint binary
crates/dshctl       host CLI
assets/             browser compatibility shim (embedded at compile time)
```

## Checks

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
```

The gateway has an integration test at `crates/dsh-gateway/tests/proxy.rs`. It
starts a local upstream, issues raw HTTP requests, and verifies 401, 200, HTML
injection, compat asset serving, 403 trust rejection, and fail-closed behavior.

## Running without Docker

```bash
cargo run -p dshctl -- --dir /tmp/dsh-demo init
cargo run -p dshctl -- --dir /tmp/dsh-demo user add alice --generate --cost 4
cargo run -p dshctl -- --dir /tmp/dsh-demo user list
cargo run -p dshctl -- --dir /tmp/dsh-demo backup
```

`config`, `user`, `init`, `backup`, `restore`, and `migrate` do not require Docker
unless `--quiesce` is used.

## Docker builds

```bash
docker build --target runtime -t dsh-docker:slim .
docker build --target dev -t dsh-docker:dev .
```

The gateway build stage cross-compiles for `linux/amd64` and `linux/arm64` from the
build platform. The dev stage installs rustup and Go tools as the `node` user.

## Running the gateway directly

The binary defaults to supervise mode and expects `dsh` on PATH. For proxy-only
experiments:

```bash
DSH_AUTH_FILE=/tmp/users.htpasswd \
DSH_INTERNAL_PORT=3079 \
DSH_PROXY_PORT=3080 \
cargo run -p dsh-gateway --bin dsh-entrypoint
```

`dsh-entrypoint prepare` performs only the persistent-home setup and exits.
`dsh-entrypoint healthcheck` is suitable for Docker healthchecks.

## Code style

- `rustfmt` and `clippy -D warnings` are required.
- No `unsafe`; Unix process/signal operations go through `nix`.
- Do not capture Docker pull/build output in a variable; live progress is a
  regression-tested requirement.
- Keep the gateway's Host/Origin, hop-by-hop, and HTML injection behavior aligned
  with the tests.

## Commits

Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `chore:`). Include tests
for behavior changes. Do not commit `.env`, `.runtime`, credentials, or backups.
