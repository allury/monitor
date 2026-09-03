#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "请使用 root 运行。" >&2
    exit 1
fi
if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
    echo "用法: $0 /path/to/monitor-server-linux-amd64" >&2
    exit 1
fi
if [ ! -f "$(dirname "$0")/monitor-server.service" ]; then
    echo "找不到 monitor-server.service，请保持安装脚本与 deploy 文件在同一目录。" >&2
    exit 1
fi

getent group monitor >/dev/null 2>&1 || groupadd --system monitor
id monitor >/dev/null 2>&1 || useradd --system --gid monitor --home-dir /var/lib/monitor --shell /usr/sbin/nologin monitor
install -m 0755 "$1" /usr/local/bin/monitor-server
install -d -m 0700 -o monitor -g monitor /var/lib/monitor
install -m 0644 "$(dirname "$0")/monitor-server.service" /etc/systemd/system/monitor-server.service

if [ ! -f /var/lib/monitor/monitor.db ]; then
    echo "正在初始化本机管理员密钥…"
    runuser -u monitor -- /usr/local/bin/monitor-server admin reset --db /var/lib/monitor/monitor.db
else
    echo "已有数据库未改动，管理员密钥保持不变。"
fi
systemctl daemon-reload
systemctl enable --now monitor-server
echo "主控已启动：127.0.0.1:34331"
