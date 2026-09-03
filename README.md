# Monitor

一个刻意保持很小的 Linux 服务器监控。主控与只读探针是两个独立的 Rust 二进制：主控内嵌状态页并持有 SQLite，探针只负责采集和上报。没有前端运行时、配置目录或插件。

## 只做这些

- CPU、负载、内存、Swap、硬盘、进程、连接数与在线状态
- 电信、联通、移动三网 TCP 延迟
- 实时网速、今日、本月和累计流量
- 30 天分钟级历史数据（为后续图表保留，不增加管理功能）
- 单管理员后台：节点、三网延迟地址、站点文字

不会加入通知、远程 SSH、插件、多用户权限、Windows/macOS 探针或自动更新。

## 安全边界

- 探针仅读取 Linux 的 `/proc`、`/sys`、`/etc/os-release` 和指定挂载点，不写文件。
- 每个节点使用独立的 256-bit 随机密钥；数据库只保存 SHA-256 摘要。
- 只有一个管理员密钥，没有账号列表、角色或权限系统。数据库只保存密钥摘要，登录会话只存在主控内存中。
- WebSocket 上报限制为 64 KiB，字段长度和数值范围会在入库前校验。
- 建议控制端只监听 `127.0.0.1`，由 Caddy/Nginx 提供 HTTPS；探针连接 `wss://` 地址。
- `monitor-agent` 不包含主控、SQLite、网页、监听端口或管理命令，也没有自动更新和文件写入逻辑。

## 使用 GitHub 构建

推送到 GitHub 后，`CI` 会自动格式化检查、静态检查和测试。创建 `v*` 标签后，`Release` 会生成两个静态 Linux 二进制：

- `monitor-server-linux-amd64` / `monitor-server-linux-arm64`
- `monitor-agent-linux-amd64` / `monitor-agent-linux-arm64`

因此开发电脑不需要安装 Rust、Node.js 或 SQLite。

## 快速开始

### 1. 控制端

下载与服务器架构对应的 Release 文件，重命名并赋予执行权限：

```bash
sudo install -m 0755 monitor-server-linux-amd64 /usr/local/bin/monitor-server
sudo install -d -o monitor -g monitor /var/lib/monitor
sudo -u monitor /usr/local/bin/monitor-server node create \
  --db /var/lib/monitor/monitor.db \
  --id hk-1 \
  --name 香港
```

也可以先启动控制端，然后访问 `/admin` 创建节点。第一次启动会在终端打印一次管理员密钥。如果丢失，可在主控服务器本机执行：

```bash
sudo -u monitor /usr/local/bin/monitor-server admin reset \
  --db /var/lib/monitor/monitor.db
sudo systemctl restart monitor-server
```

创建节点时只显示一次节点密钥，请立即保存。主控默认只监听本机 `34331` 端口：

```bash
sudo -u monitor /usr/local/bin/monitor-server \
  --listen 127.0.0.1:34331 \
  --db /var/lib/monitor/monitor.db
```

生产环境可复制 [`deploy/monitor-server.service`](deploy/monitor-server.service) 到 `/etc/systemd/system/`。HTTPS 反向代理需要支持 WebSocket；Caddy 最小配置如下：

```caddy
monitor.example.com {
    reverse_proxy 127.0.0.1:34331
}
```

### 2. 探针

先前生成的完整密钥已经包含节点 ID。临时运行：

```bash
MONITOR_TOKEN='hk-1.密钥内容' monitor-agent \
  --server https://monitor.example.com
```

`--server` 填的就是对外 HTTPS 反向代理域名，不是 `127.0.0.1:34331`。探针会自动转换为 `wss://monitor.example.com/api/agent` 连接。

长期运行时，将密钥写入 root 专有文件，并使用 hardened systemd 单元：

```bash
sudo install -m 0600 /dev/stdin /etc/monitor-agent.token <<'TOKEN'
hk-1.密钥内容
TOKEN
sudo cp deploy/monitor-agent.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now monitor-agent
```

编辑单元中的 `--server` 地址后再启动。Agent 通过 systemd credential 读取密钥，密钥不会出现在命令行或普通环境变量中。

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

三网探测默认使用 TCP/80 地址。登录 `/admin` 可以统一修改，保存后立即下发到所有在线探针；探针参数和环境变量仅作为首次连接前的备用值：

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

Agent 上报内核网卡累计计数，控制端按 `boot_id` 计算增量并持久化。首次接入只建立基线，不把安装前的流量算进来；Agent 重启不会重复累计，VPS 重启后也会从新一轮内核计数继续累加。回环网卡不计入流量；月流量按 UTC 自然月归零。

SQLite 使用 WAL、8 MiB page cache 和批量写入。每 2 秒上报实时状态，每分钟最多落一个历史采样，自动保留 30 天。状态 API 与浏览器 WebSocket 共用同一份 2 秒 JSON 快照。

## 项目结构

```text
src/bin/monitor-agent.rs    独立探针入口
src/bin/monitor-server.rs   独立主控入口
src/agent.rs                Linux 只读采集与上报
src/server.rs               HTTP/WebSocket 与内存快照
src/db.rs                   SQLite、节点密钥与流量累计
src/ui/                     嵌入主控的状态页
deploy/            完全分开的主控/探针安装脚本与 systemd 安全单元
.github/workflows  无本地环境的 CI 与 Release 构建
```

许可证：[MIT](LICENSE)
