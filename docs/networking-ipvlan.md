# 绑定公网 IP 直通模式（IPVlan / Macvlan 免端口映射）

本文档介绍如何将指定的公网 IP 直接绑定给 `dsh-docker` 容器（基于 Docker `ipvlan` / `macvlan` 驱动）。

---

## 解决的核心痛点

在服务器上开发项目时，常见两大挑战：
1. **指定公网 IP 绑定**：希望开发容器**直接绑定特定某一个公网 IP**，让该公网 IP 的所有流量直达容器。
2. **动态端口免映射**：容器内部可能会同时运行多个开发项目（如前端 `3000`/`5173`、后端 API `8080`、数据库 `5432` 等），且端口在开发过程中经常变动、无法预先固定。

传统 Docker 桥接（Bridge）模式必须逐一在 `compose.yaml` 中配置 `ports:` 映射，一旦端口有变动必须重启容器。
**通过直接绑定公网 IP（直通模式）**：
- 容器**直接持有该公网 IP**，完全免除 Docker 端口映射。
- 容器内部监听任何端口，外部均可通过该公网 IP 直接访问对应端口，随开随用。

---

## 为什么推荐 IPVlan 而非 Macvlan？

| 驱动模式 | 机制 | VPS 兼容性 | 推荐场景 |
|---|---|---|---|
| **IPVlan (L2)**（推荐） | 容器与宿主机物理网卡**共享同一个 MAC 地址**，仅分配独立的 IP。 | **极高**。公网云厂商（AWS、阿里云、腾讯云、Hetzner 等）在虚拟交换机上开启了防 MAC 欺骗，IPVlan 不产生新 MAC，不会被拦截。 | **公网 VPS / 云服务器首选** |
| **Macvlan** | 容器虚拟出全新的独立的 MAC 地址与 IP。 | 较低。若宿主机网络开启了端口安全（Port Security）或禁止多 MAC，数据包会被直接丢弃。 | 仅限自建机房或内网裸金属环境 |

---

## 快速配置指南

### 第一步：准备你要绑定的公网 IP

你**只需要知道准备分配给该容器的公网 IP 地址**（例如 `198.51.100.2`）。

> **无需操心复杂的网络参数**：宿主机物理网卡（如 `eth0`）、子网掩码与网段（Subnet）、默认网关（Gateway）均由安装脚本与 `dshctl` 从系统网络路由中**全自动识别与推导**，你只需要传 `--network-ip` 即可！

（可选）如果想查看宿主机当前绑定的所有 IP 与网卡：
```bash
ip -4 addr show
```

### 第二步：选择部署方式

#### 方式 A：使用一键安装脚本（全新一键部署，推荐）

直接在终端执行一条命令，通过 `--network-ip` 指定要绑定的公网 IP（物理网卡、子网掩码与网关均自动探测）：

```bash
curl -fsSL https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.sh | bash -s -- \
  --dir /workapps/test/dsh-docker \
  --variant dev \
  --admin qwq \
  --password 123456 \
  --weak-password \
  --network-ip 198.51.100.2 \
  --yes
```

#### 方式 B：手动编辑已存在的 `.env` 并部署

若你已克隆或存在项目目录，在根目录的 `.env` 中添加以下配置即可（网卡、网关与子网由系统自动推导，一般无需手动填写）：

```bash
# 启用绑定公网 IP 直通模式
DSH_NETWORK_MODE=ipvlan

# 直接绑定给该容器的公网 IP
DSH_NETWORK_IP=198.51.100.2

# 高级可选配置（仅当默认自动推导不符合特殊网络环境时手动指定）：
# DSH_NETWORK_PARENT=eth0
# DSH_NETWORK_SUBNET=198.51.100.0/24
# DSH_NETWORK_GATEWAY=198.51.100.1
# DSH_NETWORK_IPVLAN_MODE=l2
# DSH_NETWORK_ATTACH_BRIDGE=true
```

通过 `dshctl` 进行校验并部署：

```bash
# 1. 检查配置合法性
dshctl config validate

# 2. 部署运行（dshctl 会自动选择 compose.ipvlan.yaml 启动）
dshctl deploy

# 3. 查看状态
dshctl status
```

`dshctl status` 将输出类似如下信息：
```text
Network mode : ipvlan (direct IP: 198.51.100.2, parent: eth0)
Proxy URL    : http://198.51.100.2:3080
...
```

---

## 开发使用体验

部署成功后，外部网络即可直接通过 `http://198.51.100.2:3080` 访问 DSH Web 控制台。

当你在容器内部开发项目时：
1. 启动一个 Vite 前端项目：
   ```bash
   pnpm dev --host 0.0.0.0 --port 5173
   ```
   外部电脑直接在浏览器访问：`http://198.51.100.2:5173`
2. 启动一个 Python Flask API：
   ```bash
   python -m flask run --host 0.0.0.0 --port 8000
   ```
   外部直接访问：`http://198.51.100.2:8000`
3. 启动一个自定义 TCP 服务（如 Redis、MQTT）：
   外部直接连接 `198.51.100.2:<对应端口>`，完全免除重新配置 Docker 端口映射与重启容器的烦恼！

---

## 进阶注意事项与故障排查

### 1. 宿主机与容器互通（Linux 内核机制）
Linux 内核出于安全隔离设计，**默认禁止宿主机物理网卡与其衍生的 IPVlan/Macvlan 子网卡直接通信**：
- **外部互联网访问容器 IP**：正常通畅。
- **容器内部访问外部互联网**：正常通畅（由于开启了双网络 `DSH_NETWORK_ATTACH_BRIDGE=true`，出站完全无忧）。
- **VPS 宿主机自身执行 `curl http://198.51.100.2:3080`**：可能会提示超时或不可达。

> **如果 VPS 宿主机上的 Nginx/反代需要访问容器**：
> 宿主机可直接通过 Docker bridge 的内网 IP 通信，或者在宿主机创建一条虚拟 shim 路由：
> ```bash
> # 可选：在宿主机上添加一条 shim 接口以打通宿主机到容器 IP 的直连
> ip link add dsh-shim link eth0 type ipvlan mode l2
> ip addr add 198.51.100.254/32 dev dsh-shim
> ip link set dsh-shim up
> ip route add 198.51.100.2/32 dev dsh-shim
> ```

### 2. 安全与防火墙
在 IPVlan 独占 IP 模式下，容器内运行的任何端口均会直接暴露在公网该 IP 上。
建议：
- 不要在容器内使用默认无密码的数据库或敏感服务；
- 如需限制访问，可在 VPS 宿主机使用 `nftables`/`iptables` 或云厂商安全组只对可信源 IP 放行特定开发端口。
