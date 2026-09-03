use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 1;
#[cfg(feature = "server")]
pub const OFFLINE_AFTER_SECS: i64 = 12;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Latency {
    pub telecom: Option<f32>,
    pub unicom: Option<f32>,
    pub mobile: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LatencyTargets {
    pub telecom: String,
    pub unicom: String,
    pub mobile: String,
}

impl Default for LatencyTargets {
    fn default() -> Self {
        Self {
            telecom: "ha-ct-v4.ip.zstaticcdn.com:80".to_owned(),
            unicom: "ha-cu-v4.ip.zstaticcdn.com:80".to_owned(),
            mobile: "ha-cm-v4.ip.zstaticcdn.com:80".to_owned(),
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
            footer: "只读状态页 · 数据每 2 秒刷新".to_owned(),
        }
    }
}

#[cfg(feature = "server")]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub latency: LatencyTargets,
    pub site: SiteSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    LatencyTargets { targets: LatencyTargets },
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
        NodeView {
            id: self.id.clone(),
            name: self.name.clone(),
            online: self.metrics.is_some() && now - self.last_seen <= OFFLINE_AFTER_SECS,
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
            month_rx: self.month_rx,
            month_tx: self.month_tx,
            day_rx: self.day_rx,
            day_tx: self.day_tx,
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
}
