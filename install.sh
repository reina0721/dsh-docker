#!/usr/bin/env bash
# DSH Docker installer.
#
# Usage:
#   Interactive mode (recommended):
#     curl -fsSL https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.sh | bash
#     # 或国内加速:
#     curl -fsSL https://ghfast.top/https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.sh | bash
#
#   Non-interactive mode (options/flags):
#     bash install.sh [--dir DIR] [--port PORT] [--bind IP] [--variant dev|slim]
#                     [--mirror global|cn|off] [--admin USER] [--password PASS]
#                     [--version vX.Y.Z] [--no-deploy] [--yes] [--system-link]
#                     [--insecure]
set -Eeuo pipefail

REPO="reina0721/dsh-docker"
FALLBACK_VERSION="v2.0.3"

# Defaults
TARGET_DIR="${DSHCTL_DIR:-$HOME/dsh-docker}"
TARGET_DIR_EXPLICIT=0
PORT="${DSH_PORT:-3080}"
PORT_EXPLICIT=0
BIND_ADDR="${BIND_ADDRESS:-127.0.0.1}"
BIND_EXPLICIT=0
VARIANT="${DSH_IMAGE_VARIANT:-dev}"
VARIANT_EXPLICIT=0
MIRROR_POLICY="${DSH_IMAGE_MIRROR:-global}"
MIRROR_EXPLICIT=0
ADMIN_USER="admin"
ADMIN_USER_EXPLICIT=0
ADMIN_PASS=""
ADMIN_PASS_EXPLICIT=0
PASSWORD_MODE="${DSH_PASSWORD_MODE:-}"
PASSWORD_MODE_EXPLICIT=0
MIN_PASSWORD_LEN=12
VERSION="${DSHCTL_VERSION:-latest}"
DO_DEPLOY=1
EXPLICIT_YES=0
SYSTEM_LINK=0
SYSTEM_LINK_EXPLICIT=0
INSTALL_COMPLETIONS=0
INSECURE=0
FROM_SOURCE=0
DOWNLOAD_MIRRORS="https://ghfast.top https://gh-proxy.com"

usage() {
    cat <<'EOF'
DSH Docker 安装向导 (Interactive & Self-Contained Installer)

用法:
  bash install.sh [选项]

选项:
  --dir DIR              项目安装目录 (默认: ~/dsh-docker)
  --port PORT            Web 访问端口 (默认: 3080)
  --bind IP              监听绑定地址 (默认: 127.0.0.1)
  --variant dev|slim     开发环境变体 (默认: dev)
  --mirror global|cn|off 网络加速策略 (默认: global)
  --admin USER           管理员用户名 (默认: admin)
  --password PASS        管理员访问密码 (留空自动生成)
  --password-mode MOD    密码安全模式: strong|weak (默认: strong，密码>=12位; weak 模式允许简短密码>=6位)
  --weak-password        允许弱密码模式 (快捷选项)
  --strong-password      强制强密码模式 (快捷选项)
  --min-password-length N 指定最小密码长度
  --version vX.Y.Z       指定发布版本 (默认: latest)
  --system-link          在 /usr/local/bin 创建全局 dshctl 软链接
  --no-deploy            仅下载解压与初始化，不立即启动 Docker 构建
  --from-source          从当前源码编译 (本地开发使用)
  --completions          安装 shell 补全 (bash/zsh/fish)
  --yes, -y              非交互模式，遇到未指定项使用默认值
  --insecure             跳过 Checksum 校验 (不推荐)
  -h, --help             显示此帮助信息
EOF
}

log() { printf '%s\n' "$*"; }
warn() { printf '\033[33mwarning: %s\033[0m\n' "$*" >&2; }
die() { printf '\033[31merror: %s\033[0m\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --dir) TARGET_DIR="$2"; TARGET_DIR_EXPLICIT=1; shift 2 ;;
        --port) PORT="$2"; PORT_EXPLICIT=1; shift 2 ;;
        --bind) BIND_ADDR="$2"; BIND_EXPLICIT=1; shift 2 ;;
        --variant) VARIANT="$2"; VARIANT_EXPLICIT=1; shift 2 ;;
        --mirror) MIRROR_POLICY="$2"; MIRROR_EXPLICIT=1; shift 2 ;;
        --admin) ADMIN_USER="$2"; ADMIN_USER_EXPLICIT=1; shift 2 ;;
        --password) ADMIN_PASS="$2"; ADMIN_PASS_EXPLICIT=1; shift 2 ;;
        --password-mode|--password-strength)
            case "$2" in
                weak|strong) PASSWORD_MODE="$2"; PASSWORD_MODE_EXPLICIT=1; shift 2 ;;
                *) die "invalid --password-mode: must be 'strong' or 'weak'" ;;
            esac
            ;;
        --weak-password) PASSWORD_MODE="weak"; PASSWORD_MODE_EXPLICIT=1; shift ;;
        --strong-password) PASSWORD_MODE="strong"; PASSWORD_MODE_EXPLICIT=1; shift ;;
        --min-password-length) MIN_PASSWORD_LEN="$2"; shift 2 ;;
        --version) VERSION="$2"; shift 2 ;;
        --system-link) SYSTEM_LINK=1; SYSTEM_LINK_EXPLICIT=1; shift ;;
        --no-deploy) DO_DEPLOY=0; shift ;;
        --from-source) FROM_SOURCE=1; shift ;;
        --completions) INSTALL_COMPLETIONS=1; shift ;;
        --yes|-y) EXPLICIT_YES=1; shift ;;
        --insecure) INSECURE=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

# Expand ~ in TARGET_DIR
case "$TARGET_DIR" in
    "~"*) TARGET_DIR="${HOME}${TARGET_DIR#"~"}" ;;
esac

case "$MIRROR_POLICY" in
    global|cn|off) ;;
    *) die "--mirror must be global, cn, or off" ;;
esac

case "$VARIANT" in
    dev|slim) ;;
    *) die "--variant must be dev or slim" ;;
esac

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

# TTY / Interactive input helpers
IS_TTY=0
if [ "$EXPLICIT_YES" -eq 0 ]; then
    if [ -t 0 ] || [ -c /dev/tty ]; then
        IS_TTY=1
    fi
fi

prompt_input() {
    local msg="$1"
    local def="$2"
    local var="$3"
    local val=""
    if [ -c /dev/tty ]; then
        printf '%s' "$msg" > /dev/tty
        read -r val < /dev/tty || true
    else
        printf '%s' "$msg"
        read -r val || true
    fi
    val="${val#"${val%%[![:space:]]*}"}"
    val="${val%"${val##*[![:space:]]}"}"
    if [ -z "$val" ]; then
        val="$def"
    fi
    eval "$var=\"\$val\""
}

prompt_secret() {
    local msg="$1"
    local var="$2"
    local val=""
    if [ -c /dev/tty ]; then
        printf '%s' "$msg" > /dev/tty
        read -r -s val < /dev/tty || true
        printf '\n' > /dev/tty
    else
        printf '%s' "$msg"
        read -r -s val || true
        printf '\n'
    fi
    eval "$var=\"\$val\""
}

generate_random_password() {
    if command -v openssl >/dev/null 2>&1; then
        openssl rand -base64 24 | tr -dc 'a-zA-Z0-9!@#$%^&*' | head -c 16
    else
        LC_ALL=C tr -dc 'a-zA-Z0-9!@#$%^&*' < /dev/urandom | head -c 16
    fi
}

check_port_occupied() {
    local p="$1"
    if command -v ss >/dev/null 2>&1; then
        ss -tuln 2>/dev/null | grep -Eq ":${p}\b" && return 0
    elif command -v netstat >/dev/null 2>&1; then
        netstat -tuln 2>/dev/null | grep -Eq ":${p}\b" && return 0
    elif command -v lsof >/dev/null 2>&1; then
        lsof -i ":${p}" >/dev/null 2>&1 && return 0
    fi
    return 1
}

# ==============================================================================
# Interactive wizard (when in terminal and not forced non-interactive)
# ==============================================================================
if [ "$IS_TTY" -eq 1 ]; then
    cat <<'EOF'
================================================================
            DSH Docker 交互式安装向导 (v2)
================================================================
本向导将引导您完成 DSH 开发环境的配置与安装。
所有项目产物将独立集中存放在您指定的项目目录下。
EOF

    # 1. Target Directory
    if [ "$TARGET_DIR_EXPLICIT" -eq 0 ]; then
        printf '\n[1/7] 项目安装目录 (Installation Directory)\n'
        printf '      将在此目录下存放 dshctl、Docker 编排文件、配置与工作区数据。\n'
        prompt_input "请输入安装目录 [默认: ${TARGET_DIR}]: " "${TARGET_DIR}" TARGET_DIR
        case "$TARGET_DIR" in
            "~"*) TARGET_DIR="${HOME}${TARGET_DIR#"~"}" ;;
        esac
    fi

    # 2. Port
    if [ "$PORT_EXPLICIT" -eq 0 ]; then
        printf '\n[2/7] 外部访问端口 (Port Mapping)\n'
        printf '      DSH Web 服务的访问端口。\n'
        while true; do
            prompt_input "请输入访问端口 [默认: ${PORT}]: " "${PORT}" PORT
            if ! printf '%s\n' "$PORT" | grep -Eq '^[0-9]+$' || [ "$PORT" -lt 1 ] || [ "$PORT" -gt 65535 ]; then
                warn "端口必须是 1 到 65535 之间的整数"
                continue
            fi
            if check_port_occupied "$PORT"; then
                warn "检测到端口 ${PORT} 当前已被占用，请确认或更换端口。"
            fi
            break
        done
    fi

    # 3. Bind address
    if [ "$BIND_EXPLICIT" -eq 0 ]; then
        printf '\n[3/7] 监听绑定地址 (Bind Address)\n'
        printf '      1) 127.0.0.1 (推荐: 仅本机访问，适合搭配 Nginx/Caddy 等反向代理) [默认]\n'
        printf '      2) 0.0.0.0   (局域网或公网直接访问)\n'
        prompt_input "请选择监听地址 [1/2, 默认: 1]: " "1" BIND_CHOICE
        case "$BIND_CHOICE" in
            2|0.0.0.0) BIND_ADDR="0.0.0.0" ;;
            *) BIND_ADDR="127.0.0.1" ;;
        esac
    fi

    # 4. Variant
    if [ "$VARIANT_EXPLICIT" -eq 0 ]; then
        printf '\n[4/7] 开发环境变体 (Environment Variant)\n'
        printf '      1) dev  (完整开发环境: 包含 Node, DSH, Go, Rust, pnpm, Python) [默认]\n'
        printf '      2) slim (轻量精简环境: 仅包含 Node, DSH, Git, SSH 基础依赖)\n'
        prompt_input "请选择变体 [1/2, 默认: 1]: " "1" VARIANT_CHOICE
        case "$VARIANT_CHOICE" in
            2|slim) VARIANT="slim" ;;
            *) VARIANT="dev" ;;
        esac
    fi

    # 5. Mirror
    if [ "$MIRROR_EXPLICIT" -eq 0 ]; then
        printf '\n[5/7] 网络加速策略 (Network Acceleration)\n'
        printf '      1) global (官方默认源: GitHub / Docker Hub / NPM 官方源) [默认]\n'
        printf '      2) cn     (中国大陆加速: 使用国内 GitHub 代理、NPM 镜像及 Docker 镜像源)\n'
        prompt_input "请选择加速策略 [1/2, 默认: 1]: " "1" MIRROR_CHOICE
        case "$MIRROR_CHOICE" in
            2|cn) MIRROR_POLICY="cn" ;;
            *) MIRROR_POLICY="global" ;;
        esac
    fi

    # 6. Password Strength Mode
    if [ "$PASSWORD_MODE_EXPLICIT" -eq 0 ]; then
        printf '\n[6/7] 密码安全强度模式 (Password Security Mode)\n'
        printf '      1) strong (强密码模式: 密码至少12位，安全性高) [默认]\n'
        printf '      2) weak   (弱密码模式: 允许简短密码，至少6位，如 123456，适合测试环境)\n'
        prompt_input "请选择密码强度模式 [1/2, 默认: 1]: " "1" PASS_MODE_CHOICE
        case "$PASS_MODE_CHOICE" in
            2|weak)
                PASSWORD_MODE="weak"
                MIN_PASSWORD_LEN=6
                ;;
            *)
                PASSWORD_MODE="strong"
                MIN_PASSWORD_LEN=12
                ;;
        esac
    fi

    # 7. Admin Account
    if [ "$ADMIN_USER_EXPLICIT" -eq 0 ]; then
        printf '\n[7/7] 管理员账号设置 (Basic Auth)\n'
        prompt_input "请输入管理员用户名 [默认: admin]: " "admin" ADMIN_USER
    fi
    if [ "$ADMIN_PASS_EXPLICIT" -eq 0 ]; then
        prompt_secret "请输入管理员访问密码 (至少${MIN_PASSWORD_LEN}位，直接回车自动生成随机安全密码): " ADMIN_PASS
        if [ -z "$ADMIN_PASS" ]; then
            ADMIN_PASS="$(generate_random_password)"
            PASSWORD_WAS_GENERATED=1
        else
            while [ "${#ADMIN_PASS}" -lt "$MIN_PASSWORD_LEN" ]; do
                warn "密码长度至少需要 ${MIN_PASSWORD_LEN} 位（当前为 ${#ADMIN_PASS} 位）"
                prompt_secret "请重新输入密码 (直接回车自动生成): " ADMIN_PASS
                if [ -z "$ADMIN_PASS" ]; then
                    ADMIN_PASS="$(generate_random_password)"
                    PASSWORD_WAS_GENERATED=1
                    break
                fi
            done
        fi
    fi

    # Confirm summary
    printf '\n----------------------------------------------------------------\n'
    printf '安装配置摘要:\n'
    printf '  项目目录 : %s\n' "$TARGET_DIR"
    printf '  访问端口 : %s\n' "$PORT"
    printf '  绑定地址 : %s\n' "$BIND_ADDR"
    printf '  环境变体 : %s\n' "$VARIANT"
    printf '  加速策略 : %s\n' "$MIRROR_POLICY"
    printf '  管理账号 : %s\n' "$ADMIN_USER"
    printf '  密码模式 : %s (最小长度: %s位)\n' "${PASSWORD_MODE:-strong}" "$MIN_PASSWORD_LEN"
    if [ "${PASSWORD_WAS_GENERATED:-0}" -eq 1 ]; then
        printf '  管理密码 : %s (自动生成)\n' "$ADMIN_PASS"
    else
        printf '  管理密码 : [已设置]\n'
    fi
    printf '----------------------------------------------------------------\n'
    prompt_input "确认并开始安装? [Y/n]: " "Y" CONFIRM_INSTALL
    case "$CONFIRM_INSTALL" in
        n*|N*) log "已取消安装。"; exit 0 ;;
    esac
fi

# Determine password mode and validation for non-interactive / explicit values
if [ -z "$PASSWORD_MODE" ]; then
    if [ -n "$ADMIN_PASS" ] && [ "${#ADMIN_PASS}" -lt 12 ]; then
        PASSWORD_MODE="weak"
        MIN_PASSWORD_LEN=6
    else
        PASSWORD_MODE="strong"
        MIN_PASSWORD_LEN=12
    fi
fi

if [ "$PASSWORD_MODE" = "weak" ] && [ "$MIN_PASSWORD_LEN" -gt 6 ]; then
    MIN_PASSWORD_LEN=6
fi

# Ensure password exists for non-interactive
if [ -z "$ADMIN_PASS" ]; then
    ADMIN_PASS="$(generate_random_password)"
    PASSWORD_WAS_GENERATED=1
fi

if [ "${#ADMIN_PASS}" -lt "$MIN_PASSWORD_LEN" ]; then
    die "密码长度至少需要 ${MIN_PASSWORD_LEN} 位（当前为 ${#ADMIN_PASS} 位），如需弱密码请使用 --weak-password 或 --password-mode weak"
fi

# ==============================================================================
# Download & Extract Release Bundle
# ==============================================================================
mkdir -p "$TARGET_DIR"
mkdir -p "$TARGET_DIR/bin"
cd "$TARGET_DIR"

if [ "$FROM_SOURCE" -eq 1 ]; then
    if [ ! -f Cargo.toml ] || [ ! -f compose.yaml ]; then
        die "--from-source must run from the repository checkout"
    fi
    need cargo
    log "Building dshctl and dsh-entrypoint from source..."
    cargo build --release --locked -p dshctl
    cargo build --release --locked -p dsh-gateway --bin dsh-entrypoint || true
    cp target/release/dshctl "$TARGET_DIR/bin/dshctl"
    if [ -f target/release/dsh-entrypoint ]; then
        cp target/release/dsh-entrypoint "$TARGET_DIR/bin/dsh-entrypoint"
    fi
    cp compose.yaml compose.build.yaml Dockerfile .env.example "$TARGET_DIR/" 2>/dev/null || true
    mkdir -p "$TARGET_DIR/assets"
    cp -r assets/* "$TARGET_DIR/assets/" 2>/dev/null || true
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
            for base in $DOWNLOAD_MIRRORS; do
                printf '%s\n' "${base%/}/$url"
            done
            printf '%s\n' "$url"
        else
            printf '%s\n' "$url"
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
    log "Downloading DSH package ${VERSION} for ${TARGET}..."
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

    # Extract all assets directly into TARGET_DIR
    log "Extracting files to ${TARGET_DIR}..."
    tar -xzf "${TMP}/${ASSET}" -C "$TARGET_DIR"
fi

# Ensure binaries are placed correctly inside TARGET_DIR/bin
if [ ! -f "$TARGET_DIR/bin/dshctl" ] && [ -f "$TARGET_DIR/dshctl" ]; then
    mkdir -p "$TARGET_DIR/bin"
    cp "$TARGET_DIR/dshctl" "$TARGET_DIR/bin/dshctl"
fi

chmod +x "$TARGET_DIR/bin/dshctl"
[ -f "$TARGET_DIR/bin/dsh-entrypoint" ] && chmod +x "$TARGET_DIR/bin/dsh-entrypoint" || true

# Root launcher symlink inside TARGET_DIR so user can run ./dshctl directly
ln -sf "bin/dshctl" "$TARGET_DIR/dshctl"

DSHCTL="$TARGET_DIR/bin/dshctl"
[ -x "$DSHCTL" ] || die "dshctl binary missing in ${TARGET_DIR}/bin"

# ==============================================================================
# Configure .env in TARGET_DIR
# ==============================================================================
ENV_FILE="$TARGET_DIR/.env"
if [ ! -f "$ENV_FILE" ]; then
    if [ -f "$TARGET_DIR/.env.example" ]; then
        cp "$TARGET_DIR/.env.example" "$ENV_FILE"
    else
        "$DSHCTL" --dir "$TARGET_DIR" init >/dev/null 2>&1 || true
    fi
fi

# Update settings in .env
replace_env() {
    local key="$1"
    local val="$2"
    local file="$3"
    if grep -Eq "^${key}=" "$file" 2>/dev/null; then
        sed -i "s|^${key}=.*|${key}=${val}|" "$file"
    elif grep -Eq "^#\s*${key}=" "$file" 2>/dev/null; then
        sed -i "s|^#\s*${key}=.*|${key}=${val}|" "$file"
    else
        printf '%s=%s\n' "$key" "$val" >> "$file"
    fi
}

replace_env "DSH_PORT" "$PORT" "$ENV_FILE"
replace_env "BIND_ADDRESS" "$BIND_ADDR" "$ENV_FILE"
replace_env "DSH_IMAGE_VARIANT" "$VARIANT" "$ENV_FILE"
replace_env "DSH_IMAGE_MODE" "build" "$ENV_FILE"
replace_env "DSH_IMAGE" "dsh-docker:local" "$ENV_FILE"
replace_env "DSH_IMAGE_MIRROR" "$MIRROR_POLICY" "$ENV_FILE"
replace_env "AUTH_MIN_PASSWORD_LENGTH" "$MIN_PASSWORD_LEN" "$ENV_FILE"

if [ "$MIRROR_POLICY" = "cn" ]; then
    replace_env "BUILD_MIRROR" "cn" "$ENV_FILE"
    replace_env "NPM_REGISTRY" "https://registry.npmmirror.com" "$ENV_FILE"
    replace_env "GOPROXY" "https://goproxy.cn,direct" "$ENV_FILE"
fi

# ==============================================================================
# Initialize Runtime & Create Admin User
# ==============================================================================
log "Initializing runtime directories in ${TARGET_DIR}..."
"$DSHCTL" --dir "$TARGET_DIR" init

log "Configuring admin account '${ADMIN_USER}'..."
USER_ADD_FLAGS=("--password-stdin")
if [ "$PASSWORD_MODE" = "weak" ] || [ "$MIN_PASSWORD_LEN" -lt 12 ]; then
    USER_ADD_FLAGS+=("--insecure-password")
fi
printf '%s\n' "$ADMIN_PASS" | "$DSHCTL" --dir "$TARGET_DIR" user add "$ADMIN_USER" "${USER_ADD_FLAGS[@]}" >/dev/null

# Ensure runtime directories are accessible to container node user
if [ "$(id -u)" -eq 0 ] || [ -n "${SUDO_UID:-}" ]; then
    chown -R 1000:1000 "$TARGET_DIR/.runtime" 2>/dev/null || true
    chmod 755 "$TARGET_DIR" "$TARGET_DIR/.runtime" "$TARGET_DIR/.runtime/auth" 2>/dev/null || true
    [ -f "$TARGET_DIR/.runtime/auth/users.htpasswd" ] && chmod 644 "$TARGET_DIR/.runtime/auth/users.htpasswd" 2>/dev/null || true
fi

# Optional completions
if [ "$INSTALL_COMPLETIONS" -eq 1 ]; then
    if [ -d "$HOME/.local/share/bash-completion/completions" ]; then
        "$DSHCTL" completion bash > "$HOME/.local/share/bash-completion/completions/dshctl" || true
    fi
    if [ -d "$HOME/.zsh/completions" ]; then
        "$DSHCTL" completion zsh > "$HOME/.zsh/completions/_dshctl" || true
    fi
    if [ -d "$HOME/.config/fish/completions" ]; then
        "$DSHCTL" completion fish > "$HOME/.config/fish/completions/dshctl.fish" || true
    fi
fi

# Optional system-wide symlink
if [ "$SYSTEM_LINK_EXPLICIT" -eq 0 ] && [ "$IS_TTY" -eq 1 ]; then
    printf '\n是否在 /usr/local/bin/dshctl 创建软链接方便全局执行? [y/N]: ' > /dev/tty
    read -r LINK_CHOICE < /dev/tty || true
    case "$LINK_CHOICE" in
        y*|Y*) SYSTEM_LINK=1 ;;
    esac
fi

if [ "$SYSTEM_LINK" -eq 1 ]; then
    if [ -w "/usr/local/bin" ]; then
        ln -sf "$TARGET_DIR/bin/dshctl" /usr/local/bin/dshctl
        log "已创建系统全局快捷方式: /usr/local/bin/dshctl"
    elif command -v sudo >/dev/null 2>&1; then
        sudo ln -sf "$TARGET_DIR/bin/dshctl" /usr/local/bin/dshctl
        log "已通过 sudo 创建系统全局快捷方式: /usr/local/bin/dshctl"
    else
        warn "/usr/local/bin 不可写，未能创建全局快捷方式。"
    fi
fi

# ==============================================================================
# Local Build & Deploy
# ==============================================================================
if ! command -v docker >/dev/null 2>&1; then
    warn "Docker 未安装或未运行。所有文件已安装于 ${TARGET_DIR}。"
    warn "请在安装 Docker 后，进入该目录执行 ./dshctl deploy 完成部署。"
    DO_DEPLOY=0
fi

if [ "$DO_DEPLOY" -eq 1 ]; then
    log
    log "================================================================"
    log "正在本地极速构建 Docker 镜像并启动容器..."
    log "================================================================"
    cd "$TARGET_DIR"
    if [ "$MIRROR_POLICY" = "global" ]; then
        "$DSHCTL" --dir "$TARGET_DIR" deploy --yes
    else
        "$DSHCTL" --dir "$TARGET_DIR" deploy --mirror "$MIRROR_POLICY" --yes
    fi
fi

# ==============================================================================
# Success Summary
# ==============================================================================
cat <<EOF

================================================================
               🎉 DSH Docker 安装部署成功!
================================================================
服务地址 : http://${BIND_ADDR}:${PORT}
管理账号 : ${ADMIN_USER}
管理密码 : ${ADMIN_PASS}

项目目录 : ${TARGET_DIR}
所有产物与数据已集中存放在上述项目目录下。

常用管理命令 (在项目目录下执行):
  ./dshctl status        - 查看运行状态与容器健康
  ./dshctl logs          - 实时跟踪日志
  ./dshctl stop / up     - 停止 / 启动服务
  ./dshctl restart       - 重启服务
  ./dshctl user add      - 添加新的登录用户
  ./dshctl doctor        - 诊断部署与环境
================================================================
EOF
