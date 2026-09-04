#!/bin/sh
set -eu

# Run only on an isolated Linux test runner, never on a monitored production host.
set -- --quiet --wait --pipe --collect "--unit=monitor-sandbox-test-${GITHUB_RUN_ID:-$$}"
while IFS= read -r line; do
    case "$line" in
        DynamicUser=*|NoNewPrivileges=*|Protect*=*|Private*=*|ReadOnlyPaths=*|InaccessiblePaths=*|SystemCallFilter=*|Restrict*=*|LockPersonality=*|MemoryDenyWriteExecute=*|CapabilityBoundingSet=*|SystemCallArchitectures=*|UMask=*)
            set -- "$@" "--property=$line"
            ;;
    esac
done < deploy/monitor-agent.service

sudo systemd-run "$@" /bin/sh -c '
    set -eu
    test -r /proc/stat
    test -r /proc/net/dev
    test -r /proc/meminfo
    for directory in /tmp /var/tmp /dev/shm /etc /run; do
        if (printf denied > "$directory/monitor-sandbox-write-test") 2>/dev/null; then
            echo "FAIL: writable $directory" >&2
            exit 1
        fi
    done
    if (printf 0 > /proc/self/oom_score_adj) 2>/dev/null; then
        echo "FAIL: writable /proc" >&2
        exit 1
    fi
    echo "PASS: proc metrics readable; filesystem and temporary-directory writes denied"
'
