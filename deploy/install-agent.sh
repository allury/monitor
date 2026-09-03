#!/bin/sh
set -eu

release_base=${MONITOR_RELEASE_BASE:-https://github.com/allury/monitor/releases/latest/download}
temporary_dir=
downloaded_binary=
downloaded_checksum=
binary_path=
server_url=
token=

usage() {
    echo "用法: $0 [--binary 文件] --server http(s)://主控地址 [--token 节点密钥]" >&2
    echo "兼容用法: $0 ./monitor-agent-linux-amd64 http(s)://主控地址" >&2
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
    echo "请使用 root 运行。" >&2
    exit 1
fi

while [ "$#" -gt 0 ]; do
    case "$1" in
        --binary | -b)
            if [ "$#" -lt 2 ]; then
                usage
                exit 1
            fi
            binary_path=$2
            shift 2
            ;;
        --server | --endpoint | -e)
            if [ "$#" -lt 2 ]; then
                usage
                exit 1
            fi
            server_url=$2
            shift 2
            ;;
        --token | -t)
            if [ "$#" -lt 2 ]; then
                usage
                exit 1
            fi
            token=$2
            shift 2
            ;;
        --help | -h)
            usage
            exit 0
            ;;
        http://* | https://*)
            if [ -n "$server_url" ]; then
                usage
                exit 1
            fi
            server_url=$1
            shift
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

if [ -z "$server_url" ]; then
    echo "缺少主控地址。" >&2
    usage
    exit 1
fi
if ! printf '%s\n' "$server_url" | grep -Eq '^https?://[A-Za-z0-9.-]+(:[0-9]{1,5})?$'; then
    echo "主控地址格式无效，例如 http://192.0.2.10:34331 或 https://monitor.example.com。" >&2
    exit 1
fi

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

asset_name="monitor-agent-linux-$release_arch"
if [ -n "$binary_path" ]; then
    if [ ! -f "$binary_path" ]; then
        echo "找不到探针文件: $binary_path" >&2
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
        echo "探针文件校验失败，已终止安装。" >&2
        exit 1
    fi
    binary_path=$downloaded_binary
fi

for command_name in install systemctl; do
    require_command "$command_name"
done

if [ -z "$token" ]; then
    printf '请粘贴后台创建节点时显示的节点密钥: '
    if [ -t 0 ]; then
        trap 'stty echo 2>/dev/null || true; cleanup' EXIT
        trap 'exit 130' INT
        trap 'exit 143' TERM
        stty -echo
        IFS= read -r token
        stty echo
        trap cleanup EXIT
        trap - INT TERM
        printf '\n'
    else
        IFS= read -r token
    fi
fi
if [ -z "$token" ]; then
    echo "节点密钥不能为空。" >&2
    exit 1
fi

install -m 0755 "$binary_path" /usr/local/bin/monitor-agent
umask 077
printf '%s\n' "$token" > /etc/monitor-agent.token
unset token

cat > /etc/systemd/system/monitor-agent.service <<UNIT
[Unit]
Description=Monitor read-only agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
DynamicUser=yes
LoadCredential=token:/etc/monitor-agent.token
Environment=MONITOR_TOKEN_FILE=%d/token
ExecStart=/usr/local/bin/monitor-agent --server $server_url --interval 2 --disk /
Restart=always
RestartSec=5s

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
UMask=0077

[Install]
WantedBy=multi-user.target
UNIT

# v0.1.0 使用过单独的地址覆盖文件；删除它以确保重新安装可以切换上报地址。
rm -f -- /etc/systemd/system/monitor-agent.service.d/10-server.conf
rmdir -- /etc/systemd/system/monitor-agent.service.d 2>/dev/null || true

systemctl daemon-reload
systemctl enable monitor-agent >/dev/null
if ! systemctl restart monitor-agent; then
    echo "探针启动失败，最近日志如下：" >&2
    systemctl --no-pager --full status monitor-agent >&2 || true
    exit 1
fi

echo
echo "探针安装完成，上报到: $server_url"
echo "查看状态: systemctl status monitor-agent --no-pager"
echo "查看日志: journalctl -u monitor-agent -n 100 --no-pager"
