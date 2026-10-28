#!/usr/bin/env bash
# DSH Docker installer.
#
# Usage:
#   bash install.sh [--dir DIR] [--version vX.Y.Z] [--mirror global|cn|off]
#                   [--no-deploy] [--from-source] [--system] [--completions]
#                   [--insecure]
#
# The script downloads a prebuilt dshctl, verifies SHA256SUMS, initializes a
# deployment directory, and (on a TTY) starts the interactive deploy flow.
set -Eeuo pipefail

REPO="reina0721/dsh-docker"
FALLBACK_VERSION="v2.0.0"
TARGET_DIR="${DSHCTL_DIR:-$HOME/dsh-docker}"
INSTALL_DIR="${DSHCTL_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${DSHCTL_VERSION:-latest}"
MIRROR_POLICY="global"
DO_DEPLOY=1
FROM_SOURCE=0
SYSTEM_INSTALL=0
INSTALL_COMPLETIONS=0
INSECURE=0
TARGET_DIR_EXPLICIT=0
DOWNLOAD_MIRRORS="https://ghfast.top https://gh-proxy.com"

usage() {
    cat <<'EOF'
Install dshctl and initialize a DSH Docker deployment.

Options:
  --dir DIR              Deployment directory (default: ~/dsh-docker)
  --version vX.Y.Z       Release tag (default: latest)
  --mirror POLICY        global, cn, or off (default: global)
  --no-deploy            Only install and initialize
  --from-source          Build dshctl from the current source checkout
  --system               Install to /usr/local/bin (may use sudo)
  --completions          Install shell completions when possible
  --insecure             Skip checksum verification (strongly discouraged)
  -h, --help             Show this help

Environment:
  DSHCTL_VERSION, DSHCTL_DIR, DSHCTL_INSTALL_DIR,
  DSHCTL_DOWNLOAD_MIRROR  Custom prefix or template containing {url}
  DSH_IMAGE_MIRROR        Image mirror policy passed to deploy

Domestic acceleration:
  bash install.sh --mirror cn
EOF
}

log() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --dir) TARGET_DIR="$2"; TARGET_DIR_EXPLICIT=1; shift 2 ;;
        --version) VERSION="$2"; shift 2 ;;
        --mirror) MIRROR_POLICY="$2"; shift 2 ;;
        --no-deploy) DO_DEPLOY=0; shift ;;
        --from-source) FROM_SOURCE=1; shift ;;
        --system) SYSTEM_INSTALL=1; shift ;;
        --completions) INSTALL_COMPLETIONS=1; shift ;;
        --insecure) INSECURE=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

case "$MIRROR_POLICY" in
    global|cn|off) ;;
    *) die "--mirror must be global, cn, or off" ;;
esac

if [ "$TARGET_DIR_EXPLICIT" -eq 0 ] && [ -f compose.yaml ] && [ -f Cargo.toml ]; then
    TARGET_DIR="$(pwd)"
fi

case "$(uname -s)" in
    Linux) OS="linux" ;;
    Darwin) OS="macos" ;;
    *) die "unsupported operating system: $(uname -s); use install.ps1 on Windows" ;;
esac

case "$(uname -m)" in
    x86_64|amd64) ARCH="x86_64" ;;
    arm64|aarch64) ARCH="aarch64" ;;
    *) die "unsupported architecture: $(uname -m)" ;;
esac

if [ "$OS" = "linux" ]; then
    TARGET="${ARCH}-unknown-linux-musl"
elif [ "$ARCH" = "x86_64" ]; then
    TARGET="x86_64-apple-darwin"
else
    TARGET="aarch64-apple-darwin"
fi

need() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

need curl
need tar

if command -v sha256sum >/dev/null 2>&1; then
    SHA_CMD="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
    SHA_CMD="shasum -a 256"
else
    die "need sha256sum or shasum for checksum verification"
fi

if [ "$SYSTEM_INSTALL" -eq 1 ]; then
    INSTALL_DIR="/usr/local/bin"
fi

if [ "$FROM_SOURCE" -eq 1 ]; then
    if [ ! -f Cargo.toml ] || [ ! -f compose.yaml ]; then
        die "--from-source must run from the repository checkout"
    fi
    need cargo
    log "Building dshctl from source..."
    cargo build --release --locked -p dshctl
    BINARY="target/release/dshctl"
    TARGET_DIR="$(pwd)"
else
    resolve_version() {
        if [ "$VERSION" != "latest" ]; then
            printf '%s\n' "$VERSION"
            return 0
        fi
        url="https://github.com/${REPO}/releases/latest"
        resolved="$(curl -fsSIL --retry 2 --connect-timeout 10 -o /dev/null -w '%{url_effective}' "$url" 2>/dev/null || true)"
        case "$resolved" in
            */tag/v*) printf '%s\n' "${resolved##*/}" ;;
            *) printf '%s\n' "$FALLBACK_VERSION" ;;
        esac
    }
    VERSION="$(resolve_version)"
    ASSET="dshctl-${TARGET}.tar.gz"
    BASE="https://github.com/${REPO}/releases/download/${VERSION}"
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT

    download_urls() {
        url="$1"
        if [ "$MIRROR_POLICY" = "cn" ]; then
            if [ -n "${DSHCTL_DOWNLOAD_MIRROR:-}" ]; then
                case "$DSHCTL_DOWNLOAD_MIRROR" in
                    *"{url}"*) printf '%s\n' "${DSHCTL_DOWNLOAD_MIRROR//\{url\}/$url}" ;;
                    *) printf '%s\n' "${DSHCTL_DOWNLOAD_MIRROR%/}/$url" ;;
                esac
            fi
            for base in $DOWNLOAD_MIRRORS; do
                printf '%s\n' "${base%/}/$url"
            done
            printf '%s\n' "$url"
        else
            printf '%s\n' "$url"
            if [ -n "${DSHCTL_DOWNLOAD_MIRROR:-}" ]; then
                case "$DSHCTL_DOWNLOAD_MIRROR" in
                    *"{url}"*) printf '%s\n' "${DSHCTL_DOWNLOAD_MIRROR//\{url\}/$url}" ;;
                    *) printf '%s\n' "${DSHCTL_DOWNLOAD_MIRROR%/}/$url" ;;
                esac
            fi
            for base in $DOWNLOAD_MIRRORS; do
                printf '%s\n' "${base%/}/$url"
            done
        fi
    }

    fetch() {
        source_url="$1"
        destination="$2"
        for candidate in $(download_urls "$source_url"); do
            log "Downloading $candidate"
            if curl -fL --retry 2 --connect-timeout 15 --max-time 300 \
                -o "$destination" "$candidate" 2>/dev/null; then
                return 0
            fi
        done
        return 1
    }

    ASSET_URL="${BASE}/${ASSET}"
    log "Installing dshctl ${VERSION} for ${TARGET}"
    fetch "$ASSET_URL" "${TMP}/${ASSET}" || die "failed to download ${ASSET}"

    if [ "$INSECURE" -eq 1 ]; then
        warn "skipping checksum verification (--insecure)"
    else
        fetch "${BASE}/SHA256SUMS" "${TMP}/SHA256SUMS" || die "failed to download SHA256SUMS"
        expected="$(awk -v name="$ASSET" '$2 == name { print $1 }' "${TMP}/SHA256SUMS" | head -n 1)"
        [ -n "$expected" ] || die "SHA256SUMS does not contain ${ASSET}"
        actual="$($SHA_CMD "${TMP}/${ASSET}" | awk '{ print $1 }')"
        [ "$actual" = "$expected" ] || die "checksum mismatch for ${ASSET}"
        log "Checksum verified."
    fi

    tar -xzf "${TMP}/${ASSET}" -C "${TMP}"
    BINARY="$(find "${TMP}" -type f -name dshctl | head -n 1)"
    [ -n "$BINARY" ] || die "archive did not contain dshctl"
fi

mkdir -p "$INSTALL_DIR"
if [ ! -w "$INSTALL_DIR" ]; then
    die "${INSTALL_DIR} is not writable; use --system with sudo or set DSHCTL_INSTALL_DIR"
fi
install -m 0755 "$BINARY" "${INSTALL_DIR}/dshctl"
DSHCTL="${INSTALL_DIR}/dshctl"
log "Installed: ${DSHCTL}"

case ":$PATH:" in
    *":${INSTALL_DIR}:"*) ;;
    *) warn "${INSTALL_DIR} is not on PATH; add it to your shell profile" ;;
esac

if [ "$FROM_SOURCE" -eq 0 ]; then
    mkdir -p "$TARGET_DIR"
    "$DSHCTL" --dir "$TARGET_DIR" init
else
    "$DSHCTL" --dir "$TARGET_DIR" init
fi

if [ "$INSTALL_COMPLETIONS" -eq 1 ]; then
    if [ -d "$HOME/.local/share/bash-completion/completions" ]; then
        "$DSHCTL" completion bash > "$HOME/.local/share/bash-completion/completions/dshctl"
        log "Installed bash completion."
    fi
    if [ -d "$HOME/.zsh/completions" ]; then
        "$DSHCTL" completion zsh > "$HOME/.zsh/completions/_dshctl"
        log "Installed zsh completion."
    fi
    if [ -d "$HOME/.config/fish/completions" ]; then
        "$DSHCTL" completion fish > "$HOME/.config/fish/completions/dshctl.fish"
        log "Installed fish completion."
    fi
fi

if ! command -v docker >/dev/null 2>&1; then
    warn "Docker is not installed; install Docker Engine/Desktop before deploying."
    DO_DEPLOY=0
fi

if [ -f "$TARGET_DIR/.env" ]; then
    "$DSHCTL" --dir "$TARGET_DIR" migrate >/dev/null 2>&1 || true
fi

if [ "$DO_DEPLOY" -eq 1 ] && [ -t 0 ]; then
    log
    log "Starting deployment in ${TARGET_DIR}"
    if [ "$MIRROR_POLICY" = "global" ]; then
        "$DSHCTL" --dir "$TARGET_DIR" deploy
    else
        "$DSHCTL" --dir "$TARGET_DIR" deploy --mirror "$MIRROR_POLICY"
    fi
else
    log
    log "Next steps:"
    log "  cd ${TARGET_DIR}"
    log "  ${DSHCTL} user add <name>"
    log "  ${DSHCTL} deploy"
    if [ "$MIRROR_POLICY" = "cn" ]; then
        log "  # or: ${DSHCTL} deploy --mirror cn"
    fi
fi
