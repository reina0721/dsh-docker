# DSH Docker

[简体中文](README.md) | [English](README.en.md)

[![CI](https://github.com/reina0721/dsh-docker/actions/workflows/ci.yml/badge.svg)](https://github.com/reina0721/dsh-docker/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/reina0721/dsh-docker?sort=semver)](https://github.com/reina0721/dsh-docker/releases)
[![GHCR](https://img.shields.io/badge/ghcr.io-reina0721%2Fdsh--docker-blue?logo=github)](https://github.com/reina0721/dsh-docker/pkgs/container/dsh-docker)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

DeepSeek Harness（DSH）的 Docker 化长期开发环境。v2 使用 Rust 重写了宿主 CLI 与容器网关：

- **`dshctl`**：单个静态二进制，负责初始化、部署、更新、诊断、认证、备份、恢复、自更新。
- **`dsh-entrypoint`**：容器内 Rust 入口 + 反向代理，替换 Caddy 和 Node 代理。
- **单容器**：DSH 只监听容器回环，Rust 网关对外提供 Basic Auth、Host/Origin 校验、HTML 兼容注入、WebSocket/SSE 透传。
- **无 shell 业务脚本**：不再需要修复可执行位，Windows/Linux/macOS 使用同一个 CLI。

> v2 是破坏性大版本。旧仓库的 `./scripts/*.sh` 不再提供；请阅读 [v1 → v2 迁移](docs/migration-v1-to-v2.md)。

## 架构

```text
Browser
  │  HTTPS（可选，由外部 Nginx/Caddy/Traefik 终止）
  ▼
127.0.0.1:3080 ── dsh 容器 ──────────────────────────────
                   dsh-entrypoint (Rust, via tini)
                     ├─ Rust Gateway :3080
                     │    ├─ Basic Auth（users.htpasswd，bcrypt 缓存/热加载）
                     │    ├─ Host/Origin 校验与内部化
                     │    ├─ HTML compat 注入 / WebSocket / SSE
                     │    └─ 502/503、结构化日志、健康检查
                     └─ dsh web --host 127.0.0.1 --port 3079
                          └─ /workspace, /home/node/.dsh, /home/node/.persist
```

## 快速开始

### 一条命令（推荐）

Linux / macOS 交互式安装：

```bash
curl -fsSL https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.sh | bash
```

中国大陆网络可加 `--mirror cn`（脚本与镜像都会优先使用国内加速）：

```bash
curl -fsSL https://ghfast.top/https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.sh | bash -s -- --mirror cn
```

Windows（PowerShell 5.1+，需要 Docker Desktop / WSL2）：

```powershell
irm https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.ps1 | iex
```

安装脚本提供友好、可配置的**交互式向导**，引导您设置：
- **项目安装目录**（默认 `~/dsh-docker`，所有项目程序、编排文件与数据均集中存放在该目录下）
- **外部访问端口**（默认 `3080`）与**监听地址**（`127.0.0.1` 本机反代 或 `0.0.0.0` 局域网/公网）
- **开发环境变体**（`dev` 完整环境或 `slim` 极简版）
- **网络加速策略**（官方默认源或中国大陆镜像加速）
- **管理登录密码**（自定义输入或自动生成高强度安全随机密码）

### 非交互式快捷安装（自动化 / 脚本部署）

如果您需要在自动化脚本、云服务器初始化或 CI/CD 中一键完成配置与安装，可通过参数指定配置并跳过交互确认：

```bash
curl -fsSL https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.sh | bash -s -- \
  --dir /workapps/test/dsh-docker \
  --port 3080 \
  --bind 0.0.0.0 \
  --variant dev \
  --admin qwq \
  --password 123456 \
  --weak-password \
  --yes
```

> **国内服务器加速安装**：可将命令开头的 URL 替换为加速镜像地址：`curl -fsSL https://ghfast.top/https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.sh | bash -s -- ...`，并在参数中添加 `--mirror cn`。

#### 参数使用说明

| 参数 | 默认值 | 说明 |
|---|---|---|
| `--dir <PATH>` | `~/dsh-docker` | **项目安装根目录**。所有程序（`bin/dshctl`）、编排（`compose.yaml`）、配置（`.env`）与持久化运行时数据（`.runtime/`）均独立集中存放在该目录下，严格杜绝向系统外部散落文件。 |
| `--port <PORT>` | `3080` | **宿主机 Web 访问端口映射**。DSH Web 服务对外暴露的端口。 |
| `--bind <IP>` | `127.0.0.1` | **监听绑定地址**。设为 `127.0.0.1` 仅限本机回环访问（适合搭配外部 Nginx/Caddy 等前端反代终止 TLS）；设为 `0.0.0.0` 允许局域网或公网直接访问。 |
| `--variant <dev\|slim>` | `dev` | **开发环境变体**。<br>• `dev`：完整开发环境，包含 Node、DSH、Go、Rust、pnpm、Python 等常用开发语言与工具。<br>• `slim`：轻量精简环境，仅包含 Node、DSH、Git、SSH 等基础依赖，后续可根据需要按需安装扩展。 |
| `--admin <USER>` | `admin` | **管理员 Basic Auth 用户名**。登录 Web 界面所使用的账号名称。 |
| `--password <PASS>` | 自动生成 | **管理员登录访问密码**。若未指定，脚本将自动生成 16 位高强度安全随机密码。 |
| `--weak-password` | 强密码模式 | **允许弱密码模式**。快捷选项，允许设置 6 位或以上简短密码（如 `123456`），并在配置中自动设定 `AUTH_MIN_PASSWORD_LENGTH=6`，适合测试/内网环境（默认强密码模式要求至少 12 位密码）。 |
| `--password-mode <strong\|weak>` | `strong` | **密码安全模式**。显式选择 `strong`（>=12 位）或 `weak`（>=6 位）。 |
| `--min-password-length <N>` | 12 (强) / 6 (弱) | **自定义最小密码长度**。 |
| `--dsh-version <VER>` | npm 最新版 | **指定 DSH 版本**。安装时默认自动查询 npm 官方最新发布版本并锁定，避免命中本地 Docker 缓存导致安装旧版。 |
| `--yes`, `-y` | 交互向导 | **非交互模式确认**。对未指定的项自动采用默认值，跳过终端交互提示并立即开始下载、初始化与 Docker 本地部署启动。 |
| `--mirror <global\|cn\|off>` | `global` | **网络加速策略**。`cn` 模式会自动启用国内 GitHub 代理、NPM 镜像源及 Go 代理加速。 |
| `--no-deploy` | 立即部署 | **仅初始化**。仅下载解压并完成目录与用户初始化，不立即启动 Docker 容器构建与运行。 |
| `--system-link` | 否 | **全局软链接**。可选在 `/usr/local/bin/dshctl` 创建系统全局软链接（默认不创建，保证项目完全独立自包含）。 |


产物包内置了静态编译好的 `dsh-entrypoint` 容器网关，用户端通过 Docker 构建时**无需下载庞大的 Rust 编译镜像，也无需耗时编译**，1~2 分钟内即可极速完成构建与部署。

部署后直接打开 `http://127.0.0.1:3080/`（或您指定的端口），用 Basic Auth 登录即可。网关会自动捕获 DSH 的
单次 web token（日志中脱敏），并在 DSH 设置认证 Cookie 前注入，不需要手动复制
`?token=...` 地址。

### 从源码

需要 Rust 1.85+、Docker Engine 24+、Docker Compose v2：

```bash
git clone https://github.com/reina0721/dsh-docker.git
cd dsh-docker
cargo build --release -p dshctl
./target/release/dshctl deploy
```

国内克隆/拉取加速：

```bash
git clone https://ghfast.top/https://github.com/reina0721/dsh-docker.git
# 或
git clone https://gh-proxy.com/https://github.com/reina0721/dsh-docker.git
```

## 镜像

默认：

```dotenv
DSH_IMAGE_MODE=auto
DSH_IMAGE=ghcr.io/reina0721/dsh-docker:2
```

- `:2` / `:latest`：`dev` 镜像，包含 Go、Rust、Node、pnpm、DSH 与常用开发工具。
- `:2-slim`：`slim` 镜像，保留 Node、DSH、Git/SSH、构建基础工具，不包含 Go/Rust 工具链。

`auto` 模式按 `pull → 本地兼容镜像 → 本地构建` 回退，并实时显示拉取进度。镜像必须带有
`io.github.reina0721.dsh-docker.runtime=2` 标签，避免误用 v1 缓存。

### 国内镜像

```bash
dshctl deploy --mirror cn
```

或写入 `.env`：

```dotenv
DSH_IMAGE_MIRROR=cn
DSH_IMAGE_MIRRORS=ghcr.nju.edu.cn,ghcr.m.daocloud.io
```

镜像只作为传输层；拉取后会 tag 回 canonical 名称并校验 v2 标签。第三方镜像可用性不由本项目保证。

## 常用命令

| 命令 | 说明 |
| --- | --- |
| `dshctl init` | 创建 `.env`、缺失的 `compose.yaml`、`.runtime/` 与权限 |
| `dshctl deploy` | 预检、认证、拉取/构建镜像、启动、健康检查 |
| `dshctl up` / `down` / `stop` / `restart` | 生命周期管理 |
| `dshctl status [--json]` | 服务、HTTP、认证、镜像状态 |
| `dshctl doctor [--fix] [--network] [--json]` | 诊断 Docker、端口、配置、权限、认证、镜像与网络 |
| `dshctl logs -f` / `dshctl shell` / `dshctl exec -- cmd` | 日志/终端/容器内命令 |
| `dshctl user add <name>` | 创建或修改密码（bcrypt，本地生成，无需重启，支持 `--insecure-password`） |
| `dshctl user list` / `dshctl user remove <name>` | 查看/删除用户 |
| `dshctl ssh list/generate/import/remove/test` | 管理持久化 SSH Key（不打印私钥） |
| `dshctl git setup --name ... --email ... [--key ...]` | 配置容器内 Git 身份/默认 Key |
| `dshctl backup [path] [--quiesce]` | 备份 `.env` + `.runtime`（0600） |
| `dshctl restore <archive> --yes` | 校验并恢复；失败回滚 |
| `dshctl upgrade [--check] [--rebuild]` | 检查并升级 DSH (@deepseek-ai/dsh) 到官方最新版本 |
| `dshctl update [--source] [--backup]` | 拉取最新镜像并重建 |
| `dshctl rebuild` | 强制本地构建 |
| `dshctl image check` / `dshctl image pull` | 镜像/网络诊断与拉取 |
| `dshctl config validate` / `dshctl config show` | 配置校验/脱敏展示 |
| `dshctl migrate [<DIR>] [--to <DIR>]` | 将项目及运行时完整迁移至指定生产目录（或 v1 迁移） |
| `dshctl account [status/login/where]` | 查看 DeepSeek 官网账户登录状态与 SSH 端口转发登录指南 |
| `dshctl self-update` | 从 GitHub Releases 更新自身 |
| `dshctl completion bash` | 生成补全脚本 |

完整说明见 [CLI 文档](docs/cli.md) 与 [配置参考](docs/configuration.md)。

## 持久化

默认所有长期状态都在仓库的 `.runtime/`：

```text
.runtime/
├── workspace/                -> /workspace
├── dsh-home/                 -> /home/node/.dsh
├── home/                     -> /home/node/.persist
│   ├── ssh/                  -> ~/.ssh
│   ├── gnupg/                -> ~/.gnupg
│   ├── aws/                  -> ~/.aws
│   ├── kube/                 -> ~/.kube
│   ├── docker/               -> ~/.docker
│   ├── config/               -> ~/.config
│   ├── local/share/          -> ~/.local/share
│   ├── local/state/          -> ~/.local/state
│   ├── git/config            -> ~/.gitconfig
│   ├── git/credentials       -> ~/.git-credentials
│   ├── shell/bash_history    -> ~/.bash_history
│   ├── shell/python_history  -> ~/.python_history
│   ├── npm/npmrc             -> ~/.npmrc
│   ├── cargo/config.toml     -> ~/.cargo/config.toml
│   ├── cargo/credentials.toml-> ~/.cargo/credentials.toml
│   └── netrc / pypirc
└── auth/users.htpasswd       -> Web Basic Auth
```

不映射整个 `/home/node`，因为镜像里的 Go/Rust/npm/DSH 工具链有一部分位于 Home 下。容器启动时只把需要长期保存的路径链接到 `/home/node/.persist`。

## 认证

```bash
dshctl user add alice
dshctl user list
dshctl user remove alice
```

- 默认 bcrypt cost `14`，可用 `AUTH_BCRYPT_COST` 调整；小 VPS 可考虑 `12`。
- 新密码默认至少 12 字符，可用 `AUTH_MIN_PASSWORD_LENGTH` 调整。
- 明文密码不写入磁盘、不出现在进程参数中；支持 `--password-stdin` / `--password-file`。
- 网关缓存成功凭证并在文件变更后热加载；新增/删除用户无需重启容器。
- 删除最后一个用户后网关 fail closed（503），不会在无认证状态下继续放行。

## SSH / Git 身份

GitHub/GitLab 的 SSH Key 持久化在 `.runtime/home/ssh`，Git 配置在
`.runtime/home/git/config`。推荐用 CLI 管理：

```bash
# 生成（交互式输入 passphrase）或使用 --no-passphrase
dshctl ssh generate work --type ed25519 --comment "you@example.com"

# 导入宿主机已有 Key
dshctl ssh import ~/.ssh/id_ed25519 --name github

# 查看 / 测试 / 删除（只显示公钥和 fingerprint）
dshctl ssh list
dshctl ssh test github --key work
dshctl ssh remove work

# 配置 Git 用户与默认 Key
dshctl git setup --name "Your Name" --email "you@example.com" --key work
dshctl git show
```

把 `dshctl ssh list` 显示的公钥添加到 GitHub → Settings → SSH and GPG keys，然后
`dshctl ssh test github`。私钥不会写入日志、镜像或 Git；`dshctl backup` 会包含
`.runtime/home/ssh`，请按敏感文件保管。详见 [SSH/Git 文档](docs/ssh-git.md)。

## 反向代理

Core 默认只监听 `127.0.0.1:3080`，TLS 由你现有的 Nginx/Caddy/1Panel/Traefik 终止。示例：

- [Nginx](examples/nginx.conf)
- [Caddy](examples/Caddyfile)
- [Traefik](examples/traefik.yaml)
- [1Panel](examples/1panel.md)

关键要求：保留原始 `Host` 与 `Origin`；WebSocket 需要 Upgrade；SSE 关闭 buffering。

## 安全

- Basic Auth 只应在 HTTPS 后面使用；不要直接把 3080 暴露到公网明文 HTTP。
- 备份包含 SSH 私钥、Git/云凭据与 DSH 凭据，必须按敏感文件保管。
- 镜像从 GHCR 拉取时建议校验 digest/cosign；第三方镜像只作为传输层。
- 详细威胁模型与建议见 [安全说明](docs/security.md)。

## 更新与迁移

```bash
# 一键升级 DSH (@deepseek-ai/dsh) 到官方最新版本（自动检查 npm 仓库发布）
./dshctl upgrade
./dshctl upgrade --check              # 仅检查是否有新版本
./dshctl upgrade --version 0.2.1-rc.5 # 升级到指定版本
./dshctl upgrade --rebuild            # 重新构建镜像层并应用升级

# 更新镜像并重建（不删除 .runtime）
./dshctl update --backup

# 将测试安装目录及全部配置与持久化数据平滑迁移至正式生产目录（如 /workapps/dsh-docker）
./dshctl migrate --to /workapps/dsh-docker
# 或直接简写目标路径：
./dshctl migrate /workapps/dsh-docker

# 更新 dshctl CLI 自身到 GitHub 最新 Release
./dshctl self-update

# v1 数据迁移（users.caddy -> users.htpasswd，提示 :latest -> :2）
dshctl migrate --dry-run
dshctl migrate
dshctl deploy
```

### 登录 DeepSeek 官网账户

DeepSeek 官方平台（`platform.deepseek.com`）OAuth 登录规范限定授权回调仅信任本地回环地址（`http://localhost:<端口>/oauth/callback`）。

当 DSH 部署在远程 Linux 服务器时，登录官网账户的推荐方式为：
1. **建立本地 SSH 隧道端口转发**（在您的本地电脑终端执行）：
   ```bash
   ssh -N -L 3080:127.0.0.1:3080 <SSH用户名>@<服务器地址> -p <SSH端口>
   ```
2. **在本地浏览器打开** `http://localhost:3080/` 并输入 Basic Auth 管理员密码登录。
3. **进入 Web UI**，点击右上角「设置 (Settings) -> 账号 (Account) -> 登录 (Sign in)」。
4. 浏览器将弹出 DeepSeek 官方登录窗口，完成登录后会自动回调至 `http://localhost:3080/oauth/callback`。
5. 网关与 DSH 自动放行该 OAuth 回调端点并完成 PKCE 换票，凭据将永久保存在 `.runtime/home/dsh/.credentials.yaml` 中，重启与升级均不受影响。

回滚：`git checkout <v1 tag>` 后使用旧 Compose/镜像启动；`.runtime` 不会被新版本删除。详见 [迁移指南](docs/migration-v1-to-v2.md)。

## 开发

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
docker build --target runtime -t dsh-docker:slim .
docker build --target dev -t dsh-docker:dev .
```

无 Docker 时仍可运行配置/认证/备份测试。网关集成测试在 `crates/dsh-gateway/tests/proxy.rs`。

## 文档

- [架构](docs/architecture.md)
- [配置](docs/configuration.md)
- [CLI](docs/cli.md)
- [SSH / Git](docs/ssh-git.md)
- [安全](docs/security.md)
- [故障排查](docs/troubleshooting.md)
- [常见问题](docs/faq.md)
- [开发](docs/development.md)
- [发布流程](docs/release.md)
- [v1 → v2 迁移](docs/migration-v1-to-v2.md)
- [文档索引](docs/README.md)

## 直链与国内访问

| 内容 | 直链 |
| --- | --- |
| 仓库 | `https://github.com/reina0721/dsh-docker` |
| 安装脚本 | `https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.sh` |
| 安装脚本（加速） | `https://ghfast.top/https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.sh` |
| 文档镜像 | `https://cdn.jsdelivr.net/gh/reina0721/dsh-docker@main/README.md` |
| Releases | `https://github.com/reina0721/dsh-docker/releases/latest` |
| GHCR | `ghcr.io/reina0721/dsh-docker:2` |
| GHCR 镜像示例 | `ghcr.nju.edu.cn/reina0721/dsh-docker:2`、`ghcr.m.daocloud.io/reina0721/dsh-docker:2` |

第三方加速地址仅用于传输，可能会变更；请以 SHA256/cosign 校验和官方 GitHub 为信任根。

## 许可证

MIT OR Apache-2.0，见 [LICENSE-MIT](LICENSE-MIT) 与 [LICENSE-APACHE](LICENSE-APACHE)。
