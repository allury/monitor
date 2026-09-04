use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 1;
pub const DEFAULT_LATENCY_INTERVAL_SECS: u64 = 30;

fn default_latency_interval() -> u64 {
    DEFAULT_LATENCY_INTERVAL_SECS
}

pub fn valid_latency_interval(seconds: u64) -> bool {
    (10..=3600).contains(&seconds)
}
#[cfg(feature = "server")]
pub const OFFLINE_AFTER_SECS: i64 = 12;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Latency {
    pub telecom: Option<f32>,
    pub unicom: Option<f32>,
    pub mobile: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencySample {
    pub id: u64,
    pub revision: u64,
    #[serde(default)]
    pub interval_seconds: Option<u64>,
    pub values: Latency,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkCounter {
    pub name: String,
    pub index: u32,
    pub rx: u64,
    pub tx: u64,
}

// A newly discovered/replaced interface establishes a baseline, not a traffic spike.
pub fn network_delta(previous: &[NetworkCounter], current: &[NetworkCounter]) -> (u64, u64) {
    current.iter().fold((0_u64, 0_u64), |(rx, tx), next| {
        let Some(old) = previous
            .iter()
            .find(|old| old.name == next.name && old.index == next.index)
        else {
            return (rx, tx);
        };
        let delta = |before, after| {
            if after >= before {
                after - before
            } else {
                after
            }
        };
        (
            rx.saturating_add(delta(old.rx, next.rx)),
            tx.saturating_add(delta(old.tx, next.tx)),
        )
    })
}

pub fn valid_target(target: &str) -> bool {
    if target.is_empty() || target.len() > 255 || !target.is_ascii() {
        return false;
    }
    let Some((host, port)) = target.rsplit_once(':') else {
        return false;
    };
    if !port.parse::<u16>().is_ok_and(|port| port > 0) {
        return false;
    }
    if host.starts_with('[') && host.ends_with(']') {
        return host[1..host.len() - 1]
            .parse::<std::net::Ipv6Addr>()
            .is_ok();
    }
    !host.is_empty()
        && host.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LatencyTargets {
    pub telecom: String,
    pub unicom: String,
    pub mobile: String,
    #[serde(default = "default_latency_interval")]
    pub interval_seconds: u64,
}

impl Default for LatencyTargets {
    fn default() -> Self {
        Self {
            telecom: "ha-ct-v4.ip.zstaticcdn.com:80".to_owned(),
            unicom: "ha-cu-v4.ip.zstaticcdn.com:80".to_owned(),
            mobile: "ha-cm-v4.ip.zstaticcdn.com:80".to_owned(),
            interval_seconds: DEFAULT_LATENCY_INTERVAL_SECS,
        }
    }
}

#[cfg(feature = "server")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSettings {
    pub name: String,
    pub description: String,
    pub footer: String,
}

#[cfg(feature = "server")]
impl Default for SiteSettings {
    fn default() -> Self {
        Self {
            name: "Monitor".to_owned(),
            description: "服务器状态".to_owned(),
            footer: String::new(),
        }
    }
}

#[cfg(feature = "server")]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub latency: LatencyTargets,
    pub latency_revision: u64,
    pub site: SiteSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    LatencyTargets {
        targets: LatencyTargets,
        #[serde(default)]
        revision: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Metrics {
    pub cpu: f32,
    pub load: [f32; 3],
    pub mem_used: u64,
    pub swap_used: u64,
    pub disk_used: u64,
    pub net_rx: u64,
    pub net_tx: u64,
    pub net_rx_total: u64,
    pub net_tx_total: u64,
    pub tcp: u32,
    pub udp: u32,
    pub processes: u32,
    pub uptime: u64,
    pub latency: Latency,
    #[serde(default)]
    pub latency_sample: Option<LatencySample>,
    #[serde(default)]
    pub latency_at: Option<i64>,
    #[serde(default)]
    pub network: Vec<NetworkCounter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReport {
    pub protocol: u8,
    pub agent_version: String,
    pub boot_id: String,
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub virtualization: String,
    pub cpu_name: String,
    pub cpu_cores: u32,
    pub mem_total: u64,
    pub swap_total: u64,
    pub disk_total: u64,
    pub metrics: Metrics,
}

#[cfg(feature = "server")]
#[derive(Debug, Clone)]
pub struct StoredNode {
    pub id: String,
    pub name: String,
    pub token_hash: Vec<u8>,
    pub last_seen: i64,
    pub agent_version: String,
    pub boot_id: String,
    pub last_rx_counter: u64,
    pub last_tx_counter: u64,
    pub total_rx: u64,
    pub total_tx: u64,
    pub month_key: String,
    pub month_rx: u64,
    pub month_tx: u64,
    pub day_key: String,
    pub day_rx: u64,
    pub day_tx: u64,
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub virtualization: String,
    pub cpu_name: String,
    pub cpu_cores: u32,
    pub mem_total: u64,
    pub swap_total: u64,
    pub disk_total: u64,
    pub metrics: Option<Metrics>,
    pub last_sample_minute: i64,
    pub connection_id: u64,
    pub close_signal: Option<tokio::sync::watch::Sender<bool>>,
}

#[cfg(feature = "server")]
#[derive(Debug, Clone, Serialize)]
pub struct NodeView {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub last_seen: i64,
    pub agent_version: String,
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub virtualization: String,
    pub cpu_name: String,
    pub cpu_cores: u32,
    pub mem_total: u64,
    pub swap_total: u64,
    pub disk_total: u64,
    pub total_rx: u64,
    pub total_tx: u64,
    pub month_rx: u64,
    pub month_tx: u64,
    pub day_rx: u64,
    pub day_tx: u64,
    pub metrics: Option<Metrics>,
}

#[cfg(feature = "server")]
impl StoredNode {
    pub fn as_view(&self, now: i64) -> NodeView {
        let date = chrono::DateTime::from_timestamp(now, 0).unwrap_or_default();
        let month_matches = self.month_key == date.format("%Y-%m").to_string();
        let day_matches = self.day_key == date.format("%Y-%m-%d").to_string();
        NodeView {
            id: self.id.clone(),
            name: self.name.clone(),
            online: self.close_signal.is_some()
                && self.metrics.is_some()
                && (0..=OFFLINE_AFTER_SECS).contains(&(now - self.last_seen)),
            last_seen: self.last_seen,
            agent_version: self.agent_version.clone(),
            hostname: self.hostname.clone(),
            os: self.os.clone(),
            kernel: self.kernel.clone(),
            arch: self.arch.clone(),
            virtualization: self.virtualization.clone(),
            cpu_name: self.cpu_name.clone(),
            cpu_cores: self.cpu_cores,
            mem_total: self.mem_total,
            swap_total: self.swap_total,
            disk_total: self.disk_total,
            total_rx: self.total_rx,
            total_tx: self.total_tx,
            month_rx: if month_matches { self.month_rx } else { 0 },
            month_tx: if month_matches { self.month_tx } else { 0 },
            day_rx: if day_matches { self.day_rx } else { 0 },
            day_tx: if day_matches { self.day_tx } else { 0 },
            metrics: self.metrics.clone(),
        }
    }
}

#[cfg(feature = "server")]
#[derive(Debug, Clone, Serialize)]
pub struct StatusResponse {
    pub generated_at: i64,
    pub site: SiteSettings,
    pub nodes: Vec<NodeView>,
}

#[cfg(feature = "server")]
#[derive(Debug, Clone, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

#[cfg(feature = "server")]
#[derive(Debug, Clone)]
pub struct PersistEvent {
    pub node: StoredNode,
    pub write_sample: bool,
    pub latency: Option<LatencySample>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nic(name: &str, index: u32, rx: u64, tx: u64) -> NetworkCounter {
        NetworkCounter {
            name: name.into(),
            index,
            rx,
            tx,
        }
    }

    #[test]
    fn network_baseline_reset_and_interface_replacement() {
        let old = vec![nic("eth0", 2, 100, 200)];
        assert_eq!(network_delta(&[], &old), (0, 0));
        assert_eq!(network_delta(&old, &[nic("eth0", 2, 150, 260)]), (50, 60));
        assert_eq!(network_delta(&old, &[nic("eth0", 2, 10, 20)]), (10, 20));
        assert_eq!(network_delta(&old, &[nic("eth0", 3, 5000, 6000)]), (0, 0));
        assert_eq!(network_delta(&old, &[]), (0, 0));
        assert_eq!(
            network_delta(
                &old,
                &[nic("eth0", 2, 101, 201), nic("eth1", 4, 99999, 99999)]
            ),
            (1, 1)
        );
    }

    #[test]
    fn targets_require_a_host_and_port_and_support_ipv6() {
        for value in ["example.com:80", "127.0.0.1:34331", "[2001:db8::1]:443"] {
            assert!(valid_target(value), "{value}");
        }
        for value in [
            "",
            "http://example.com:80",
            "::1:80",
            "[]:80",
            "[oops]:80",
            "host:0",
            "host:65536",
            "a..b:80",
            "x:80/path",
            "x:80#bad",
            "x\n:80",
            "-x:80",
        ] {
            assert!(!valid_target(value), "{value}");
        }
    }

    #[test]
    fn legacy_metrics_deserialize_without_new_fields() {
        let mut json = serde_json::to_value(Metrics::default()).unwrap();
        for key in ["network", "latency_sample", "latency_at"] {
            json.as_object_mut().unwrap().remove(key);
        }
        let metrics: Metrics = serde_json::from_value(json).unwrap();
        assert!(metrics.network.is_empty());
        assert!(metrics.latency_sample.is_none());
    }

    #[test]
    fn legacy_latency_config_and_samples_remain_readable() {
        let targets: LatencyTargets = serde_json::from_str(
            r#"{"telecom":"a.example:80","unicom":"b.example:80","mobile":"c.example:80"}"#,
        )
        .unwrap();
        assert_eq!(targets.interval_seconds, 30);
        assert_eq!(targets.telecom, "a.example:80");
        let sample: LatencySample = serde_json::from_str(
            r#"{"id":1,"revision":2,"values":{"telecom":null,"unicom":0,"mobile":30}}"#,
        )
        .unwrap();
        assert_eq!(sample.interval_seconds, None);
        assert_eq!(sample.values.unicom, Some(0.0));
        for value in [10, 30, 60, 3600] {
            assert!(valid_latency_interval(value));
        }
        for value in [0, 9, 3601, u64::MAX] {
            assert!(!valid_latency_interval(value));
        }
    }
}
