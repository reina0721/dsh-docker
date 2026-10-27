# Tasks / 01-implementation-plan.md

> 本计划聚焦**文档化落地路径**；如需实际改代码，可按下述顺序执行并在每个阶段验收。

## 0. 前置说明

当前任务目标是「分析 `tmp/a.md` 解决方案并产出规划文档」。若后续要真正修代码，
以下为推荐实施顺序（与设计文档 D1–D4 对应）。

## 1. 任务分解

### T0. 复核现状（已完成）
- [x] 阅读 `tmp/a.md`，提取解决方案。
- [x] 核对 `scripts/lib.sh`（`resolve_dsh_delivery`）确为进度被吞的元凶。
- [x] 核对 `deploy.sh`、`compose.yaml`、`Dockerfile`、`publish-image.yml`。

### T1. 实施核心修复（对应 D1）✅
- [x] Read `scripts/lib.sh`，定位 `resolve_dsh_delivery` 的 `auto` 分支。
- [x] 将 `if output="$(docker pull ...)"` + `printf` 替换为 `if docker pull ...`，保留 `local` / `build` fallback 分支。
- [x] `bash -n scripts/lib.sh` 通过；`shellcheck scripts/lib.sh` 通过。
- [x] 确认 `export DSH_DELIVERY` 仍保留、`prepare_dsh_image` 各分支行为不变。

### T2. 防回归校验（对应 D2）✅
- [x] Read `scripts/self-test.sh`。
- [x] 新增 grep 校验：`lib.sh` 不得再出现 `output="$(docker pull` / `output="$(docker compose pull` 模式。
- [x] `bash scripts/self-test.sh` 全绿。

### T3. 诊断能力（对应 D3，推荐）✅
- [x] Read `scripts/check-image.sh`，决定增强（直接在 `check-image.sh` 内增强，不新建）。
- [x] 实现 manifest / GHCR Registry（`curl -I ghcr.io/v2/`）/ Layer CDN DNS 分诊步骤（全部带超时）。
- [x] 保证 `401` 判为「正常」，脚本自身不挂死。
- [x] `bash -n` + `shellcheck`.

### T4. 文档更新（对应 D4）✅
- [x] `README.md` 部署章节说明实时进度。
- [x] `README.md` 新增「镜像拉取与网络排查」小节。
- [x] `README.md`「状态与诊断」加入 `check-image.sh`。

### T5. 阅读并提交（按 git-conventional-commit 规范）✅
- [x] 复跑 `bash scripts/self-test.sh`（`bash -n` 全通过；运行时项因无 docker/Windows 环境跳过）。
- [x] 仅暂存预期文件，`git add` + Conventional Commits 提交（`188bb85`）。

## 2. 依赖与顺序

```text
T1 ──> T2（依赖 T1 的修改）
T1 ──> T3（独立，可并行）
T1 ──> T4（独立，文档）
T5：收尾
```

## 3. 回调/验收入口

每完成一个 T 阶段，对照《tasks/02-testing-and-acceptance.md》的对应检查项逐条打勾。

## 4. 风险清单

- `set -Eeuo pipefail`：`if docker pull ...` 失败不应导致脚本提前退出（`if` 上下文已安全）。
- fallback 回归：改后必须验证 `local` 与 `build` 分支可达。
- 误改 `check-image.sh`：先读其现职责再决定增强或新建，避免破坏既有命令。
