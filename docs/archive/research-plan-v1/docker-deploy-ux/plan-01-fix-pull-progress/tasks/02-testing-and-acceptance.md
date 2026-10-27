# Tasks / 02-testing-and-acceptance.md

## 1. 静态校验（必做）

```bash
bash -n scripts/lib.sh
shellcheck scripts/lib.sh
bash scripts/self-test.sh
```

- `self-test.sh` 中的新增回归项必须为 `[OK]`：`lib.sh` 中不允许 `output="$(docker pull`。
- 若实现 T3 诊断脚本：
  ```bash
  bash -n scripts/diagnose-pull.sh
  shellcheck scripts/diagnose-pull.sh
  ```

## 2. 功能验收（按验收标准 6.1–6.4）

### 2.1 进度实时显示
- 环境：`DSH_IMAGE_MODE=auto`，本地无 `ghcr.io/rin721/dsh-docker:latest`。
- 操作：`./scripts/deploy.sh`（或仅触发 `prepare_dsh_image`）。
- 预期：出现「尝试拉取预构建 DSH 镜像」后，**立即**看到 `Pulling from ...`、
  `Pulling fs layer`、`Downloading ...` 等持续更新的进度输出；不再静止无输出。

### 2.2 fallback 三段回退不回归
- 分支 1（prebuilt）：正常网络 → `DSH_DELIVERY=prebuilt`，「使用预构建镜像，跳过本地编译」。
- 分支 2（local）：`docker pull` 失败但本机已有该镜像 → 打印「复用本机已有 DSH 镜像」，
  `DSH_DELIVERY=local`。
- 分支 3（build）：两者皆无 → 打印「自动回退本地构建」，进入 `docker compose build dsh`。
- 验证小技巧：临时改 `DSH_IMAGE` 指向一个不存在的 tag 制造 pull 失败，再用 `docker
  image tag` 造一个本地镜像，观察分支 2；去掉本地镜像观察分支 3。

### 2.3 网络分诊命令可用
- `scripts/diagnose-pull.sh`（若实现）：
  - 任一步骤不得让脚本挂起（均有超时）。
  - `curl -I https://ghcr.io/v2/` 返回 `HTTP/2 401` → 必须判为 `OK/正常`。
  - 能对「manifest 快 + pull 卡」给出「Layer CDN / pkg-containers...
     问题」的明确结论。

### 2.4 文档一致性
- `README.md` 中描述的部署行为与实际一致；新增故障排查命令均真实可执行。

## 3. 回归范围

- 不涉及 `compose.yaml` / `Dockerfile` / Workflow / 持久化，无需专项回归，但应跑一次
  `docker compose config >/dev/null` 确认 compose 仍有效。

## 4. 发布前检查

- [ ] `bash scripts/self-test.sh` 全绿（含新增回归项）。
- [ ] `shellcheck` 通过（改动涉及的脚本）。
- [ ] 手动验证 2.1 与 2.2 的至少两种情况。
- [ ] `git status` 仅包含预期文件。
- [ ] 按 Conventional Commits 提交（如 `fix(deploy): restore live docker pull progress`）。

## 5. 结论模板

```text
[ ] 2.1 进度实时显示      : 通过 / 不通过（说明）
[ ] 2.2 pull→local→build  : 通过 / 不通过
[ ] 2.3 诊断命令          : 通过 / 跳过
[ ] 2.4 文档一致性        : 通过
[ ] self-test / shellcheck: 通过
是否可发布：是 / 否
```
