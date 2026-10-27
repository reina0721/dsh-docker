# Configuration

`dshctl` reads `.env` from the project directory. Process environment variables
override `.env`; defaults fill the rest. Values are never sourced as shell code.

## Runtime

| Variable | Default | Notes |
| --- | --- | --- |
| `RUNTIME_DIR` | `./.runtime` | Relative to the project directory |
| `RUNTIME_UID` / `RUNTIME_GID` | `1000` | Used for chown when running as root |
| `BIND_ADDRESS` | `127.0.0.1` | Host bind IP; bracket IPv6 (`[::1]`) in `.env` |
| `DSH_PORT` | `3080` | Published host port |
| `DSH_PROXY_PORT` | `3080` | Gateway port inside the container |
| `DSH_INTERNAL_HOST` / `DSH_INTERNAL_PORT` | `127.0.0.1` / `3079` | DSH loopback |
| `DSH_LOG_LEVEL` | `info` | `trace`, `debug`, `info`, `warn`, `error` |
| `DSH_LOG_FORMAT` | `text` | `text` or `json` |
| `DSH_HTTP_COMPAT_SHIM` | `true` | Inject `/__dsh_http_compat.js` into HTML |
| `DSH_GATEWAY_MAX_HTML_BYTES` | `8388608` | HTML injection buffer limit |

## Image delivery

| Variable | Default | Notes |
| --- | --- | --- |
| `DSH_IMAGE_MODE` | `auto` | `auto`, `pull`, `build` |
| `DSH_IMAGE` | `ghcr.io/rin721/dsh-docker:2` | Canonical image |
| `DSH_IMAGE_VARIANT` | `dev` | `dev` or `slim`; also the Docker build target |
| `DSH_IMAGE_MIRROR` | `global` | `global`, `cn`, `off` |
| `DSH_IMAGE_MIRRORS` | empty | Comma-separated GHCR mirror hosts |

`auto` order: pull → compatible local image → local build. Pull/build inherit stdio,
so progress is visible. A local image is only reused when its
`io.github.rin721.dsh-docker.runtime` label is `2`.

`cn` tries mirrors before GHCR. If `DSH_IMAGE_MIRRORS` is empty, the built-in
examples `ghcr.nju.edu.cn,ghcr.m.daocloud.io` are used. After a mirror pull the image
is tagged back to the canonical name.

## Authentication

| Variable | Default | Notes |
| --- | --- | --- |
| `AUTH_REALM` | `DeepSeek Harness` | Basic Auth realm |
| `AUTH_BCRYPT_COST` | `14` | 4–31; lower is faster but weaker |
| `AUTH_MIN_PASSWORD_LENGTH` | `12` | New passwords only |

The auth file is `.runtime/auth/users.htpasswd` (0644) in `username hash` format.
`users.caddy` is read as a fallback and migrated by `init`/`migrate`.

## DSH build

| Variable | Default |
| --- | --- |
| `DSH_VERSION` | `latest` |
| `DSH_PERMISSION_MODE` | `workspace-write` |
| `DSH_TELEMETRY_DISABLED` | `1` |
| `INSTALL_GO_DEV_TOOLS` | `true` |
| `NODE_VERSION` | `24.18.0` |
| `GO_VERSION` | `1.26.5` |
| `RUST_VERSION` | `1.90.0` |
| `PNPM_VERSION` | `11.7.0` |

## Build acceleration (China)

| Variable | Example |
| --- | --- |
| `BUILD_MIRROR` | `cn` |
| `APT_MIRROR` | `mirrors.tuna.tsinghua.edu.cn` |
| `NPM_REGISTRY` | `https://registry.npmmirror.com` |
| `PIP_INDEX_URL` | `https://pypi.tuna.tsinghua.edu.cn/simple` |
| `GOPROXY` | `https://goproxy.cn,direct` |
| `RUSTUP_DIST_SERVER` | `https://rsproxy.cn` |
| `CARGO_REGISTRY` | `rsproxy` or a sparse registry URL |
| `NODE_IMAGE` / `GO_IMAGE` / `RUST_IMAGE` | domestic Docker Hub proxy image references |

These only affect local builds. Prebuilt images already contain the toolchains.

## Compose overrides

The CLI loads `compose.yaml`, an optional `compose.override.yaml`/`.yml`, and
`compose.build.yaml` when building. `COMPOSE_PROJECT_NAME` overrides the derived
project name.

## Precedence

```text
process environment > .env > defaults
```

`dshctl config show` prints the effective configuration with secrets redacted.
