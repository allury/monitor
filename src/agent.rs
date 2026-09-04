use std::time::Duration;

#[cfg(not(target_os = "linux"))]
use anyhow::bail;
use anyhow::Result;

#[derive(Debug, Clone)]
pub struct AgentOptions {
    pub server: String,
    pub token: String,
    pub interval: Duration,
    pub disk: String,
    pub telecom: String,
    pub unicom: String,
    pub mobile: String,
}

#[cfg(not(target_os = "linux"))]
pub async fn run(_options: AgentOptions) -> Result<()> {
    bail!("探针只支持 Linux；控制端可以在任意 Linux 服务器运行")
}

#[cfg(target_os = "linux")]
pub async fn run(options: AgentOptions) -> Result<()> {
    linux::run(options).await
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::{CStr, CString};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use anyhow::{bail, Context, Result};
    use futures_util::{SinkExt, StreamExt};
    use http::header::{AUTHORIZATION, USER_AGENT};
    use tokio::net::{lookup_host, TcpStream};
    use tokio::sync::watch;
    use tokio::time::{interval, sleep, timeout, MissedTickBehavior};
    use tokio_tungstenite::connect_async_with_config;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
    use tokio_tungstenite::tungstenite::Message;
    use tracing::{info, warn};

    use super::AgentOptions;
    use crate::model::{
        network_delta, valid_target, AgentReport, Latency, LatencySample, LatencyTargets, Metrics,
        NetworkCounter, ServerMessage, PROTOCOL_VERSION,
    };

    const VERSION: &str = env!("CARGO_PKG_VERSION");

    #[derive(Debug, Clone)]
    struct StaticInfo {
        boot_id: String,
        hostname: String,
        os: String,
        kernel: String,
        arch: String,
        virtualization: String,
        cpu_name: String,
        cpu_cores: u32,
        mem_total: u64,
        swap_total: u64,
        disk_total: u64,
    }

    #[derive(Debug, Default)]
    struct Collector {
        previous_cpu: Option<(u64, u64)>,
        previous_network: Option<(Vec<NetworkCounter>, Instant)>,
    }

    pub async fn run(options: AgentOptions) -> Result<()> {
        if options.interval < Duration::from_secs(1) || options.interval > Duration::from_secs(10) {
            bail!("上报间隔必须为 1–10 秒");
        }
        let mut static_info = collect_static(&options.disk)?;
        let endpoint = websocket_endpoint(&options.server);
        let mut backoff = 2_u64;
        let mut collector = Collector::default();
        let initial_targets = LatencyTargets {
            telecom: options.telecom.clone(),
            unicom: options.unicom.clone(),
            mobile: options.mobile.clone(),
        };
        if !valid_targets(&initial_targets) {
            bail!("延迟地址必须为 host:port 或 [IPv6]:port");
        }
        let (targets, target_updates) = watch::channel(Some((initial_targets, 0)));
        let (sample_tx, samples) = watch::channel(None);
        tokio::spawn(sample_latency(target_updates, sample_tx));
        loop {
            let started = Instant::now();
            match connect_and_report(
                &endpoint,
                &options,
                &mut static_info,
                &mut collector,
                &targets,
                &samples,
            )
            .await
            {
                Ok(()) => warn!("控制端关闭了连接"),
                Err(error) => warn!(%error, "上报连接中断"),
            }
            if started.elapsed() >= Duration::from_secs(30) {
                backoff = 2;
            }
            sleep(Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(30);
            info!("正在重新连接控制端");
        }
    }

    async fn connect_and_report(
        endpoint: &str,
        options: &AgentOptions,
        static_info: &mut StaticInfo,
        collector: &mut Collector,
        targets: &watch::Sender<Option<(LatencyTargets, u64)>>,
        samples: &watch::Receiver<Option<LatencySample>>,
    ) -> Result<()> {
        let mut request = endpoint.into_client_request().context("控制端地址无效")?;
        request.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {}", options.token)
                .parse()
                .context("密钥格式无效")?,
        );
        request.headers_mut().insert(
            USER_AGENT,
            format!("monitor-agent/{VERSION}").parse().unwrap(),
        );
        let mut config = WebSocketConfig::default();
        config.read_buffer_size = 4 * 1024;
        config.write_buffer_size = 4 * 1024;
        config.max_write_buffer_size = 64 * 1024;
        config.max_message_size = Some(64 * 1024);
        config.max_frame_size = Some(64 * 1024);
        let (socket, _) = timeout(
            Duration::from_secs(15),
            connect_async_with_config(request, Some(config), true),
        )
        .await
        .context("连接控制端超时")??;
        info!(endpoint, "已连接控制端");
        let (mut writer, mut reader) = socket.split();
        let mut ticker = interval(options.interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut heartbeat = interval(Duration::from_secs(15));
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut heard_at = Instant::now();
        let mut static_at = Instant::now();
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if static_at.elapsed() >= Duration::from_secs(300) {
                        *static_info = collect_static(&options.disk)?;
                        static_at = Instant::now();
                    }
                    let sample = samples.borrow().clone();
                    let report = collect_report(collector, static_info, options, sample)?;
                    let payload = serde_json::to_string(&report)?;
                    timeout(Duration::from_secs(5), writer.send(Message::Text(payload.into()))).await??;
                }
                _ = heartbeat.tick() => {
                    if heard_at.elapsed() > Duration::from_secs(45) { bail!("控制端心跳超时"); }
                    timeout(Duration::from_secs(5), writer.send(Message::Ping(Vec::new().into()))).await??;
                }
                incoming = reader.next() => {
                    heard_at = Instant::now();
                    match incoming {
                        Some(Ok(Message::Text(payload))) => {
                            if let Ok(ServerMessage::LatencyTargets { targets: updated, revision }) = serde_json::from_str(&payload) {
                                if valid_targets(&updated) {
                                    let next = Some((updated, revision));
                                    if *targets.borrow() != next { targets.send_replace(next); }
                                }
                            }
                        }
                        Some(Ok(Message::Ping(payload))) => { timeout(Duration::from_secs(5), writer.send(Message::Pong(payload))).await??; }
                        Some(Ok(Message::Close(_))) | None => return Ok(()),
                        Some(Err(error)) => return Err(error.into()),
                        _ => {}
                    }
                }
            }
        }
    }

    async fn sample_latency(
        mut config: watch::Receiver<Option<(LatencyTargets, u64)>>,
        samples: watch::Sender<Option<LatencySample>>,
    ) {
        let mut ticker = interval(Duration::from_secs(30));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticker.tick() => {},
                changed = config.changed() => {
                    if changed.is_err() { return; }
                    ticker.reset();
                }
            }
            let Some((targets, revision)) = config.borrow_and_update().clone() else {
                continue;
            };
            let values = tokio::select! {
                values = measure_latency(&targets) => values,
                changed = config.changed() => {
                    if changed.is_err() { return; }
                    ticker.reset_immediately();
                    continue;
                }
            };
            let id = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                .min(i64::MAX as u128) as u64;
            samples.send_replace(Some(LatencySample {
                id,
                revision,
                values,
            }));
        }
    }

    fn collect_report(
        collector: &mut Collector,
        static_info: &StaticInfo,
        options: &AgentOptions,
        sample: Option<LatencySample>,
    ) -> Result<AgentReport> {
        let cpu_now = read_cpu()?;
        let cpu = collector
            .previous_cpu
            .replace(cpu_now)
            .map(|previous| cpu_percent(previous, cpu_now))
            .unwrap_or(0.0);
        let network = read_network()?;
        let (net_rx_total, net_tx_total) = network.iter().fold((0_u64, 0_u64), |(rx, tx), nic| {
            (rx.saturating_add(nic.rx), tx.saturating_add(nic.tx))
        });
        let now = Instant::now();
        let (net_rx, net_tx) = match collector.previous_network.replace((network.clone(), now)) {
            Some((old, old_at)) => {
                let elapsed = now.duration_since(old_at).as_secs_f64().max(0.001);
                let (rx, tx) = network_delta(&old, &network);
                ((rx as f64 / elapsed) as u64, (tx as f64 / elapsed) as u64)
            }
            None => (0, 0),
        };
        let memory = read_memory()?;
        let disk_used = disk_space(&options.disk)?.1;
        Ok(AgentReport {
            protocol: PROTOCOL_VERSION,
            agent_version: VERSION.to_owned(),
            boot_id: static_info.boot_id.clone(),
            hostname: static_info.hostname.clone(),
            os: static_info.os.clone(),
            kernel: static_info.kernel.clone(),
            arch: static_info.arch.clone(),
            virtualization: static_info.virtualization.clone(),
            cpu_name: static_info.cpu_name.clone(),
            cpu_cores: static_info.cpu_cores,
            mem_total: static_info.mem_total,
            swap_total: static_info.swap_total,
            disk_total: static_info.disk_total,
            metrics: Metrics {
                cpu,
                load: read_load(),
                mem_used: memory.mem_used,
                swap_used: memory.swap_used,
                disk_used,
                net_rx,
                net_tx,
                net_rx_total,
                net_tx_total,
                network,
                tcp: socket_count("/proc/net/tcp") + socket_count("/proc/net/tcp6"),
                udp: socket_count("/proc/net/udp") + socket_count("/proc/net/udp6"),
                processes: process_count(),
                uptime: read_uptime(),
                latency: sample
                    .as_ref()
                    .map(|sample| sample.values.clone())
                    .unwrap_or_default(),
                latency_sample: sample,
                latency_at: None,
            },
        })
    }

    fn collect_static(disk: &str) -> Result<StaticInfo> {
        let memory = read_memory()?;
        let (disk_total, _) = disk_space(disk)?;
        let cpu_info = read_text("/proc/cpuinfo");
        let cpu_name = cpu_info
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                matches!(key.trim(), "model name" | "Hardware" | "Processor")
                    .then(|| value.trim().to_owned())
            })
            .unwrap_or_else(|| "Unknown CPU".to_owned());
        let cpu_cores = cpu_info
            .lines()
            .filter(|line| line.starts_with("processor"))
            .count()
            .max(1) as u32;

        Ok(StaticInfo {
            boot_id: read_text("/proc/sys/kernel/random/boot_id")
                .trim()
                .to_owned(),
            hostname: read_text("/etc/hostname").trim().to_owned(),
            os: os_name(),
            kernel: kernel_version(),
            arch: std::env::consts::ARCH.to_owned(),
            virtualization: virtualization(),
            cpu_name,
            cpu_cores,
            mem_total: memory.mem_total,
            swap_total: memory.swap_total,
            disk_total,
        })
    }

    fn websocket_endpoint(server: &str) -> String {
        let mut value = server.trim().trim_end_matches('/').to_owned();
        if let Some(rest) = value.strip_prefix("https://") {
            value = format!("wss://{rest}");
        } else if let Some(rest) = value.strip_prefix("http://") {
            value = format!("ws://{rest}");
        }
        if !value.ends_with("/api/agent") {
            value.push_str("/api/agent");
        }
        value
    }

    #[derive(Debug, Default)]
    struct Memory {
        mem_total: u64,
        mem_used: u64,
        swap_total: u64,
        swap_used: u64,
    }

    fn read_memory() -> Result<Memory> {
        let text = fs::read_to_string("/proc/meminfo").context("无法读取 /proc/meminfo")?;
        let value = |name: &str| -> u64 {
            text.lines()
                .find(|line| line.starts_with(name))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0)
                * 1024
        };
        let total = value("MemTotal:");
        let available = value("MemAvailable:");
        let swap_total = value("SwapTotal:");
        let swap_free = value("SwapFree:");
        Ok(Memory {
            mem_total: total,
            mem_used: total.saturating_sub(available),
            swap_total,
            swap_used: swap_total.saturating_sub(swap_free),
        })
    }

    fn read_cpu() -> Result<(u64, u64)> {
        let text = fs::read_to_string("/proc/stat").context("无法读取 /proc/stat")?;
        let values: Vec<u64> = text
            .lines()
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .skip(1)
            .filter_map(|value| value.parse().ok())
            .collect();
        if values.len() < 4 {
            bail!("/proc/stat 格式无效");
        }
        // guest/guest_nice are already included in user/nice.
        let total = values.iter().take(8).sum();
        let idle = values[3] + values.get(4).copied().unwrap_or(0);
        Ok((total, idle))
    }

    fn cpu_percent(previous: (u64, u64), current: (u64, u64)) -> f32 {
        let total = current.0.saturating_sub(previous.0);
        let idle = current.1.saturating_sub(previous.1);
        if total == 0 {
            return 0.0;
        }
        ((total.saturating_sub(idle)) as f64 * 100.0 / total as f64).clamp(0.0, 100.0) as f32
    }

    fn read_network() -> Result<Vec<NetworkCounter>> {
        let mut counters =
            parse_network(&fs::read_to_string("/proc/net/dev").context("无法读取网络统计")?)?;
        for nic in &mut counters {
            nic.index = read_text(&format!("/sys/class/net/{}/ifindex", nic.name))
                .trim()
                .parse()
                .unwrap_or(0);
        }
        Ok(counters)
    }

    fn parse_network(text: &str) -> Result<Vec<NetworkCounter>> {
        let mut result = Vec::new();
        for line in text.lines().skip(2) {
            let Some((name, counters)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim();
            if name == "lo"
                || [
                    "br",
                    "cni",
                    "docker",
                    "podman",
                    "flannel",
                    "veth",
                    "virbr",
                    "vmbr",
                    "tap",
                    "tun",
                    "wg",
                    "tailscale",
                    "fwbr",
                    "fwpr",
                ]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                continue;
            }
            let fields: Vec<&str> = counters.split_whitespace().collect();
            if fields.len() < 9 {
                bail!("网络统计格式无效");
            }
            result.push(NetworkCounter {
                name: name.to_owned(),
                index: 0,
                rx: fields[0].parse()?,
                tx: fields[8].parse()?,
            });
        }
        if result.len() > 64 {
            bail!("网卡数量超出 64 个的限制");
        }
        Ok(result)
    }

    fn read_load() -> [f32; 3] {
        let text = read_text("/proc/loadavg");
        let mut values = text
            .split_whitespace()
            .take(3)
            .map(|value| value.parse().unwrap_or(0.0));
        [
            values.next().unwrap_or(0.0),
            values.next().unwrap_or(0.0),
            values.next().unwrap_or(0.0),
        ]
    }

    fn read_uptime() -> u64 {
        read_text("/proc/uptime")
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0) as u64
    }

    fn process_count() -> u32 {
        fs::read_dir("/proc")
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .bytes()
                            .all(|byte| byte.is_ascii_digit())
                    })
                    .count() as u32
            })
            .unwrap_or(0)
    }

    fn socket_count(path: &str) -> u32 {
        read_text(path).lines().count().saturating_sub(1) as u32
    }

    fn disk_space(path: &str) -> Result<(u64, u64)> {
        let path = CString::new(Path::new(path).as_os_str().as_bytes())?;
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        let result = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
        if result != 0 {
            return Err(std::io::Error::last_os_error()).context("无法读取磁盘统计");
        }
        let stats = unsafe { stats.assume_init() };
        let block_size = stats.f_frsize;
        let total = stats.f_blocks.saturating_mul(block_size);
        let available = stats.f_bavail.saturating_mul(block_size);
        Ok((total, total.saturating_sub(available)))
    }

    fn os_name() -> String {
        read_text("/etc/os-release")
            .lines()
            .find_map(|line| line.strip_prefix("PRETTY_NAME="))
            .map(|value| value.trim_matches('"').to_owned())
            .unwrap_or_else(|| "Linux".to_owned())
    }

    fn kernel_version() -> String {
        let mut name = std::mem::MaybeUninit::<libc::utsname>::uninit();
        if unsafe { libc::uname(name.as_mut_ptr()) } != 0 {
            return "Linux".to_owned();
        }
        let name = unsafe { name.assume_init() };
        unsafe { CStr::from_ptr(name.release.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    fn virtualization() -> String {
        if Path::new("/.dockerenv").exists() {
            return "docker".to_owned();
        }
        let cgroup = read_text("/proc/1/cgroup").to_ascii_lowercase();
        for name in ["docker", "lxc", "podman", "containerd"] {
            if cgroup.contains(name) {
                return name.to_owned();
            }
        }
        let product = read_text("/sys/class/dmi/id/product_name").to_ascii_lowercase();
        for (needle, name) in [
            ("kvm", "kvm"),
            ("qemu", "qemu"),
            ("vmware", "vmware"),
            ("virtualbox", "virtualbox"),
            ("hyper-v", "hyper-v"),
        ] {
            if product.contains(needle) {
                return name.to_owned();
            }
        }
        if Path::new("/proc/xen").exists() {
            return "xen".to_owned();
        }
        if read_text("/proc/cpuinfo").contains("hypervisor") {
            "virtualized"
        } else {
            "unknown"
        }
        .to_owned()
    }

    async fn measure_latency(targets: &LatencyTargets) -> Latency {
        let (telecom, unicom, mobile) = tokio::join!(
            tcp_latency(&targets.telecom),
            tcp_latency(&targets.unicom),
            tcp_latency(&targets.mobile),
        );
        Latency {
            telecom,
            unicom,
            mobile,
        }
    }

    async fn tcp_latency(address: &str) -> Option<f32> {
        // Resolve separately so the displayed TCP latency does not include DNS time.
        let destination = timeout(Duration::from_secs(3), lookup_host(address))
            .await
            .ok()?
            .ok()?
            .next()?;
        let started = Instant::now();
        match timeout(Duration::from_secs(3), TcpStream::connect(destination)).await {
            Ok(Ok(_)) => Some(started.elapsed().as_secs_f32() * 1000.0),
            _ => None,
        }
    }

    fn valid_targets(targets: &LatencyTargets) -> bool {
        [&targets.telecom, &targets.unicom, &targets.mobile]
            .into_iter()
            .all(|target| valid_target(target))
    }

    fn read_text(path: &str) -> String {
        fs::read_to_string(path).unwrap_or_default()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_network_and_ignores_loopback() {
            let fixture = "Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n    lo: 100 0 0 0 0 0 0 0 200 0 0 0 0 0 0 0\n  eth0: 300 0 0 0 0 0 0 0 400 0 0 0 0 0 0 0\n";
            let counters = parse_network(fixture).unwrap();
            assert_eq!(
                counters,
                vec![NetworkCounter {
                    name: "eth0".into(),
                    index: 0,
                    rx: 300,
                    tx: 400
                }]
            );
        }

        #[test]
        fn calculates_cpu_delta() {
            assert_eq!(cpu_percent((100, 40), (200, 80)), 60.0);
        }

        #[test]
        fn normalizes_websocket_url() {
            assert_eq!(
                websocket_endpoint("https://monitor.example.com/"),
                "wss://monitor.example.com/api/agent"
            );
            assert_eq!(
                websocket_endpoint("http://192.0.2.10:34331"),
                "ws://192.0.2.10:34331/api/agent"
            );
        }
    }
}
