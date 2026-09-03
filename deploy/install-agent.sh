#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "请使用 root 运行。" >&2
    exit 1
fi
if [ "$#" -ne 2 ] || [ ! -f "$1" ]; then
    echo "用法: $0 /path/to/monitor-agent-linux-amd64 https://monitor.example.com" >&2
    exit 1
fi
if [ ! -f "$(dirname "$0")/monitor-agent.service" ]; then
    echo "找不到 monitor-agent.service，请保持安装脚本与 deploy 文件在同一目录。" >&2
    exit 1
fi

if ! printf '%s\n' "$2" | grep -Eq '^https://[A-Za-z0-9.-]+(:[0-9]{1,5})?$'; then
    echo "上报地址必须是纯 HTTPS 反向代理域名，例如 https://monitor.example.com。" >&2
    exit 1
fi

printf '请粘贴后台创建节点时显示的密钥: '
trap 'stty echo 2>/dev/null || true' EXIT INT TERM
stty -echo
IFS= read -r token
stty echo
trap - EXIT INT TERM
printf '\n'
if [ -z "$token" ]; then
    echo "密钥不能为空。" >&2
    exit 1
fi

install -m 0755 "$1" /usr/local/bin/monitor-agent
umask 077
printf '%s\n' "$token" > /etc/monitor-agent.token
unset token
install -m 0644 "$(dirname "$0")/monitor-agent.service" /etc/systemd/system/monitor-agent.service
install -d -m 0755 /etc/systemd/system/monitor-agent.service.d
cat > /etc/systemd/system/monitor-agent.service.d/10-server.conf <<EOF
[Service]
ExecStart=
ExecStart=/usr/local/bin/monitor-agent --server $2 --interval 2 --disk /
EOF
systemctl daemon-reload
systemctl enable --now monitor-agent
echo "探针已启动，上报到 $2"
