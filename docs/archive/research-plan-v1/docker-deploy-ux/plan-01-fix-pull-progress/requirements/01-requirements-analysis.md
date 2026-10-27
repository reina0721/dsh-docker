# Requirements / 01-requirements-analysis.md

## 1. 背景与问题定义

用户反馈：运行 `./scripts/deploy.sh` 后，部署流程停在：

```text
尝试拉取预构建 DSH 镜像：ghcr.io/rin721/dsh-docker:latest
```

然后长时间无任何输出，容易被误判为「程序死锁」。

经核对 `tmp/a.md` 解决方案并对本仓库源码复核，**根因并非 Docker 卡死，而是脚本把
`docker pull` 的实时进度全部吞掉了**，形成「假卡死」。

## 2. 现状（已核实）

`scripts/lib.sh` 中 `resolve_dsh_delivery()` 的 `auto` 模式（第 234–247 行）：

```bash
auto)
    echo "尝试拉取预构建 DSH 镜像：${image}"
    if output="$(docker pull "${image}" 2>&1)"; then
        printf '%s\n' "${output}"
        DSH_DELIVERY=prebuilt
    elif docker image inspect "${image}" >/dev/null 2>&1; then
        ...
        DSH_DELIVERY=local
    else
        ...
        DSH_DELIVERY=build
    fi
    ;;
```

问题点：

- `output="$(docker pull "${image}" 2>&1)"`：命令替换会捕获 docker pull 的 stdout/stderr
  到变量，**在拉取完成前不产生任何输出**。
- 只有拉取结束（成功或失败）后，才 `printf '%s\n' "${output}"` 一次性输出全部内容。
- 于是 Docker 本应展示的 `Pulling fs layer` / `Downloading 123.4MB/800MB` 等进度在一
  段时间内完全不可见。

而 `deploy.sh` 中另一个拉取（Gateway）用的是 `docker compose pull gateway`，其输出
是实时显示的，进一步对比出「Gateway 正常、DSH 无输出」的强烈落差。

### 2.1 相关现状（背景，本轮一般不改）

- `Dockerfile`：基础镜像为 `node:${NODE_VERSION}-bookworm-slim`，并 COPY Go toolchain、
  安装 build-essential/make/cmake/clang/python3/Rust/Node 等大量工具，**镜像本身较重**。
- `.github/workflows/publish-image.yml`：`platforms: linux/amd64,linux/arm64`，multi-arch。
  - 注意：amd64/arm64 主机只会拉取对应架构，**不会**因为 multi-arch 而下载量翻倍。
- `compose.yaml`：`image: "${DSH_IMAGE:-ghcr.io/rin721/dsh-docker:latest}"`，默认镜像即
  GHCR 预构建镜像。

## 3. 目标

1. **修复假卡死**：`auto` 模式下 `docker pull` 的进度实时可见，不再被变量捕获吞掉。
2. **保持既有 fallback 语义**：拉取失败 → 复用本机已有镜像（local）→ 仍无则本地构建
   （build）的三段回退逻辑必须保留。
3. **提高可诊断性**：当拉取确实很慢/卡住时，能方便地区分「进度被吞（已修复）」与
   「到 GHCR / Layer CDN 的网络问题」。
4. 兼容 `set -Eeuo pipefail`，不破坏其他部署脚本。

## 4. 非目标（明确不做）

- 本轮不拆解/瘦身 Docker 镜像（列为后续优化主题，不阻塞本计划）。
- 不改变 DSH 容器运行时、代理、持久化、认证等逻辑。
- 不改动 `pull` / `build` 两个模式的核心语义（`pull` 已是直接 `docker pull`）。

## 5. 用户与场景

- 主要使用者：通过 `./scripts/deploy.sh` 首次或更新部署本仓库的开发者/运维。
- 场景 A：正常网络——期望看到实时下载进度，部署流畅。
- 场景 B：GHCR 不可达或 Layer CDN 卡住——期望能快速定位到网络层问题而非误判死锁。

## 6. 验收标准（可量化）

1. `DSH_IMAGE_MODE=auto` 下运行 `deploy.sh`，在拉取大镜像期间能看到逐步增长的下载
   输出（`Downloading ...`），而非静止无输出。
2. 移除本地镜像后，`deploy.sh` 仍能按 `pull → local → build` 顺序正确回退（通过
   模拟/断网验证日志分支）。
3. `bash -n scripts/lib.sh`、`shellcheck scripts/lib.sh` 通过；`bash scripts/self-test.sh`
   通过（含新增校验）。
4. 新增的诊断命令能对「Registry 可达但 Layer CDN 不可达」给出明确提示。
