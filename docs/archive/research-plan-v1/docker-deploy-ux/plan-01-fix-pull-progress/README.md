# Plan-01：修复 DSH 镜像 `docker pull` 进度被吞导致的「假卡死」部署体验问题

> 检索自 `tmp/a.md`（GPT 提供的解决方案），并结合本仓库实际代码核对后的落地计划。

## 一、一句话结论

部署脚本 `scripts/lib.sh` 的 `auto` 模式下用 `output="$(docker pull ...)"` 捕获了
`docker pull` 的全部 stdout/stderr，导致下载进度被缓存到变量里、只在拉取完全结束后才
一次性输出。用户看到「尝试拉取预构建 DSH 镜像：...」后就长时间没有任何输出，极容易
被误判为程序死锁 —— 这是**最主要的问题**。

## 二、文档导航

| 文档 | 说明 |
| --- | --- |
| [requirements/01-requirements-analysis.md](requirements/01-requirements-analysis.md) | 问题背景、现状分析、目标、非目标、验收标准 |
| [design/01-overall-design.md](design/01-overall-design.md) | 解决思路、方案对比、总体架构调整 |
| [design/02-detailed-design.md](design/02-detailed-design.md) | 具体改动点（脚本、诊断、文档）与伪代码/示例 |
| [tasks/01-implementation-plan.md](tasks/01-implementation-plan.md) | 分步实施清单、责任人、顺序、依赖 |
| [tasks/02-testing-and-acceptance.md](tasks/02-testing-and-acceptance.md) | 自测、手动验收、回归与发布检查 |

## 三、范围与交付物

- **范围**：仅涉及 Docker 镜像获取（pull）阶段的**部署 UX** 与**诊断能力**改进，
  不改动 DSH 容器本身的运行时逻辑、代理逻辑与持久化机制。
- **交付物**：
  1. `scripts/lib.sh` 中 `resolve_dsh_delivery` 的 `auto` 模式去捕获（实时输出）。
  2. （可选增强）新增一个网络/镜像拉取诊断能力或脚本，区分「进度被吞」与「到 GHCR
     的网络问题」。
  3. 文档更新：README 部署/故障排查章节。
  4. 测试：扩展 `scripts/self-test.sh` 校验逻辑。

## 四、核心决策摘要

| 决策点 | 结论 |
| --- | --- |
| 是否保留 `output="$(...)"` 捕获 | 否——改为直接 `if docker pull ...`，实时展示进度 |
| 是否保留 fallback（本地镜像复用 / 本地构建） | 是——已有逻辑正确，改动后依然生效 |
| `pull` 模式是否要改 | 已是直接 `docker pull`，无需改动 |
| 是否本轮就大改镜像瘦身 | 否——列为后续/非目标，不阻塞本计划 |

## 五、关联文件

- `scripts/lib.sh`（`resolve_dsh_delivery`，第 221–254 行）
- `scripts/deploy.sh`（调用 `prepare_dsh_image`）
- `scripts/self-test.sh` / `scripts/doctor.sh`（测试与诊断）
- `README.md`（部署 / 故障排查文档）
- `Dockerfile`、`compose.yaml`、`.github/workflows/publish-image.yml`（背景，不改动或仅诊断参考）

## 六、风险与注意

- `set -euo pipefail` 下需确保 `if docker pull ...` 的失败路径仍走 fallback，不提前退出。
- `docker pull` 在部分终端环境（无 TTY）下仍可能输出有限；本方案以「不吞输出」为
  首要目标，是否开启 `--progress=plain` 留作可选项。
- 改动前后需跑 `bash scripts/self-test.sh` 与 `shellcheck scripts/lib.sh`。
