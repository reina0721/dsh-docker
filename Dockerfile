# syntax=docker/dockerfile:1.7

ARG NODE_IMAGE=node:24.18.0-bookworm-slim
ARG GO_IMAGE=golang:1.26.5-bookworm
ARG RUST_IMAGE=rust:1.90.0-bookworm
# BuildKit supplies these for multi-arch builds. The fallbacks keep a plain
# single-platform `docker build` working without overriding BuildKit's values.
ARG BUILDPLATFORM

# =============================================================================
# Rust gateway / entrypoint
# =============================================================================
FROM --platform=${BUILDPLATFORM:-linux/amd64} ${RUST_IMAGE} AS gateway-build

ARG TARGETARCH
ARG BUILDARCH
ARG CARGO_REGISTRY=

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY assets ./assets

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    set -eux; \
    if [ -n "${CARGO_REGISTRY}" ]; then \
        case "${CARGO_REGISTRY}" in \
            rsproxy) registry="https://rsproxy.cn/index/" ;; \
            *) registry="${CARGO_REGISTRY}" ;; \
        esac; \
        printf '[source.crates-io]\nreplace-with = "mirror"\n[source.mirror]\nregistry = "sparse+%s"\n' "${registry}" \
            > /usr/local/cargo/config.toml; \
    fi; \
    case "${TARGETARCH:-amd64}" in \
        amd64) \
            triple="x86_64-unknown-linux-gnu" ;; \
        arm64) \
            triple="aarch64-unknown-linux-gnu"; \
            if [ "${BUILDARCH:-amd64}" != "arm64" ]; then \
                apt-get update; \
                apt-get install -y --no-install-recommends gcc-aarch64-linux-gnu libc6-dev-arm64-cross; \
                export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc; \
                export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc; \
                export AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar; \
            fi ;; \
        *) echo "unsupported TARGETARCH=${TARGETARCH:-amd64}" >&2; exit 1 ;; \
    esac; \
    rustup target add "${triple}" || true; \
    cargo build --release --locked -p dsh-gateway --bin dsh-entrypoint --target "${triple}"; \
    mkdir -p /out; \
    cp "target/${triple}/release/dsh-entrypoint" /out/dsh-entrypoint; \
    chmod 0755 /out/dsh-entrypoint

# =============================================================================
# Go toolchain (dev target only)
# =============================================================================
FROM ${GO_IMAGE} AS go-toolchain

# =============================================================================
# Runtime / slim target
# =============================================================================
FROM ${NODE_IMAGE} AS runtime

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

ARG DEBIAN_FRONTEND=noninteractive
ARG BUILD_MIRROR=global
ARG APT_MIRROR=
ARG NPM_REGISTRY=
ARG PNPM_VERSION=11.7.0
ARG DSH_VERSION=latest
ARG VERSION=dev
ARG REVISION=unknown
ARG CREATED=unknown

ENV LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    TZ=UTC \
    HOME=/home/node \
    XDG_CACHE_HOME=/home/node/.cache \
    NPM_CONFIG_PREFIX=/opt/dsh \
    DSH_HOME=/home/node/.dsh \
    DSH_PERMISSION_MODE=workspace-write \
    DSH_TELEMETRY_DISABLED=1 \
    NODE_USE_ENV_PROXY=1 \
    NPM_CONFIG_FUND=false \
    NPM_CONFIG_UPDATE_NOTIFIER=false \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PATH=/opt/dsh/bin:/home/node/.local/bin:${PATH}

# Keep downloaded packages for BuildKit apt caches.
RUN rm -f /etc/apt/apt.conf.d/docker-clean; \
    printf 'Binary::apt::APT::Keep-Downloaded-Packages "true";\n' \
      > /etc/apt/apt.conf.d/keep-cache

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    set -eux; \
    if [ "${BUILD_MIRROR}" = "cn" ] && [ -f /etc/apt/sources.list.d/debian.sources ]; then \
        sed -i "s|deb.debian.org|${APT_MIRROR:-mirrors.tuna.tsinghua.edu.cn}|g" \
            /etc/apt/sources.list.d/debian.sources; \
    fi; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        bash \
        bash-completion \
        ca-certificates \
        curl \
        wget \
        git \
        git-lfs \
        openssh-client \
        build-essential \
        make \
        pkg-config \
        python3 \
        python3-pip \
        python3-venv \
        python-is-python3 \
        jq \
        ripgrep \
        fd-find \
        fzf \
        tree \
        less \
        nano \
        vim-tiny \
        tmux \
        file \
        rsync \
        unzip \
        zip \
        xz-utils \
        zstd \
        iproute2 \
        procps \
        lsof \
        tini; \
    ln -sf /usr/bin/fdfind /usr/local/bin/fd; \
    git lfs install --system

RUN set -eux; \
    mkdir -p \
        /workspace \
        /home/node/.dsh \
        /home/node/.local/bin \
        /opt/dsh \
        /home/node/.cache \
        /home/node/.npm \
        /home/node/go/bin \
        /home/node/go/pkg/mod; \
    chown -R node:node /workspace /home/node /opt/dsh; \
    git config --system --add safe.directory /workspace

USER node

RUN --mount=type=cache,target=/home/node/.npm,uid=1000,gid=1000 \
    set -eux; \
    registry_arg=""; \
    if [ -n "${NPM_REGISTRY}" ]; then registry_arg="--registry=${NPM_REGISTRY}"; fi; \
    npm install --global --no-audit --no-fund ${registry_arg} \
        "pnpm@${PNPM_VERSION}" \
        typescript@latest \
        tsx@latest \
        "@deepseek-ai/dsh@${DSH_VERSION}"; \
    node --version; \
    npm --version; \
    pnpm --version; \
    dsh --version

USER root

COPY --from=gateway-build --chmod=0755 /out/dsh-entrypoint /usr/local/bin/dsh-entrypoint
COPY LICENSE-MIT LICENSE-APACHE /usr/share/licenses/dsh-docker/

LABEL org.opencontainers.image.title="DSH Docker" \
      org.opencontainers.image.description="DeepSeek Harness development environment with a Rust gateway" \
      org.opencontainers.image.source="https://github.com/rin721/dsh-docker" \
      org.opencontainers.image.documentation="https://github.com/rin721/dsh-docker#readme" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}" \
      io.github.rin721.dsh-docker.runtime="2" \
      io.github.rin721.dsh-docker.cli-compat="2" \
      io.github.rin721.dsh-docker.variant="slim"

USER node
WORKDIR /workspace
EXPOSE 3080
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=15s --timeout=5s --start-period=60s --retries=5 \
    CMD ["/usr/local/bin/dsh-entrypoint", "healthcheck"]
ENTRYPOINT ["/usr/bin/tini", "-g", "--", "/usr/local/bin/dsh-entrypoint"]

# =============================================================================
# Dev target: full toolchains for interactive coding
# =============================================================================
FROM runtime AS dev

USER root

ARG DEBIAN_FRONTEND=noninteractive
ARG BUILD_MIRROR=global
ARG APT_MIRROR=
ARG NODE_VERSION=24.18.0
ARG GO_VERSION=1.26.5
ARG RUST_VERSION=1.90.0
ARG PNPM_VERSION=11.7.0
ARG DSH_VERSION=latest
ARG INSTALL_GO_DEV_TOOLS=true
ARG NPM_REGISTRY=
ARG PIP_INDEX_URL=
ARG GOPROXY=
ARG RUSTUP_DIST_SERVER=
ARG VERSION=dev
ARG REVISION=unknown
ARG CREATED=unknown

ENV GOPATH=/home/node/go \
    GOMODCACHE=/home/node/go/pkg/mod \
    GOCACHE=/home/node/.cache/go-build \
    RUSTUP_HOME=/home/node/.rustup \
    CARGO_HOME=/home/node/.cargo \
    PATH=/opt/dsh/bin:/usr/local/go/bin:/home/node/go/bin:/home/node/.cargo/bin:/home/node/.local/bin:${PATH}

COPY --from=go-toolchain /usr/local/go /usr/local/go

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    set -eux; \
    if [ "${BUILD_MIRROR}" = "cn" ] && [ -f /etc/apt/sources.list.d/debian.sources ]; then \
        sed -i "s|deb.debian.org|${APT_MIRROR:-mirrors.tuna.tsinghua.edu.cn}|g" \
            /etc/apt/sources.list.d/debian.sources; \
    fi; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        cmake \
        ninja-build \
        clang \
        clangd \
        gdb \
        strace \
        libssl-dev \
        openssl \
        zlib1g-dev \
        libsqlite3-dev \
        sqlite3 \
        protobuf-compiler \
        pipx \
        shellcheck; \
    go version

USER node

# Do not cache-mount RUSTUP_HOME/toolchains: rustup renames from tmp into
# toolchains, and separate filesystems can fail with EXDEV.
RUN set -eux; \
    curl --proto '=https' --tlsv1.2 --fail --show-error --silent \
        https://sh.rustup.rs -o /tmp/rustup-init.sh; \
    if [ -n "${RUSTUP_DIST_SERVER}" ]; then \
        export RUSTUP_DIST_SERVER; \
        export RUSTUP_UPDATE_ROOT="${RUSTUP_DIST_SERVER}/rustup"; \
        sh /tmp/rustup-init.sh -y --no-modify-path --profile minimal --default-toolchain "${RUST_VERSION}"; \
    else \
        sh /tmp/rustup-init.sh -y --no-modify-path --profile minimal --default-toolchain "${RUST_VERSION}"; \
    fi; \
    rm -f /tmp/rustup-init.sh; \
    rustup component add rustfmt clippy rust-analyzer; \
    rustc --version; \
    cargo --version

RUN --mount=type=cache,target=/home/node/go/pkg/mod,uid=1000,gid=1000 \
    --mount=type=cache,target=/home/node/.cache/go-build,uid=1000,gid=1000 \
    set -eux; \
    if [ "${INSTALL_GO_DEV_TOOLS}" = "true" ]; then \
        if [ -n "${GOPROXY}" ]; then export GOPROXY="${GOPROXY}"; fi; \
        go install golang.org/x/tools/gopls@latest; \
        go install golang.org/x/tools/cmd/goimports@latest; \
        go install github.com/go-delve/delve/cmd/dlv@latest; \
        go install honnef.co/go/tools/cmd/staticcheck@latest; \
        go install github.com/go-task/task/v3/cmd/task@latest; \
    fi; \
    go version

LABEL org.opencontainers.image.title="DSH Docker (dev)" \
      io.github.rin721.dsh-docker.variant="dev" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}"

USER node
WORKDIR /workspace
