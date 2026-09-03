#!/bin/sh
set -eu

release_base=${MONITOR_RELEASE_BASE:-https://github.com/allury/monitor/releases/latest/download}
temporary_dir=
downloaded_binary=
downloaded_checksum=
binary_path=
listen_address=127.0.0.1:34331

usage() {
    echo "用法: $0 [--public] [--binary 文件]" >&2
    echo "兼容用法: $0 ./monitor-server-linux-amd64" >&2
    echo "不传文件名时，脚本会自动下载与当前架构匹配的最新版主控。" >&2
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "缺少系统命令: $1" >&2
        exit 1
    fi
}

download_file() {
    source_url=$1
    destination=$2
    if command -v curl >/dev/null 2>&1; then
        curl --fail --location --retry 3 --output "$destination" "$source_url"
    elif command -v wget >/dev/null 2>&1; then
        wget --output-document="$destination" "$source_url"
    else
        echo "自动下载需要 curl 或 wget。" >&2
        exit 1
    fi
}

cleanup() {
    if [ -n "$downloaded_binary" ]; then
        rm -f -- "$downloaded_binary"
    fi
    if [ -n "$downloaded_checksum" ]; then
        rm -f -- "$downloaded_checksum"
    fi
    if [ -n "$temporary_dir" ]; then
        rmdir -- "$temporary_dir" 2>/dev/null || true
    fi
}
trap cleanup EXIT

if [ "$(id -u)" -ne 0 ]; then
    echo "请使用 root 运行，例如: sudo sh ./install-server.sh" >&2
    exit 1
fi

while [ "$#" -gt 0 ]; do
    case "$1" in
        --public)
            listen_address=0.0.0.0:34331
            shift
            ;;
        --local)
            listen_address=127.0.0.1:34331
            shift
            ;;
        --binary | -b)
            if [ "$#" -lt 2 ]; then
                usage
                exit 1
            fi
            binary_path=$2
            shift 2
            ;;
        --help | -h)
            usage
            exit 0
            ;;
        -*)
            echo "未知参数: $1" >&2
            usage
            exit 1
            ;;
        *)
            if [ -n "$binary_path" ]; then
                usage
                exit 1
            fi
            binary_path=$1
            shift
            ;;
    esac
done
set --

case "$(uname -m)" in
    x86_64 | amd64)
        release_arch=amd64
        ;;
    aarch64 | arm64)
        release_arch=arm64
        ;;
    *)
        echo "不支持的 CPU 架构: $(uname -m)；目前只提供 amd64 和 arm64。" >&2
        exit 1
        ;;
esac

asset_name="monitor-server-linux-$release_arch"
if [ -n "$binary_path" ]; then
    if [ ! -f "$binary_path" ]; then
        echo "找不到主控文件: $binary_path" >&2
        exit 1
    fi
else
    for command_name in mktemp sha256sum; do
        require_command "$command_name"
    done
    temporary_dir=$(mktemp -d)
    downloaded_binary="$temporary_dir/$asset_name"
    downloaded_checksum="$temporary_dir/$asset_name.sha256"
    echo "正在下载 $asset_name …"
    download_file "$release_base/$asset_name" "$downloaded_binary"
    download_file "$release_base/$asset_name.sha256" "$downloaded_checksum"
    if ! (cd "$temporary_dir" && sha256sum -c "$asset_name.sha256"); then
        echo "主控文件校验失败，已终止安装。" >&2
        exit 1
    fi
    binary_path=$downloaded_binary
fi

for command_name in getent groupadd id useradd install runuser systemctl; do
    require_command "$command_name"
done

nologin_shell=$(command -v nologin 2>/dev/null || true)
if [ -z "$nologin_shell" ]; then
    nologin_shell=/usr/sbin/nologin
fi

getent group monitor >/dev/null 2>&1 || groupadd --system monitor
id monitor >/dev/null 2>&1 || useradd \
    --system \
    --gid monitor \
    --home-dir /var/lib/monitor \
    --shell "$nologin_shell" \
    monitor

install -m 0755 "$binary_path" /usr/local/bin/monitor-server
install -d -m 0700 -o monitor -g monitor /var/lib/monitor

cat > /etc/systemd/system/monitor-server.service <<UNIT
[Unit]
Description=Monitor server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=monitor
Group=monitor
WorkingDirectory=/var/lib/monitor
ExecStart=/usr/local/bin/monitor-server --listen $listen_address --db /var/lib/monitor/monitor.db
Restart=on-failure
RestartSec=3s

NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
RestrictSUIDSGID=yes
RestrictNamespaces=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
CapabilityBoundingSet=
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
SystemCallArchitectures=native
ReadWritePaths=/var/lib/monitor
UMask=0077

[Install]
WantedBy=multi-user.target
UNIT

if [ ! -f /var/lib/monitor/monitor.db ]; then
    echo
    echo "正在初始化管理员密钥，请立即保存下面显示的密钥："
    runuser -u monitor -- /usr/local/bin/monitor-server admin reset \
        --db /var/lib/monitor/monitor.db
else
    echo "检测到已有数据库：保留全部数据和原管理员密钥。"
fi

systemctl daemon-reload
systemctl enable monitor-server >/dev/null
if ! systemctl restart monitor-server; then
    echo "主控启动失败，最近日志如下：" >&2
    systemctl --no-pager --full status monitor-server >&2 || true
    exit 1
fi

echo
echo "主控安装完成。"
echo "监听地址: $listen_address"
echo "查看状态: systemctl status monitor-server --no-pager"
echo "查看日志: journalctl -u monitor-server -n 100 --no-pager"
