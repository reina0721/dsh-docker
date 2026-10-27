# Design / 02-detailed-design.md

## 1. 改动点总览

| # | 文件 | 改动 | 必须/可选 |
| --- | --- | --- | --- |
| D1 | `scripts/lib.sh` `resolve_dsh_delivery`（auto 分支） | 去捕获，实时输出 | 必须 |
| D2 | `scripts/self-test.sh` | 新增：禁止 auto 分支出现 `output="$(docker pull` 模式 | 必须（防回归） |
| D3 | 诊断脚本（`scripts/check-image.sh` 或新增 `scripts/diagnose-pull.sh`） | 网络分诊 | 可选推荐 |
| D4 | `README.md` + 各 `--help`/脚本注释 | 部署 & 故障排查说明 | 可选推荐 |

## 2. D1：`scripts/lib.sh` 详细改动

### 2.1 目标代码（`auto` 分支完整替换）

```bash
auto)
    echo "尝试拉取预构建 DSH 镜像：${image}"

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
    ;;
```

### 2.2 正确性论证

- `if docker pull ...`：命令成功（退出码 0）→ `prebuilt`；失败 → 进入 fallback。
- 不再有 `output="$(...)"`，因此 `docker pull` 的 stdout/stderr 直通终端，**实时可见**。
- `elif docker image inspect ...` 与 `else` 分支：行为与现状一致，fallback 语义不变。
- 兼容性：函数仍以 `export DSH_DELIVERY` 结束（函数末尾已有 `export DSH_DELIVERY`，
  保持不动）。

### 2.3 关于可选项 `--progress=plain`

- 若在无 TTY 的 CI/守护进程里使用，可考虑 `docker pull --progress=plain "${image}"`。
- 实现：不写死在脚本里，遵循 `docker` 自身的环境/配置；若需提供，可用
  `DSH_DOCKER_PULL_EXTRA` 之类的变量透传，**但本轮保持最小改动，默认不引入**。

## 3. D2：`scripts/self-test.sh` 防回归校验

新增（例如放在现有 Dockerfile/compose 校验之后）：

```bash
if grep -Eq 'output="\$\(docker (pull|compose pull)' "${ROOT_DIR}/scripts/lib.sh"; then
    bad "lib.sh: docker pull 输出不得被命令替换吞掉（需实时进度）"
else
    ok "lib.sh: docker pull 输出不被捕获"
fi
```

意图：防止未来再次把实时进度捕获进变量、重蹈「假卡死」。

## 4. D3：诊断脚本（推荐）设计

> 先 `read scripts/check-image.sh`，按其定位决策：增强 or 新建
> `scripts/diagnose-pull.sh`。以下为新增脚本的推荐行为。

`scripts/diagnose-pull.sh`（或并入 check/doctor）输出类似：

```
[1/4] docker pull 冒烟         -> 立即出现 Pulling fs layer?  [OK/FAIL]
[2/4] DNS: ghcr.io              -> <IP> / ✗
[2/4] DNS: pkg-containers.githubusercontent.com -> <IP> / ✗
[3/4] TCRP: curl ghcr.io/v2/    -> HTTP/2 401 (正常) / 超时 ✗
[4/4] manifest inspect          -> 时间 < 3s?  [OK/慢 ✗]
结论：dns/tcp/tls took  (401 is NORMAL)  -> 若 4 通过而 pull 卡 => Layer CDN 问题
```

实现注意：

- 用 `--connect-timeout 10`、`-m` 超时，避免脚本自身挂起。
- `401` 视为正常，不判为失败。
- 所有步骤都带超时与友好结论，输出给用户可直接照做的下一步。

## 5. D4：文档更新

- `README.md`「部署」与「状态与诊断」章节补充：
  - 说明 `auto` 模式拉取时进度实时显示；
  - 新增「拉取/网络故障排查」小节：`curl -I https://ghcr.io/v2/`（401 正常）、
    `docker manifest inspect`、`getent hosts`、`pkg-containers.githubusercontent.com`。
- 若新增 `scripts/diagnose-pull.sh`，加入「常用命令」清单。

## 6. 不做的事（防蔓延）

- 不改 `pull` / `build` 分支（各自已有正确语义）。
- 不改 `prepare_dsh_image` 其它逻辑。
- 不引入新的依赖（纯 bash + 既有 docker/curl/getent/getent）。
- 不优化镜像体积（列为后续）。

## 7. 参考素材（来自 tmp/a.md）

- GHCR 相关网络端点：`ghcr.io`、`pkg-containers.githubusercontent.com`、`*.pkg.github.com`。
- GitHub Packages 状态：https://www.githubstatus.com/ （Packages 服务）。
- GitHub Docs（容器/注册表网络端点、匿名拉取）：https://docs.github.com/en/packages
- 文中诊断命令（第 4–6 节）可直接作为脚本步骤的蓝本。
