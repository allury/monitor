# Monitor

一个刻意保持很小的 Linux 服务器监控。主控与只读探针是两个独立的 Rust 二进制：主控内嵌状态页并持有 SQLite，探针只负责采集和上报。没有前端运行时、配置目录或插件。

## 安装

先执行 `uname -m` 确认架构：

| 输出 | 下载文件后缀 |
| --- | --- |
| `x86_64` | `linux-amd64` |
| `aarch64`、`arm64` | `linux-arm64` |

所有文件都在 [Releases](https://github.com/allury/monitor/releases/latest)。以下命令适用于公开仓库；仓库保持私有时，未认证的 `curl`/`wget` 会收到 404，需要先在浏览器登录 GitHub 后下载再上传。

### 主控

使用 `curl` 一键安装：

```bash
curl -fsSL https://github.com/allury/monitor/releases/latest/download/install-server.sh | sudo sh
```

暂时没有反代、需要直接用 `http://服务器IP:34331` 测试时，显式开启公网监听：

```bash
curl -fsSL https://github.com/allury/monitor/releases/latest/download/install-server.sh | sudo sh -s -- --public
```

HTTP 会明文传输节点密钥和监控数据，只适合临时测试。配置好 HTTPS 后，重新执行不带 `--public` 的安装命令即可切回本机监听，数据库不会被重置。

也可以使用 `wget`：

```bash
wget -qO- https://github.com/allury/monitor/releases/latest/download/install-server.sh | sudo sh
```

脚本会自动识别 `amd64`/`arm64`、下载对应二进制及 SHA-256 校验文件。也支持把文件上传到同一目录后离线安装：

```bash
sudo sh ./install-server.sh ./monitor-server-linux-amd64
```

安装脚本会创建低权限系统用户、写入 systemd 服务、初始化数据库并启动主控。首次安装时终端会显示一次管理员密钥，请立即保存。

不要写成 `deploy/install-server.sh`；只有克隆了完整仓库才存在 `deploy` 目录。两个文件就在 `/root` 时，路径必须是 `./install-server.sh`。

检查运行状态：

```bash
systemctl status monitor-server --no-pager
journalctl -u monitor-server -n 100 --no-pager
```

### 探针

主控后台创建节点后，会直接生成一条包含当前访问地址和节点密钥的一键安装命令。复制到被控服务器执行即可；用 IP 测试时会生成 HTTP 地址，换成反代域名访问后台后会生成 HTTPS 地址。形式如下：

```bash
curl -fsSL https://github.com/allury/monitor/releases/latest/download/install-agent.sh | sudo sh -s -- --server 'https://monitor.example.com' --token '节点密钥'
```

也可以使用 `wget`：

```bash
wget -qO- https://github.com/allury/monitor/releases/latest/download/install-agent.sh | sudo sh -s -- --server 'https://monitor.example.com' --token '节点密钥'
```

脚本会自动识别架构、校验下载文件、写入 root 专有密钥文件并启动 systemd 服务。主控和探针安装脚本完全独立，安装探针不会安装主控。

如果已手动上传文件，也可以交互安装；脚本会隐藏密钥输入：

```bash
sudo sh ./install-agent.sh ./monitor-agent-linux-amd64 https://monitor.example.com
```

检查运行状态：

```bash
systemctl status monitor-agent --no-pager
journalctl -u monitor-agent -n 100 --no-pager
```

### 手动更新

重新执行对应的一键安装命令即可。主控脚本会保留 `/var/lib/monitor/monitor.db` 和原管理员密钥；探针脚本会更新二进制、上报地址和节点密钥。项目不包含自动更新功能。

## 只做这些

- CPU、负载、内存、Swap、硬盘、进程、连接数与在线状态
- 电信、联通、移动三网 TCP 延迟
- 实时网速、今日、本月和累计流量
- 30 天分钟级历史数据
- 单管理员后台：节点、三网延迟地址、站点文字

不会加入通知、远程 SSH、插件、多用户权限、Windows/macOS 探针或自动更新。

## 安全边界

- 探针进程仅读取 Linux 的 `/proc`、`/sys`、`/etc/os-release` 和指定挂载点，不写文件。
- 每个节点使用独立的 256-bit 随机密钥；数据库只保存 SHA-256 摘要。
- 只有一个管理员密钥，没有账号列表、角色或权限系统。登录会话只存在主控内存中。
- WebSocket 上报限制为 64 KiB，字段长度和数值范围会在入库前校验。
- 主控默认只监听 `127.0.0.1:34331`；`--public` 可临时监听公网。探针支持 HTTP/WS 测试和 HTTPS/WSS 正式上报。
- `monitor-agent` 不包含主控、SQLite、网页、监听端口、管理命令、自动更新或文件写入逻辑。
- 两个 systemd 服务默认启用权限收紧选项；主控只能写 `/var/lib/monitor`，探针根文件系统只读。

## GitHub 构建与发布

推送代码后，`CI` 自动运行格式检查、Clippy 和测试。推送 `v*` 标签后，`Release` 自动生成四个静态 Linux 二进制、校验文件以及两个独立安装脚本：

- `monitor-server-linux-amd64` / `monitor-server-linux-arm64`
- `monitor-agent-linux-amd64` / `monitor-agent-linux-arm64`
- `install-server.sh` / `install-agent.sh`

## 命令

```text
monitor-server [--listen 127.0.0.1:34331] [--db monitor.db]
monitor-server admin reset [--db monitor.db]
monitor-server node create --id <ID> --name <名称> [--db monitor.db]
monitor-server node revoke --id <ID> [--db monitor.db]
monitor-server node list [--db monitor.db]
monitor-agent --server <URL> [--interval 2] [--disk /]
              [--telecom host:port] [--unicom host:port] [--mobile host:port]
```

三网探测默认使用 TCP/80 地址。后台可以统一修改，保存后会立即下发到所有在线探针；探针参数和环境变量只作为首次连接前的备用值：

```bash
monitor-agent --server https://monitor.example.com \
  --telecom ha-ct-v4.ip.zstaticcdn.com:80 \
  --unicom ha-cu-v4.ip.zstaticcdn.com:80 \
  --mobile ha-cm-v4.ip.zstaticcdn.com:80

MONITOR_PING_TELECOM=ha-ct-v4.ip.zstaticcdn.com:80
MONITOR_PING_UNICOM=ha-cu-v4.ip.zstaticcdn.com:80
MONITOR_PING_MOBILE=ha-cm-v4.ip.zstaticcdn.com:80
```

## 数据与流量口径

探针上报内核网卡累计计数，主控按 `boot_id` 计算增量并持久化。首次接入只建立基线，不把安装前的流量算进来；探针重启不会重复累计，VPS 重启后也会从新一轮内核计数继续累加。回环网卡不计入流量；月流量按 UTC 自然月归零。

SQLite 使用 WAL、8 MiB page cache 和批量写入。每 2 秒上报实时状态，每分钟最多落一个历史采样，自动保留 30 天。状态 API 与浏览器 WebSocket 共用同一份 2 秒 JSON 快照。

## 项目结构

```text
src/bin/monitor-agent.rs    独立探针入口
src/bin/monitor-server.rs   独立主控入口
src/agent.rs                Linux 只读采集与上报
src/server.rs               HTTP/WebSocket 与内存快照
src/db.rs                   SQLite、节点密钥与流量累计
src/ui/                     嵌入主控的状态页
deploy/                     独立安装脚本与 systemd 安全单元
.github/workflows/          CI 与 Release 构建
```

许可证：[MIT](LICENSE)
