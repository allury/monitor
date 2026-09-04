use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::{AppSettings, LatencyTargets, Metrics, PersistEvent, SiteSettings, StoredNode};

pub fn open(path: &Path) -> Result<Connection> {
    let connection =
        Connection::open(path).with_context(|| format!("无法打开数据库 {}", path.display()))?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;
         PRAGMA temp_store=MEMORY;
         PRAGMA cache_size=-8192;
         PRAGMA wal_autocheckpoint=1000;
         PRAGMA journal_size_limit=67108864;",
    )?;
    migrate(&connection)?;
    Ok(connection)
}

fn migrate(connection: &Connection) -> Result<()> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > 2 {
        bail!("数据库由较新版本创建，不能降级打开");
    }
    if version == 2 {
        return Ok(());
    }
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE IF NOT EXISTS nodes (
             id          TEXT PRIMARY KEY,
             name        TEXT NOT NULL,
             token_hash  BLOB NOT NULL,
             enabled     INTEGER NOT NULL DEFAULT 1,
             created_at  INTEGER NOT NULL
         );

         CREATE TABLE IF NOT EXISTS node_state (
             node_id             TEXT PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
             last_seen           INTEGER NOT NULL DEFAULT 0,
             agent_version       TEXT NOT NULL DEFAULT '',
             boot_id             TEXT NOT NULL DEFAULT '',
             last_rx_counter     INTEGER NOT NULL DEFAULT 0,
             last_tx_counter     INTEGER NOT NULL DEFAULT 0,
             total_rx            INTEGER NOT NULL DEFAULT 0,
             total_tx            INTEGER NOT NULL DEFAULT 0,
             month_key           TEXT NOT NULL DEFAULT '',
             month_rx            INTEGER NOT NULL DEFAULT 0,
             month_tx            INTEGER NOT NULL DEFAULT 0,
             day_key             TEXT NOT NULL DEFAULT '',
             day_rx              INTEGER NOT NULL DEFAULT 0,
             day_tx              INTEGER NOT NULL DEFAULT 0,
             hostname            TEXT NOT NULL DEFAULT '',
             os                  TEXT NOT NULL DEFAULT '',
             kernel              TEXT NOT NULL DEFAULT '',
             arch                TEXT NOT NULL DEFAULT '',
             virtualization      TEXT NOT NULL DEFAULT '',
             cpu_name            TEXT NOT NULL DEFAULT '',
             cpu_cores           INTEGER NOT NULL DEFAULT 0,
             mem_total           INTEGER NOT NULL DEFAULT 0,
             swap_total          INTEGER NOT NULL DEFAULT 0,
             disk_total          INTEGER NOT NULL DEFAULT 0,
             metrics_json        TEXT
         );

         CREATE TABLE IF NOT EXISTS samples (
             node_id      TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
             minute       INTEGER NOT NULL,
             cpu          REAL NOT NULL,
             mem_used     INTEGER NOT NULL,
             disk_used    INTEGER NOT NULL,
             net_rx       INTEGER NOT NULL,
             net_tx       INTEGER NOT NULL,
             telecom      REAL,
             unicom       REAL,
             mobile       REAL,
             PRIMARY KEY (node_id, minute)
         ) WITHOUT ROWID;

         CREATE INDEX IF NOT EXISTS idx_samples_minute ON samples(minute);

         CREATE TABLE IF NOT EXISTS latency_samples (
             node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
             sample_id INTEGER NOT NULL,
             at INTEGER NOT NULL,
             revision INTEGER NOT NULL,
             telecom REAL,
             unicom REAL,
             mobile REAL,
             PRIMARY KEY (node_id, sample_id)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS idx_latency_history ON latency_samples(node_id, revision, at);
         CREATE INDEX IF NOT EXISTS idx_latency_at ON latency_samples(at);

         CREATE TABLE IF NOT EXISTS settings (
             key    TEXT PRIMARY KEY,
             value  TEXT NOT NULL
         ) WITHOUT ROWID;

         PRAGMA user_version=2;
         COMMIT;",
    )?;
    Ok(())
}

pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub fn ensure_admin(connection: &Connection) -> Result<Option<String>> {
    if setting(connection, "admin_hash")?.is_some() {
        return Ok(None);
    }
    let password = random_secret(24);
    let encoded = URL_SAFE_NO_PAD.encode(token_hash(&password));
    connection.execute(
        "INSERT INTO settings (key, value) VALUES ('admin_hash', ?1)",
        [encoded],
    )?;
    Ok(Some(password))
}

pub fn admin_hash(connection: &Connection) -> Result<Vec<u8>> {
    let encoded = setting(connection, "admin_hash")?.context("管理员密钥尚未初始化")?;
    URL_SAFE_NO_PAD
        .decode(encoded)
        .context("管理员密钥摘要损坏")
}

pub fn reset_admin(path: &Path) -> Result<String> {
    let password = random_secret(24);
    let encoded = URL_SAFE_NO_PAD.encode(token_hash(&password));
    let connection = open(path)?;
    connection.execute(
        "INSERT INTO settings (key, value) VALUES ('admin_hash', ?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [encoded],
    )?;
    Ok(password)
}

pub fn load_settings(connection: &Connection) -> Result<AppSettings> {
    let latency = setting(connection, "latency_targets")?
        .and_then(|value| serde_json::from_str::<LatencyTargets>(&value).ok())
        .unwrap_or_default();
    let site = setting(connection, "site")?
        .and_then(|value| serde_json::from_str::<SiteSettings>(&value).ok())
        .unwrap_or_default();
    let latency_revision = setting(connection, "latency_revision")?
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    Ok(AppSettings {
        latency,
        latency_revision,
        site,
    })
}

pub fn save_latency(path: &Path, targets: &LatencyTargets) -> Result<u64> {
    let mut connection = open(path)?;
    let transaction = connection.transaction()?;
    let old = load_settings(&transaction)?;
    if old.latency == *targets {
        return Ok(old.latency_revision);
    }
    let revision = old.latency_revision + 1;
    for (key, value) in [
        ("latency_targets", serde_json::to_string(targets)?),
        ("latency_revision", revision.to_string()),
    ] {
        transaction.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value])?;
    }
    transaction.commit()?;
    Ok(revision)
}

pub fn save_site(path: &Path, site: &SiteSettings) -> Result<()> {
    save_setting(path, "site", &serde_json::to_string(site)?)
}

pub fn create_node(path: &Path, id: &str, name: &str) -> Result<String> {
    validate_id(id)?;
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        bail!("节点名称长度必须为 1–80 个字符");
    }

    let token = format!("{id}.{}", random_secret(32));
    let hash = token_hash(&token);
    let mut connection = open(path)?;
    let transaction = connection.transaction()?;
    if transaction
        .query_row("SELECT 1 FROM nodes WHERE id=?1", [id], |_| Ok(()))
        .optional()?
        .is_some()
    {
        bail!("节点 ID 已存在（包括已停用节点），请使用新 ID；重置密钥请使用节点操作");
    }
    transaction.execute(
        "INSERT INTO nodes (id, name, token_hash, enabled, created_at)
         VALUES (?1, ?2, ?3, 1, unixepoch())",
        params![id, name, hash],
    )?;
    transaction.execute(
        "INSERT INTO node_state (node_id) VALUES (?1)
         ON CONFLICT(node_id) DO NOTHING",
        [id],
    )?;
    transaction.commit()?;
    Ok(token)
}

pub fn rotate_node_token(path: &Path, id: &str) -> Result<Option<String>> {
    validate_id(id)?;
    let token = format!("{id}.{}", random_secret(32));
    let connection = open(path)?;
    let changed = connection.execute(
        "UPDATE nodes SET token_hash=?2 WHERE id=?1 AND enabled=1",
        params![id, token_hash(&token)],
    )?;
    Ok((changed > 0).then_some(token))
}

pub fn revoke_node(path: &Path, id: &str) -> Result<bool> {
    validate_id(id)?;
    let connection = open(path)?;
    let changed = connection.execute("UPDATE nodes SET enabled=0 WHERE id=?1", [id])?;
    Ok(changed > 0)
}

pub fn list_nodes(path: &Path) -> Result<Vec<(String, String, bool, i64)>> {
    let connection = open(path)?;
    let mut statement = connection
        .prepare("SELECT id, name, enabled, created_at FROM nodes ORDER BY created_at, id")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get::<_, i64>(2)? != 0,
            row.get(3)?,
        ))
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn load_nodes(connection: &Connection) -> Result<Vec<StoredNode>> {
    let mut statement = connection.prepare(
        "SELECT n.id, n.name, n.token_hash,
                COALESCE(s.last_seen, 0), COALESCE(s.agent_version, ''),
                COALESCE(s.boot_id, ''), COALESCE(s.last_rx_counter, 0),
                COALESCE(s.last_tx_counter, 0), COALESCE(s.total_rx, 0),
                COALESCE(s.total_tx, 0), COALESCE(s.month_key, ''),
                COALESCE(s.month_rx, 0), COALESCE(s.month_tx, 0),
                COALESCE(s.day_key, ''), COALESCE(s.day_rx, 0),
                COALESCE(s.day_tx, 0), COALESCE(s.hostname, ''),
                COALESCE(s.os, ''), COALESCE(s.kernel, ''),
                COALESCE(s.arch, ''), COALESCE(s.virtualization, ''),
                COALESCE(s.cpu_name, ''), COALESCE(s.cpu_cores, 0),
                COALESCE(s.mem_total, 0), COALESCE(s.swap_total, 0),
                COALESCE(s.disk_total, 0), s.metrics_json,
                COALESCE((SELECT MAX(minute) FROM samples p WHERE p.node_id=n.id), 0)
         FROM nodes n
         LEFT JOIN node_state s ON s.node_id=n.id
         WHERE n.enabled=1
         ORDER BY n.created_at, n.id",
    )?;
    let rows = statement.query_map([], |row| {
        let metrics_json: Option<String> = row.get(26)?;
        let metrics = metrics_json.and_then(|json| serde_json::from_str::<Metrics>(&json).ok());
        Ok(StoredNode {
            id: row.get(0)?,
            name: row.get(1)?,
            token_hash: row.get(2)?,
            last_seen: row.get(3)?,
            agent_version: row.get(4)?,
            boot_id: row.get(5)?,
            last_rx_counter: from_sql_u64(row.get::<_, i64>(6)?),
            last_tx_counter: from_sql_u64(row.get::<_, i64>(7)?),
            total_rx: from_sql_u64(row.get::<_, i64>(8)?),
            total_tx: from_sql_u64(row.get::<_, i64>(9)?),
            month_key: row.get(10)?,
            month_rx: from_sql_u64(row.get::<_, i64>(11)?),
            month_tx: from_sql_u64(row.get::<_, i64>(12)?),
            day_key: row.get(13)?,
            day_rx: from_sql_u64(row.get::<_, i64>(14)?),
            day_tx: from_sql_u64(row.get::<_, i64>(15)?),
            hostname: row.get(16)?,
            os: row.get(17)?,
            kernel: row.get(18)?,
            arch: row.get(19)?,
            virtualization: row.get(20)?,
            cpu_name: row.get(21)?,
            cpu_cores: row.get::<_, i64>(22)?.max(0) as u32,
            mem_total: from_sql_u64(row.get::<_, i64>(23)?),
            swap_total: from_sql_u64(row.get::<_, i64>(24)?),
            disk_total: from_sql_u64(row.get::<_, i64>(25)?),
            metrics,
            last_sample_minute: row.get(27)?,
            connection_id: 0,
            close_signal: None,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn persist_batch(connection: &mut Connection, events: &[PersistEvent]) -> Result<()> {
    let transaction = connection.transaction()?;
    for event in events {
        let node = &event.node;
        // A revoked/rotated identity must not write through an already queued event.
        if transaction
            .query_row(
                "SELECT 1 FROM nodes WHERE id=?1 AND enabled=1 AND token_hash=?2",
                params![node.id, node.token_hash],
                |_| Ok(()),
            )
            .optional()?
            .is_none()
        {
            continue;
        }
        let metrics_json = node
            .metrics
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        transaction.execute(
            "INSERT INTO node_state (
                 node_id, last_seen, agent_version, boot_id,
                 last_rx_counter, last_tx_counter, total_rx, total_tx,
                 month_key, month_rx, month_tx, day_key, day_rx, day_tx,
                 hostname, os, kernel, arch, virtualization, cpu_name, cpu_cores,
                 mem_total, swap_total, disk_total, metrics_json
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25
             ) ON CONFLICT(node_id) DO UPDATE SET
                 last_seen=excluded.last_seen,
                 agent_version=excluded.agent_version,
                 boot_id=excluded.boot_id,
                 last_rx_counter=excluded.last_rx_counter,
                 last_tx_counter=excluded.last_tx_counter,
                 total_rx=excluded.total_rx,
                 total_tx=excluded.total_tx,
                 month_key=excluded.month_key,
                 month_rx=excluded.month_rx,
                 month_tx=excluded.month_tx,
                 day_key=excluded.day_key,
                 day_rx=excluded.day_rx,
                 day_tx=excluded.day_tx,
                 hostname=excluded.hostname,
                 os=excluded.os,
                 kernel=excluded.kernel,
                 arch=excluded.arch,
                 virtualization=excluded.virtualization,
                 cpu_name=excluded.cpu_name,
                 cpu_cores=excluded.cpu_cores,
                 mem_total=excluded.mem_total,
                 swap_total=excluded.swap_total,
                 disk_total=excluded.disk_total,
                 metrics_json=excluded.metrics_json",
            params![
                node.id,
                node.last_seen,
                node.agent_version,
                node.boot_id,
                to_sql_u64(node.last_rx_counter),
                to_sql_u64(node.last_tx_counter),
                to_sql_u64(node.total_rx),
                to_sql_u64(node.total_tx),
                node.month_key,
                to_sql_u64(node.month_rx),
                to_sql_u64(node.month_tx),
                node.day_key,
                to_sql_u64(node.day_rx),
                to_sql_u64(node.day_tx),
                node.hostname,
                node.os,
                node.kernel,
                node.arch,
                node.virtualization,
                node.cpu_name,
                node.cpu_cores as i64,
                to_sql_u64(node.mem_total),
                to_sql_u64(node.swap_total),
                to_sql_u64(node.disk_total),
                metrics_json,
            ],
        )?;
        if event.write_sample {
            if let Some(metrics) = &node.metrics {
                transaction.execute(
                    "INSERT OR REPLACE INTO samples (
                         node_id, minute, cpu, mem_used, disk_used, net_rx, net_tx,
                         telecom, unicom, mobile
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        node.id,
                        node.last_sample_minute,
                        metrics.cpu,
                        to_sql_u64(metrics.mem_used),
                        to_sql_u64(metrics.disk_used),
                        to_sql_u64(metrics.net_rx),
                        to_sql_u64(metrics.net_tx),
                        metrics.latency.telecom,
                        metrics.latency.unicom,
                        metrics.latency.mobile,
                    ],
                )?;
            }
        }
        if let Some(sample) = &event.latency {
            transaction.execute(
                "INSERT OR IGNORE INTO latency_samples(node_id,sample_id,at,revision,telecom,unicom,mobile) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![node.id, to_sql_u64(sample.id), node.metrics.as_ref().and_then(|m| m.latency_at).unwrap_or(node.last_seen), to_sql_u64(sample.revision), sample.values.telecom, sample.values.unicom, sample.values.mobile],
            )?;
        }
    }
    transaction.commit()?;
    Ok(())
}

pub fn prune_history(connection: &Connection, now: i64) -> Result<usize> {
    let oldest_minute = now.div_euclid(60) - 30 * 24 * 60;
    let resources = connection.execute("DELETE FROM samples WHERE minute < ?1", [oldest_minute])?;
    let latency = connection.execute(
        "DELETE FROM latency_samples WHERE at < ?1",
        [now - 30 * 86400],
    )?;
    Ok(resources + latency)
}

#[derive(Debug, Serialize)]
pub struct ResourcePoint {
    at: i64,
    cpu: f64,
    mem_used: f64,
    disk_used: f64,
    net_rx: f64,
    net_tx: f64,
}

#[derive(Debug, Serialize)]
pub struct LatencyPoint {
    at: i64,
    telecom: Option<f64>,
    unicom: Option<f64>,
    mobile: Option<f64>,
    count: u32,
    telecom_failures: u32,
    unicom_failures: u32,
    mobile_failures: u32,
}

#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    pub from: i64,
    pub to: i64,
    pub step: i64,
    pub resources: Vec<ResourcePoint>,
    pub latency: Vec<LatencyPoint>,
    pub targets: LatencyTargets,
}

pub fn history(
    path: &Path,
    id: &str,
    hours: u32,
    latency: bool,
    now: i64,
) -> Result<HistoryResponse> {
    validate_id(id)?;
    if !(1..=720).contains(&hours) {
        bail!("历史范围必须为 1–720 小时");
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_secs(2))?;
    connection.pragma_update(None, "cache_size", -1024)?;
    if node_name(&connection, id)?.is_none() {
        bail!("节点不存在或已停用");
    }
    let settings = load_settings(&connection)?;
    let span = i64::from(hours) * 3600;
    let quantum = if latency { 30 } else { 60 };
    let step = ((span + 1199 * quantum) / (1200 * quantum)).max(1) * quantum;
    let mut response = HistoryResponse {
        from: now - span,
        to: now,
        step,
        resources: Vec::new(),
        latency: Vec::new(),
        targets: settings.latency,
    };
    if latency {
        let mut query = connection.prepare("SELECT MIN(at), AVG(telecom), AVG(unicom), AVG(mobile), COUNT(*), SUM(telecom IS NULL), SUM(unicom IS NULL), SUM(mobile IS NULL) FROM latency_samples WHERE node_id=?1 AND revision=?5 AND at>=?2 AND at<?4 GROUP BY (at-?2)/?3 ORDER BY MIN(at) LIMIT 1200")?;
        response.latency = query
            .query_map(
                params![
                    id,
                    response.from,
                    step,
                    now,
                    to_sql_u64(settings.latency_revision)
                ],
                |row| {
                    Ok(LatencyPoint {
                        at: row.get(0)?,
                        telecom: row.get(1)?,
                        unicom: row.get(2)?,
                        mobile: row.get(3)?,
                        count: row.get(4)?,
                        telecom_failures: row.get(5)?,
                        unicom_failures: row.get(6)?,
                        mobile_failures: row.get(7)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
    } else {
        let mut query = connection.prepare("SELECT MIN(minute)*60, AVG(cpu), AVG(mem_used), AVG(disk_used), AVG(net_rx), AVG(net_tx) FROM samples WHERE node_id=?1 AND minute>=?2 AND minute<?4 GROUP BY (minute-?2)/?3 ORDER BY MIN(minute) LIMIT 1200")?;
        response.resources = query
            .query_map(
                params![
                    id,
                    (response.from + 59).div_euclid(60),
                    step / 60,
                    (now + 59).div_euclid(60)
                ],
                |row| {
                    Ok(ResourcePoint {
                        at: row.get(0)?,
                        cpu: row.get(1)?,
                        mem_used: row.get(2)?,
                        disk_used: row.get(3)?,
                        net_rx: row.get(4)?,
                        net_tx: row.get(5)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
    }
    Ok(response)
}

pub fn node_name(connection: &Connection, id: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT name FROM nodes WHERE id=?1 AND enabled=1",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("节点 ID 只能包含 1–64 个字母、数字、连字符或下划线");
    }
    Ok(())
}

fn setting(connection: &Connection, key: &str) -> Result<Option<String>> {
    connection
        .query_row("SELECT value FROM settings WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(Into::into)
}

fn save_setting(path: &Path, key: &str, value: &str) -> Result<()> {
    let connection = open(path)?;
    connection.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn random_secret(bytes: usize) -> String {
    let mut secret = vec![0_u8; bytes];
    OsRng.fill_bytes(&mut secret);
    URL_SAFE_NO_PAD.encode(secret)
}

fn to_sql_u64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn from_sql_u64(value: i64) -> u64 {
    value.max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Latency, LatencySample};

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, StoredNode) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        create_node(&path, "n", "Node").unwrap();
        let mut node = load_nodes(&open(&path).unwrap()).unwrap().remove(0);
        node.last_seen = 1_780_000_000;
        node.last_sample_minute = node.last_seen / 60;
        node.metrics = Some(Metrics {
            cpu: 20.0,
            ..Metrics::default()
        });
        (dir, path, node)
    }

    #[test]
    fn duplicate_node_does_not_rotate_credentials() {
        let (_dir, path, node) = fixture();
        assert!(create_node(&path, "n", "Other").is_err());
        assert_eq!(
            load_nodes(&open(&path).unwrap()).unwrap()[0].token_hash,
            node.token_hash
        );
    }

    #[test]
    fn retry_is_atomic_and_idempotent() {
        let (_dir, path, mut node) = fixture();
        node.total_rx = 1234;
        let event = PersistEvent {
            node,
            write_sample: true,
            latency: None,
        };
        let mut connection = open(&path).unwrap();
        connection.execute_batch("CREATE TRIGGER fail_sample BEFORE INSERT ON samples BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
        assert!(persist_batch(&mut connection, std::slice::from_ref(&event)).is_err());
        assert_eq!(load_nodes(&connection).unwrap()[0].total_rx, 0);
        connection
            .execute_batch("DROP TRIGGER fail_sample;")
            .unwrap();
        persist_batch(&mut connection, std::slice::from_ref(&event)).unwrap();
        persist_batch(&mut connection, &[event]).unwrap();
        assert_eq!(load_nodes(&connection).unwrap()[0].total_rx, 1234);
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM samples", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn latency_keeps_both_thirty_second_rounds_and_failure_counts() {
        let (_dir, path, mut node) = fixture();
        let mut connection = open(&path).unwrap();
        let start = node.last_seen;
        for (id, value) in [(1, Some(20.0)), (2, None)] {
            node.last_seen = start + (id as i64 - 1) * 30;
            let sample = LatencySample {
                id,
                revision: 0,
                values: Latency {
                    telecom: value,
                    unicom: Some(30.0),
                    mobile: None,
                },
            };
            let event = PersistEvent {
                node: node.clone(),
                write_sample: false,
                latency: Some(sample),
            };
            persist_batch(&mut connection, std::slice::from_ref(&event)).unwrap();
            persist_batch(&mut connection, &[event]).unwrap();
        }
        let raw = history(&path, "n", 1, true, start + 90).unwrap();
        assert_eq!(raw.latency.len(), 2);
        assert_eq!(raw.latency[0].telecom, Some(20.0));
        assert_eq!(raw.latency[1].telecom, None);
        assert_eq!(raw.latency[1].telecom_failures, 1);
        let combined = history(&path, "n", 720, true, start + 90).unwrap();
        assert_eq!(combined.latency.len(), 1);
        assert_eq!(combined.latency[0].count, 2);
        assert_eq!(combined.latency[0].telecom, Some(20.0));
        assert_eq!(combined.latency[0].mobile_failures, 2);
    }

    #[test]
    fn targets_change_revision_without_mixing_old_history() {
        let (_dir, path, node) = fixture();
        let mut connection = open(&path).unwrap();
        let sample = LatencySample {
            id: 1,
            revision: 0,
            values: Latency::default(),
        };
        persist_batch(
            &mut connection,
            &[PersistEvent {
                node: node.clone(),
                write_sample: false,
                latency: Some(sample),
            }],
        )
        .unwrap();
        let targets = LatencyTargets {
            telecom: "example.com:443".into(),
            ..LatencyTargets::default()
        };
        assert_eq!(save_latency(&path, &targets).unwrap(), 1);
        assert_eq!(save_latency(&path, &targets).unwrap(), 1);
        assert!(history(&path, "n", 1, true, node.last_seen + 5)
            .unwrap()
            .latency
            .is_empty());
    }

    #[test]
    fn revoked_or_rotated_tokens_cannot_persist_queued_reports() {
        let (_dir, path, node) = fixture();
        let mut connection = open(&path).unwrap();
        let event = PersistEvent {
            node,
            write_sample: true,
            latency: None,
        };
        rotate_node_token(&path, "n").unwrap().unwrap();
        persist_batch(&mut connection, &[event]).unwrap();
        assert_eq!(load_nodes(&connection).unwrap()[0].last_seen, 0);
        let fresh = load_nodes(&connection).unwrap().remove(0);
        revoke_node(&path, "n").unwrap();
        persist_batch(
            &mut connection,
            &[PersistEvent {
                node: fresh,
                write_sample: true,
                latency: None,
            }],
        )
        .unwrap();
        assert!(load_nodes(&connection).unwrap().is_empty());
    }

    #[test]
    fn offline_month_and_day_rollover_do_not_reuse_old_totals() {
        let (_dir, _path, mut node) = fixture();
        node.month_key = "2026-01".into();
        node.day_key = "2026-01-31".into();
        node.month_rx = 100;
        node.day_rx = 10;
        node.total_rx = 1000;
        let at = chrono::DateTime::parse_from_rfc3339("2026-02-01T00:00:00Z")
            .unwrap()
            .timestamp();
        let view = node.as_view(at);
        assert_eq!((view.month_rx, view.day_rx, view.total_rx), (0, 0, 1000));
        assert!(!view.online);
    }

    #[test]
    fn migration_preserves_v1_state_and_rejects_future_schema() {
        let (_dir, path, mut node) = fixture();
        node.total_rx = 99;
        let mut connection = open(&path).unwrap();
        persist_batch(
            &mut connection,
            &[PersistEvent {
                node,
                write_sample: true,
                latency: None,
            }],
        )
        .unwrap();
        connection
            .execute_batch("DROP TABLE latency_samples; PRAGMA user_version=1;")
            .unwrap();
        drop(connection);
        let connection = open(&path).unwrap();
        assert_eq!(load_nodes(&connection).unwrap()[0].total_rx, 99);
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM samples", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        connection.execute_batch("PRAGMA user_version=99;").unwrap();
        drop(connection);
        assert!(open(&path).is_err());
    }

    #[test]
    fn history_has_hard_range_limits_and_prunes_both_tables() {
        let (_dir, path, node) = fixture();
        let mut connection = open(&path).unwrap();
        let sample = LatencySample {
            id: 1,
            revision: 0,
            values: Latency::default(),
        };
        persist_batch(
            &mut connection,
            &[PersistEvent {
                node: node.clone(),
                write_sample: true,
                latency: Some(sample),
            }],
        )
        .unwrap();
        assert!(history(&path, "n", 721, false, node.last_seen + 1).is_err());
        assert!(history(&path, "n", 0, false, node.last_seen + 1).is_err());
        assert!(history(&path, "missing", 1, false, node.last_seen + 1).is_err());
        assert_eq!(
            prune_history(&connection, node.last_seen + 31 * 86400).unwrap(),
            2
        );
    }

    #[test]
    fn token_has_node_prefix_and_valid_hash() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.db");
        let token = create_node(&path, "hk-1", "Hong Kong").unwrap();
        assert!(token.starts_with("hk-1."));
        let nodes = load_nodes(&open(&path).unwrap()).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].token_hash, token_hash(&token));
    }
}
