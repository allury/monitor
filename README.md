# Monitor

## 项目简介

轻量的 Linux 服务器监控，使用 Rust 编写，主控与探针独立安装。提供服务器状态、三网 TCP 延迟、实时网速、累计及每月流量，配有单管理员后台。

支持 Linux amd64 / arm64 和 systemd。探针以低权限运行，不提供远程执行、文件管理或自动更新。

## 用法

### 安装主控

```sh
curl -fsSL https://github.com/allury/monitor/releases/latest/download/install-server.sh | sudo sh
```

使用 wget：

```sh
wget -qO- https://github.com/allury/monitor/releases/latest/download/install-server.sh | sudo sh
```

默认端口为 34331，仅监听本机。需要直接通过 IP 临时测试时：

```sh
curl -fsSL https://github.com/allury/monitor/releases/latest/download/install-server.sh | sudo sh -s -- --public
```

HTTP 会明文传输密钥与数据，仅用于临时测试。首次安装显示的管理员密钥请妥善保存。

### 安装探针

在后台添加节点，复制生成的一键安装命令，到对应被控服务器执行。命令会自动使用当前访问后台的地址，安装探针不会安装主控。

```sh
curl -fsSL https://github.com/allury/monitor/releases/latest/download/install-agent.sh | sudo sh -s -- --server 'https://monitor.example.com' --token '节点密钥'
```

将示例地址和密钥替换为后台提供的值。三网检测地址和站点文字均在后台设置。状态默认每 2 秒上报，三网检测每 30 秒一次；每月流量按 UTC 自然月统计。

### 更新

主控重新执行安装命令，保留数据库和管理员密钥；通过 IP 直接测试的主控更新时仍需带上 `--public`，否则会恢复为仅监听本机。

已安装的探针在后台点击“安装 / 更新”复制更新命令，或执行：

```sh
curl -fsSL https://github.com/allury/monitor/releases/latest/download/install-agent.sh | sudo sh -s -- --update
```

也可将 `curl -fsSL` 换为 `wget -qO-`。更新只替换程序并重启，保留原地址、密钥和服务配置，不需要重新获取或重置密钥。首次安装或更换上报地址时，使用完整安装命令。

主控只保存密钥摘要，无法再次显示原密钥。新机器安装且未保存原密钥时，可在后台重置；原密钥会立即失效。

更新顺序为先主控、后探针。更新前备份主控数据库。安装命令下载[最近正式版](https://github.com/allury/monitor/releases/latest)，不包含尚未发布的源码改动。

### 查看状态和日志

```sh
systemctl status monitor-server --no-pager
journalctl -u monitor-server -n 100 --no-pager
```

探针使用：

```sh
systemctl status monitor-agent --no-pager
journalctl -u monitor-agent -n 100 --no-pager
```

## 删除方法

请在对应的主控或被控服务器执行；下面的命令不操作 Komari。

### 删除探针

```sh
sudo systemctl disable --now monitor-agent
sudo rm -f -- /etc/systemd/system/monitor-agent.service /usr/local/bin/monitor-agent /etc/monitor-agent.token
sudo rm -f -- /etc/systemd/system/monitor-agent.service.d/10-server.conf
sudo rmdir -- /etc/systemd/system/monitor-agent.service.d 2>/dev/null || true
sudo systemctl daemon-reload
```

随后可在后台停用对应节点。主控已保存的历史数据不会因此删除。

### 删除主控，保留数据

```sh
sudo systemctl disable --now monitor-server
sudo rm -f -- /etc/systemd/system/monitor-server.service /usr/local/bin/monitor-server
sudo systemctl daemon-reload
```

数据保留在 `/var/lib/monitor/monitor.db`，重新安装可以继续使用。

### 彻底删除主控数据

先完成上面的主控卸载。以下操作会永久删除节点、管理员密钥和历史数据；请确认已有备份或不再需要。

```sh
sudo rm -f -- /var/lib/monitor/monitor.db /var/lib/monitor/monitor.db-wal /var/lib/monitor/monitor.db-shm
sudo rmdir -- /var/lib/monitor
```

以上操作保留 `monitor` 系统用户和组。仅当它们由本项目创建、且未用于其他服务时，才额外执行：

```sh
sudo userdel monitor
sudo groupdel monitor
```
