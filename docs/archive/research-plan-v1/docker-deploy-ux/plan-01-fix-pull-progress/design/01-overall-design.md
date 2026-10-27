# Design / 01-overall-design.md

## 1. 设计目标回顾

- 让 `docker pull` 在部署时**实时输出进度**（解决「假卡死」）。
- 保持 `auto` 模式下的三段回退：`prebuilt → local → build`。
- 为「网络问题导致拉取缓慢/卡住」提供可执行的分诊手段。

## 2. 解决思路

### 2.1 核心改动（必做）

把 `resolve_dsh_delivery` 中 `auto` 分支的：

```bash
if output="$(docker pull "${image}" 2>&1)"; then
    printf '%s\n' "${output}"
    DSH_DELIVERY=prebuilt
```

改为「不捕获输出」的直连形式：

```bash
if docker pull "${image}"; then
    DSH_DELIVERY=prebuilt
elif docker image inspect "${image}" >/dev/null 2>&1; then
    echo "预构建镜像当前无法拉取；复用本机已有 DSH 镜像。" >&2
    echo "仓库内的启动/动态代理文件会通过只读 bind mount 覆盖镜像内副本。" >&2
    DSH_DELIVERY=local
else
    echo "预构建镜像不可用且本机无可复用镜像，自动回退本地构建。" >&2
    DSH_DELIVERY=build
fi
```

本质只有一处变化：

```diff
- if output="$(docker pull "${image}" 2>&1)"; then
-     printf '%s\n' "${output}"
+ if docker pull "${image}"; then
```

由此，Docker 的 layer 下载进度会像 `docker compose pull gateway` 一样实时显示。

## 3. 方案对比

| 方案 | 说明 | 取舍 |
| --- | --- | --- |
| A. 直接去捕获（`if docker pull ...`） | 详见 2.1，最小改动 | ✅ 首选：改动小、语义不变、实时输出 |
| B. 捕获但 `tee` 到 stderr | `docker pull ... 2>&1 \| tee >(cat >&2)` | 复杂、TTY 下进度仍可能被管道缓冲，得不偿失 |
| C. 加 `--progress=plain` | 无 TTY 时更利于脚本化 | 可作**可选项**，不做默认强制（有 TTY 时 `plain` 反而丢失动画） |

结论：本轮采用方案 A；方案 C 作为可选项通过环境变量/开关提供。

## 4. 诊断能力设计（增强，可选但推荐）

用于回答「到底是进度被吞，还是网络真有问题」。新增一个轻量脚本
`scripts/check-image.sh`（仓库中已存在同名脚本，需先查看再决定是扩展还是新增）：

> 说明：`scripts/check-image.sh` 已存在，实施前必须 `read` 其内容，按其现有职责决定
> 是增强还是另建 `scripts/diagnose-pull.sh`，避免覆盖已有功能。

分诊步骤（与 `tmp/a.md` 第 4–6 节一致）：

1. **直连拉取**：先 `docker pull ${image}` 看是否立即出现 `Pulling fs layer`。
   - 出现 → 原始问题确系进度被吞；本修复后即可。
   - 不出现 → 转入网络分诊。
2. **DNS/TCP/TLS 验证**：
   ```bash
   getent hosts ghcr.io
   getent hosts pkg-containers.githubusercontent.com
   curl -I --connect-timeout 10 https://ghcr.io/v2/
   ```
   - `HTTP/2 401` 是**正常**（表示 DNS/TCP/TLS 通，Registry 要求匿名 token 前先 401）。
   - `curl` 长时间无响应 → 网络不通。
3. **Registry 与 Layer CDN 分离测试**：
   ```bash
   time docker manifest inspect ghcr.io/rin721/dsh-docker:latest >/dev/null
   docker pull ghcr.io/rin721/dsh-docker:latest
   ```
   - manifest 快速完成、`Downloading/Waiting` 卡住 → 指向 Layer CDN
     `pkg-containers.githubusercontent.com` 或出口带宽。

## 5. 总体架构影响

- 改动集中在一个函数 `resolve_dsh_delivery` 内，不影响 `prepare_dsh_image` 其它分支。
- `deploy.sh`、`compose.yaml`、`Dockerfile`、Workflow **无需改动**。
- 文档（README、故障排查）与诊断脚本属于附带交付。

## 6. 决策记录（ADR 摘要）

- ADR-01：去捕获 vs tee/plain —— 选最小改动去捕获。
- ADR-02：fallback 三段逻辑保留 —— 已有逻辑正确，行为不得回归。
- ADR-03：镜像瘦身列为后续，不进入本计划范围。
