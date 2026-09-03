use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
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
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS nodes (
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

         CREATE TABLE IF NOT EXISTS settings (
             key    TEXT PRIMARY KEY,
             value  TEXT NOT NULL
         ) WITHOUT ROWID;

         PRAGMA optimize;
         PRAGMA user_version=1;",
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
    Ok(AppSettings { latency, site })
}

pub fn save_latency(path: &Path, targets: &LatencyTargets) -> Result<()> {
    save_setting(path, "latency_targets", &serde_json::to_string(targets)?)
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
    let connection = open(path)?;
    connection.execute(
        "INSERT INTO nodes (id, name, token_hash, enabled, created_at)
         VALUES (?1, ?2, ?3, 1, unixepoch())
         ON CONFLICT(id) DO UPDATE SET
             name=excluded.name,
             token_hash=excluded.token_hash,
             enabled=1",
        params![id, name, hash],
    )?;
    connection.execute(
        "INSERT INTO node_state (node_id) VALUES (?1)
         ON CONFLICT(node_id) DO NOTHING",
        [id],
    )?;
    Ok(token)
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
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn persist_batch(connection: &mut Connection, events: &[PersistEvent]) -> Result<()> {
    let transaction = connection.transaction()?;
    for event in events {
        let node = &event.node;
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
    }
    transaction.commit()?;
    Ok(())
}

pub fn prune_history(connection: &Connection, now: i64) -> Result<usize> {
    let oldest_minute = now.div_euclid(60) - 30 * 24 * 60;
    Ok(connection.execute("DELETE FROM samples WHERE minute < ?1", [oldest_minute])?)
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
