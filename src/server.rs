use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{DefaultBodyLimit, Path, Query, State, WebSocketUpgrade};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio::sync::{broadcast, mpsc, watch, Mutex, RwLock, Semaphore};
use tokio::time::{interval, timeout, MissedTickBehavior};
use tracing::{error, info, warn};

use crate::db;
use crate::model::{
    network_delta, valid_target, AgentReport, AppSettings, HealthResponse, LatencyTargets,
    NodeView, PersistEvent, ServerMessage, SiteSettings, StatusResponse, StoredNode,
    PROTOCOL_VERSION,
};

#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub listen: SocketAddr,
    pub database: PathBuf,
}

#[derive(Clone)]
struct AppState {
    nodes: Arc<RwLock<HashMap<String, StoredNode>>>,
    snapshot: Arc<RwLock<Arc<str>>>,
    broadcast: broadcast::Sender<Arc<str>>,
    agent_config: watch::Sender<(LatencyTargets, u64)>,
    reports: mpsc::Sender<IncomingReport>,
    database: PathBuf,
    auth: Arc<Mutex<crate::auth::AdminAuth>>,
    settings: Arc<RwLock<AppSettings>>,
    shutdown: watch::Receiver<bool>,
    history_slots: Arc<Semaphore>,
}

#[derive(Debug)]
struct IncomingReport {
    node_id: String,
    connection_id: u64,
    token_hash: Vec<u8>,
    report: AgentReport,
}

pub async fn run(options: ServerOptions) -> Result<()> {
    let connection = db::open(&options.database)?;
    let admin_hash = db::admin_hash(&connection)
        .context("请在交互终端执行 monitor-server admin init --db <数据库路径> 初始化管理员")?;
    let loaded_settings = db::load_settings(&connection)?;
    let stored = db::load_nodes(&connection)?;
    drop(connection);

    let node_count = stored.len();
    let nodes = Arc::new(RwLock::new(
        stored
            .into_iter()
            .map(|node| (node.id.clone(), node))
            .collect::<HashMap<_, _>>(),
    ));
    let settings = Arc::new(RwLock::new(loaded_settings));
    let initial_site = settings.read().await.site.clone();
    let initial = {
        let guard = nodes.read().await;
        render_snapshot(&guard, &initial_site)?
    };
    let snapshot = Arc::new(RwLock::new(initial));
    let (broadcast, _) = broadcast::channel::<Arc<str>>(16);
    let (agent_config, _) = watch::channel({
        let settings = settings.read().await;
        (settings.latency.clone(), settings.latency_revision)
    });
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (reports_tx, reports_rx) = mpsc::channel::<IncomingReport>(4096);
    let (persist_tx, database_worker) =
        start_database_writer(options.database.clone(), shutdown_tx.clone())?;

    let report_worker = tokio::spawn(process_reports(
        nodes.clone(),
        settings.clone(),
        reports_rx,
        persist_tx,
    ));
    let snapshot_worker = tokio::spawn(refresh_snapshots(
        nodes.clone(),
        snapshot.clone(),
        broadcast.clone(),
        settings.clone(),
        shutdown_rx.clone(),
    ));

    let state = AppState {
        nodes,
        snapshot,
        broadcast,
        agent_config,
        reports: reports_tx,
        database: options.database.clone(),
        auth: Arc::new(Mutex::new(crate::auth::AdminAuth::new(admin_hash)?)),
        settings,
        shutdown: shutdown_rx.clone(),
        history_slots: Arc::new(Semaphore::new(4)),
    };
    let app = Router::new()
        .route("/", get(index))
        .route("/node/{id}", get(index))
        .route("/admin", get(admin))
        .route("/assets/app.css", get(styles))
        .route("/assets/app.js", get(script))
        .route("/assets/admin.js", get(admin_script))
        .route("/assets/navigation.js", get(navigation_script))
        .route("/assets/theme.js", get(theme_script))
        .route("/favicon.svg", get(favicon))
        .route("/api/health", get(health))
        .route("/api/nodes", get(nodes_api))
        .route("/api/nodes/{id}/history", get(history_api))
        .route("/api/ws", get(browser_socket))
        .route("/api/agent", get(agent_socket))
        .route("/api/admin/login", post(admin_login))
        .route("/api/admin/logout", post(admin_logout))
        .route("/api/admin/password", put(admin_change_password))
        .route("/api/admin/state", get(admin_state))
        .route("/api/admin/nodes", post(admin_create_node))
        .route(
            "/api/admin/nodes/{id}",
            get(admin_node).delete(admin_revoke_node),
        )
        .route("/api/admin/nodes/{id}/token", post(admin_rotate_token))
        .route("/api/admin/latency", put(admin_save_latency))
        .route("/api/admin/site", put(admin_save_site))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(options.listen)
        .await
        .with_context(|| format!("无法监听 {}", options.listen))?;
    info!(listen = %options.listen, database = %options.database.display(), node_count, "控制端已启动");
    let mut stop = shutdown_rx;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! { _ = shutdown_signal() => {}, _ = stop.changed() => {} }
            let _ = shutdown_tx.send(true);
        })
        .await?;
    report_worker.await.context("上报处理线程退出失败")?;
    snapshot_worker.await.context("快照线程退出失败")?;
    tokio::task::spawn_blocking(move || {
        database_worker
            .join()
            .map_err(|_| anyhow::anyhow!("数据库线程异常退出"))
    })
    .await???;
    Ok(())
}

fn start_database_writer(
    path: PathBuf,
    shutdown: watch::Sender<bool>,
) -> Result<(mpsc::Sender<PersistEvent>, thread::JoinHandle<Result<()>>)> {
    let (sender, mut receiver) = mpsc::channel::<PersistEvent>(8192);
    let mut connection = db::open(&path)?;
    let worker = thread::Builder::new()
        .name("monitor-db".to_owned())
        .spawn(move || {
            let mut last_prune = 0_i64;
            while let Some(first) = receiver.blocking_recv() {
                let mut batch = Vec::with_capacity(256);
                batch.push(first);
                while batch.len() < 512 {
                    match receiver.try_recv() {
                        Ok(event) => batch.push(event),
                        Err(_) => break,
                    }
                }
                let mut stopping_since = None;
                loop {
                    match db::persist_batch(&mut connection, &batch) {
                        Ok(()) => break,
                        Err(error) => {
                            error!(%error, count = batch.len(), "写入失败，保留原批次重试；队列满时暂停接收");
                            if *shutdown.borrow() {
                                let since = stopping_since.get_or_insert_with(Instant::now);
                                if since.elapsed() >= Duration::from_secs(20) { return Err(error.context("停止前仍无法落盘，存在未持久化数据")); }
                            }
                            thread::sleep(Duration::from_secs(1));
                        }
                    }
                }
                let now = Utc::now().timestamp();
                if now - last_prune >= 3600 {
                    if let Err(error) = db::prune_history(&connection, now) {
                        warn!(%error, "清理历史数据失败");
                    }
                    last_prune = now;
                }
            }
            Ok(())
        })
        .context("无法启动数据库线程")?;
    Ok((sender, worker))
}

async fn process_reports(
    nodes: Arc<RwLock<HashMap<String, StoredNode>>>,
    settings: Arc<RwLock<AppSettings>>,
    mut receiver: mpsc::Receiver<IncomingReport>,
    persist: mpsc::Sender<PersistEvent>,
) {
    while let Some(incoming) = receiver.recv().await {
        let now = Utc::now();
        let timestamp = now.timestamp();
        let month_key = now.format("%Y-%m").to_string();
        let day_key = now.format("%Y-%m-%d").to_string();
        let revision = settings.read().await.latency_revision;
        let mut guard = nodes.write().await;
        let Some(node) = guard.get_mut(&incoming.node_id) else {
            continue;
        };
        if incoming.connection_id != node.connection_id || incoming.token_hash != node.token_hash {
            continue;
        }
        let mut report = incoming.report;
        let (delta_rx, delta_tx) = report_traffic_delta(node, &report);
        let previous = node.metrics.as_ref();
        let latency = report
            .metrics
            .latency_sample
            .as_ref()
            .filter(|sample| {
                sample.revision == revision
                    && previous
                        .and_then(|m| m.latency_sample.as_ref())
                        .is_none_or(|old| old.id != sample.id)
            })
            .cloned();
        if let Some(sample) = &latency {
            report.metrics.latency = sample.values.clone();
            report.metrics.latency_at = Some(timestamp);
        } else {
            // Repeated cached samples and in-flight results from old targets are not new history.
            report.metrics.latency_sample = previous.and_then(|m| m.latency_sample.clone());
            report.metrics.latency_at = previous.and_then(|m| m.latency_at);
            if let Some(previous) = previous {
                report.metrics.latency = previous.latency.clone();
            }
        }

        if node.month_key != month_key {
            node.month_key = month_key;
            node.month_rx = 0;
            node.month_tx = 0;
        }
        if node.day_key != day_key {
            node.day_key = day_key;
            node.day_rx = 0;
            node.day_tx = 0;
        }
        node.total_rx = node.total_rx.saturating_add(delta_rx);
        node.total_tx = node.total_tx.saturating_add(delta_tx);
        node.month_rx = node.month_rx.saturating_add(delta_rx);
        node.month_tx = node.month_tx.saturating_add(delta_tx);
        node.day_rx = node.day_rx.saturating_add(delta_rx);
        node.day_tx = node.day_tx.saturating_add(delta_tx);
        node.last_rx_counter = report.metrics.net_rx_total;
        node.last_tx_counter = report.metrics.net_tx_total;
        node.boot_id = report.boot_id;
        node.last_seen = timestamp;
        node.agent_version = report.agent_version;
        node.hostname = report.hostname;
        node.os = report.os;
        node.kernel = report.kernel;
        node.arch = report.arch;
        node.virtualization = report.virtualization;
        node.cpu_name = report.cpu_name;
        node.cpu_cores = report.cpu_cores;
        node.mem_total = report.mem_total;
        node.swap_total = report.swap_total;
        node.disk_total = report.disk_total;
        node.metrics = Some(report.metrics);

        let minute = timestamp.div_euclid(60);
        let write_sample = minute > node.last_sample_minute;
        if write_sample {
            node.last_sample_minute = minute;
        }
        let event = PersistEvent {
            node: node.clone(),
            write_sample,
            latency,
        };
        drop(guard);

        if persist.send(event).await.is_err() {
            error!("数据库线程已经停止，停止接收上报");
            return;
        }
    }
}

fn report_traffic_delta(node: &StoredNode, report: &AgentReport) -> (u64, u64) {
    if node.boot_id.is_empty() {
        return (0, 0);
    }
    if node.boot_id == report.boot_id && !report.metrics.network.is_empty() {
        return network_delta(
            node.metrics
                .as_ref()
                .map(|m| m.network.as_slice())
                .unwrap_or_default(),
            &report.metrics.network,
        );
    }
    (
        traffic_delta(
            &node.boot_id,
            node.last_rx_counter,
            &report.boot_id,
            report.metrics.net_rx_total,
        ),
        traffic_delta(
            &node.boot_id,
            node.last_tx_counter,
            &report.boot_id,
            report.metrics.net_tx_total,
        ),
    )
}

fn traffic_delta(previous_boot: &str, previous: u64, boot: &str, current: u64) -> u64 {
    if previous_boot.is_empty() {
        0
    } else if previous_boot == boot {
        current.saturating_sub(previous)
    } else {
        current
    }
}

async fn refresh_snapshots(
    nodes: Arc<RwLock<HashMap<String, StoredNode>>>,
    snapshot: Arc<RwLock<Arc<str>>>,
    sender: broadcast::Sender<Arc<str>>,
    settings: Arc<RwLock<AppSettings>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut ticker = interval(Duration::from_secs(2));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! { _ = ticker.tick() => {}, _ = shutdown.changed() => return }
        let site = settings.read().await.site.clone();
        let rendered = {
            let guard = nodes.read().await;
            render_snapshot(&guard, &site)
        };
        match rendered {
            Ok(rendered) => {
                *snapshot.write().await = rendered.clone();
                let _ = sender.send(rendered);
            }
            Err(error) => error!(%error, "生成状态快照失败"),
        }
    }
}

fn render_snapshot(nodes: &HashMap<String, StoredNode>, site: &SiteSettings) -> Result<Arc<str>> {
    let now = Utc::now().timestamp();
    let mut views = nodes
        .values()
        .map(|node| node.as_view(now))
        .collect::<Vec<_>>();
    views.sort_by(|left, right| left.id.cmp(&right.id));
    let response = StatusResponse {
        generated_at: now,
        site: site.clone(),
        nodes: views,
    };
    Ok(Arc::from(serde_json::to_string(&response)?))
}

async fn agent_socket(
    State(state): State<AppState>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return plain(StatusCode::UNAUTHORIZED, "missing bearer token");
    };
    let Some((node_id, _)) = token.split_once('.') else {
        return plain(StatusCode::UNAUTHORIZED, "invalid bearer token");
    };
    if token.len() > 160 || node_id.len() > 64 {
        return plain(StatusCode::UNAUTHORIZED, "invalid bearer token");
    }
    let hash = db::token_hash(token);
    let allowed = state
        .nodes
        .read()
        .await
        .get(node_id)
        .is_some_and(|node| bool::from(hash.as_slice().ct_eq(node.token_hash.as_slice())));
    if !allowed {
        return plain(StatusCode::UNAUTHORIZED, "invalid bearer token");
    }
    let node_id = node_id.to_owned();
    websocket
        .read_buffer_size(4 * 1024)
        .write_buffer_size(4 * 1024)
        .max_write_buffer_size(64 * 1024)
        .max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .on_upgrade(move |socket| handle_agent(socket, state, node_id, hash))
}

static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

async fn handle_agent(socket: WebSocket, state: AppState, node_id: String, token_hash: Vec<u8>) {
    let connection_id = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
    let (close_tx, close_rx) = watch::channel(false);
    {
        let mut nodes = state.nodes.write().await;
        let Some(node) = nodes.get_mut(&node_id) else {
            return;
        };
        // Recheck after the upgrade: revocation may have happened since the handshake.
        if node.token_hash != token_hash || *state.shutdown.borrow() {
            return;
        }
        if let Some(old) = node.close_signal.replace(close_tx) {
            let _ = old.send(true);
        }
        node.connection_id = connection_id;
    }
    if let Err(error) = agent_loop(
        socket,
        &state,
        &node_id,
        connection_id,
        &token_hash,
        close_rx,
    )
    .await
    {
        tracing::debug!(%error, %node_id, "探针连接结束");
    }
    let mut nodes = state.nodes.write().await;
    if let Some(node) = nodes
        .get_mut(&node_id)
        .filter(|node| node.connection_id == connection_id)
    {
        node.close_signal = None;
    }
}

async fn agent_loop(
    socket: WebSocket,
    state: &AppState,
    node_id: &str,
    connection_id: u64,
    token_hash: &[u8],
    mut close: watch::Receiver<bool>,
) -> Result<()> {
    let (mut writer, mut reader) = socket.split();
    let mut config = state.agent_config.subscribe();
    let initial = config.borrow_and_update().clone();
    send_targets(&mut writer, initial).await?;
    let mut shutdown = state.shutdown.clone();
    let mut last_report = Instant::now() - Duration::from_secs(1);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = close.changed() => break,
            incoming = timeout(Duration::from_secs(15), reader.next()) => {
                let incoming = incoming.context("探针上报超时")?;
                let report = match incoming {
                    Some(Ok(Message::Text(text))) => serde_json::from_str::<AgentReport>(&text)?,
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    _ => continue,
                };
                if !valid_report(&report) || last_report.elapsed() < Duration::from_millis(200) { break; }
                last_report = Instant::now();
                let incoming = IncomingReport { node_id: node_id.to_owned(), connection_id, token_hash: token_hash.to_vec(), report };
                tokio::select! {
                    _ = shutdown.changed() => break,
                    _ = close.changed() => break,
                    sent = state.reports.send(incoming) => if sent.is_err() { break; },
                }
            }
            update = config.changed() => {
                if update.is_err() { break; }
                let updated = config.borrow_and_update().clone();
                send_targets(&mut writer, updated).await?;
            }
        }
    }
    let _ = timeout(Duration::from_secs(1), writer.close()).await;
    Ok(())
}

async fn send_targets(
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    (targets, revision): (LatencyTargets, u64),
) -> Result<()> {
    let payload = serde_json::to_string(&ServerMessage::LatencyTargets { targets, revision })?;
    timeout(
        Duration::from_secs(5),
        writer.send(Message::Text(payload.into())),
    )
    .await??;
    Ok(())
}

fn valid_report(report: &AgentReport) -> bool {
    report.protocol == PROTOCOL_VERSION
        && report.agent_version.len() <= 32
        && !report.boot_id.is_empty()
        && report.boot_id.len() <= 64
        && report.hostname.len() <= 255
        && report.os.len() <= 255
        && report.kernel.len() <= 128
        && report.arch.len() <= 32
        && report.virtualization.len() <= 32
        && report.cpu_name.len() <= 255
        && report.cpu_cores <= 4096
        && report.metrics.cpu.is_finite()
        && (0.0..=100.0).contains(&report.metrics.cpu)
        && report
            .metrics
            .load
            .iter()
            .all(|value| value.is_finite() && (0.0..=100_000.0).contains(value))
        && report.metrics.network.len() <= 64
        && report.metrics.network.iter().all(|nic| {
            !nic.name.is_empty()
                && nic.name.len() <= 32
                && nic.rx <= i64::MAX as u64
                && nic.tx <= i64::MAX as u64
        })
        && report.metrics.latency_sample.as_ref().is_none_or(|sample| {
            sample.id > 0
                && sample.id <= i64::MAX as u64
                && sample.revision <= i64::MAX as u64
                && sample
                    .interval_seconds
                    .is_none_or(crate::model::valid_latency_interval)
                && [
                    sample.values.telecom,
                    sample.values.unicom,
                    sample.values.mobile,
                ]
                .into_iter()
                .flatten()
                .all(|value| value.is_finite() && (0.0..=60_000.0).contains(&value))
        })
        && [
            report.metrics.latency.telecom,
            report.metrics.latency.unicom,
            report.metrics.latency.mobile,
        ]
        .into_iter()
        .flatten()
        .all(|value| value.is_finite() && (0.0..=60_000.0).contains(&value))
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    password: String,
}

#[derive(Debug, Serialize)]
struct LoginResponse {
    token: String,
}

#[derive(Debug, Deserialize)]
struct CreateNodeRequest {
    id: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct CreateNodeResponse {
    id: String,
    name: String,
    token: String,
}

#[derive(Debug, Serialize)]
struct AdminStateResponse {
    settings: AppSettings,
    nodes: Vec<NodeView>,
}

#[derive(Debug, Serialize)]
struct AdminNodeResponse {
    #[serde(flatten)]
    node: NodeView,
    token: Option<String>,
    token_status: &'static str,
}

async fn admin_login(State(state): State<AppState>, Json(request): Json<LoginRequest>) -> Response {
    if request.password.len() > 512 {
        return json_error(StatusCode::UNAUTHORIZED, "管理员密码错误");
    }
    // Keep the owned guard in the worker: disconnecting clients cannot bypass
    // the single concurrent password verification or race a password change.
    let Ok(mut auth) = state.auth.clone().try_lock_owned() else {
        return auth_throttled();
    };
    if !auth.allow_attempt(Instant::now()) {
        return auth_throttled();
    }
    let Ok((mut auth, allowed)) = tokio::task::spawn_blocking(move || {
        let allowed = crate::auth::verify_password(&auth.credential, &request.password);
        (auth, allowed)
    })
    .await
    else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "登录暂不可用");
    };
    if !allowed {
        return json_error(StatusCode::UNAUTHORIZED, "管理员密码错误");
    }
    let token = random_session();
    let sessions = &mut auth.sessions;
    sessions.retain(|_, expiry| *expiry > Instant::now());
    if sessions.len() >= 128 {
        return json_error(StatusCode::TOO_MANY_REQUESTS, "会话过多，请先退出其他会话");
    }
    sessions.insert(
        token.clone(),
        Instant::now() + Duration::from_secs(12 * 3600),
    );
    json_value(StatusCode::OK, &LoginResponse { token })
}

async fn admin_logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = bearer_token(&headers) {
        state.auth.lock().await.sessions.remove(token);
    }
    empty(StatusCode::NO_CONTENT)
}

fn auth_throttled() -> Response {
    let mut response = json_error(StatusCode::TOO_MANY_REQUESTS, "尝试过于频繁，请稍后重试");
    response
        .headers_mut()
        .insert("retry-after", HeaderValue::from_static("60"));
    response
}

#[derive(Deserialize)]
struct PasswordChange {
    current_password: String,
    new_password: String,
    confirm_password: String,
}

async fn admin_change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PasswordChange>,
) -> Response {
    let Some(token) = bearer_token(&headers).filter(|token| token.len() <= 128) else {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    };
    let Ok(mut auth) = state.auth.clone().try_lock_owned() else {
        return auth_throttled();
    };
    if !auth
        .sessions
        .get(token)
        .is_some_and(|expiry| *expiry > Instant::now())
    {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    }
    if !auth.allow_attempt(Instant::now()) {
        return auth_throttled();
    }
    if request.current_password.len() > 512
        || !crate::auth::valid_password(&request.new_password)
        || request.new_password != request.confirm_password
        || request.current_password == request.new_password
    {
        return json_error(
            StatusCode::BAD_REQUEST,
            "新密码需为 15–128 个字符、两次输入一致且不同于原密码",
        );
    }
    let path = state.database.clone();
    // Persist and revoke under the same lock, including if the HTTP request is
    // cancelled after the write. No old-password login can create a new session.
    match tokio::task::spawn_blocking(move || -> Result<u8> {
        if !crate::auth::verify_password(&auth.credential, &request.current_password) {
            return Ok(1);
        }
        let encoded = crate::auth::hash_password(&request.new_password)?;
        if !db::change_admin_password(&path, &auth.credential, &encoded)? {
            return Ok(2);
        }
        auth.credential = encoded;
        auth.sessions.clear();
        Ok(0)
    })
    .await
    {
        Ok(Ok(0)) => empty(StatusCode::NO_CONTENT),
        Ok(Ok(1)) => json_error(StatusCode::BAD_REQUEST, "当前密码错误"),
        Ok(Ok(_)) => json_error(
            StatusCode::CONFLICT,
            "密码已在其他位置更改，请重启主控后重新登录",
        ),
        _ => json_error(StatusCode::INTERNAL_SERVER_ERROR, "修改密码失败"),
    }
}

async fn admin_state(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_admin(&state, &headers).await {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    }
    let now = Utc::now().timestamp();
    let mut nodes = state
        .nodes
        .read()
        .await
        .values()
        .map(|node| node.as_view(now))
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    let settings = state.settings.read().await.clone();
    json_value(StatusCode::OK, &AdminStateResponse { settings, nodes })
}

// Once authorized, finish both the database and in-memory update even when the
// HTTP client disconnects. Dropping a spawn_blocking handle cannot cancel its
// database write; dropping only the surrounding handler would leave stale keys.
async fn finish_mutation(
    operation: impl std::future::Future<Output = Response> + Send + 'static,
) -> Response {
    tokio::spawn(operation).await.unwrap_or_else(|_| {
        json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "保存失败，请重新查询当前状态",
        )
    })
}

async fn admin_create_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateNodeRequest>,
) -> Response {
    if !is_admin(&state, &headers).await {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    }
    finish_mutation(async move {
        let mut nodes = state.nodes.write().await;
        let path = state.database.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<(String, StoredNode)> {
            let token = db::create_node(&path, &request.id, &request.name)?;
            let node = db::load_nodes(&db::open(&path)?)?
                .into_iter()
                .find(|node| node.id == request.id)
                .context("创建后的节点不存在")?;
            Ok((token, node))
        })
        .await;
        match result {
            Ok(Ok((token, node))) => {
                let response = CreateNodeResponse {
                    id: node.id.clone(),
                    name: node.name.clone(),
                    token,
                };
                nodes.insert(node.id.clone(), node);
                json_value(StatusCode::CREATED, &response)
            }
            Ok(Err(error)) => json_error(StatusCode::BAD_REQUEST, &error.to_string()),
            Err(_) => json_error(StatusCode::INTERNAL_SERVER_ERROR, "创建节点失败"),
        }
    })
    .await
}

async fn admin_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !is_admin(&state, &headers).await {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    }
    let nodes = state.nodes.read().await;
    let Some(node) = nodes.get(&id) else {
        return json_error(StatusCode::NOT_FOUND, "节点不存在或已停用");
    };
    // All released schemas store only token_hash. Never expose it as a credential
    // or invent a plaintext fallback. Creation/rotation return the token once.
    json_value(
        StatusCode::OK,
        &AdminNodeResponse {
            node: node.as_view(Utc::now().timestamp()),
            token: None,
            token_status: "hash_only",
        },
    )
}

async fn admin_revoke_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !is_admin(&state, &headers).await {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    }
    finish_mutation(async move {
        let mut nodes = state.nodes.write().await;
        let path = state.database.clone();
        let key = id.clone();
        match tokio::task::spawn_blocking(move || db::revoke_node(&path, &key)).await {
            Ok(Ok(true)) => {
                if let Some(node) = nodes.remove(&id) {
                    if let Some(close) = node.close_signal {
                        let _ = close.send(true);
                    }
                }
                empty(StatusCode::NO_CONTENT)
            }
            Ok(Ok(false)) => json_error(StatusCode::NOT_FOUND, "节点不存在"),
            _ => json_error(StatusCode::INTERNAL_SERVER_ERROR, "停用节点失败"),
        }
    })
    .await
}

async fn admin_rotate_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !is_admin(&state, &headers).await {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    }
    finish_mutation(async move {
        let mut nodes = state.nodes.write().await;
        let Some(node) = nodes.get_mut(&id) else {
            return json_error(StatusCode::NOT_FOUND, "节点不存在");
        };
        let path = state.database.clone();
        let key = id.clone();
        match tokio::task::spawn_blocking(move || db::rotate_node_token(&path, &key)).await {
            Ok(Ok(Some(token))) => {
                if let Some(close) = node.close_signal.take() {
                    let _ = close.send(true);
                }
                node.connection_id = 0;
                node.token_hash = db::token_hash(&token);
                json_value(
                    StatusCode::OK,
                    &CreateNodeResponse {
                        id,
                        name: node.name.clone(),
                        token,
                    },
                )
            }
            _ => json_error(StatusCode::INTERNAL_SERVER_ERROR, "重置密钥失败"),
        }
    })
    .await
}

#[derive(Deserialize)]
struct LatencyUpdate {
    telecom: String,
    unicom: String,
    mobile: String,
    interval_seconds: Option<u64>,
}

async fn admin_save_latency(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LatencyUpdate>,
) -> Response {
    if !is_admin(&state, &headers).await {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    }
    finish_mutation(async move {
        let mut settings = state.settings.write().await;
        let targets = LatencyTargets {
            telecom: request.telecom,
            unicom: request.unicom,
            mobile: request.mobile,
            interval_seconds: request
                .interval_seconds
                .unwrap_or(settings.latency.interval_seconds),
        };
        if !crate::model::valid_latency_interval(targets.interval_seconds) {
            return json_error(StatusCode::BAD_REQUEST, "检测间隔必须为 10–3600 秒");
        }
        if !valid_targets(&targets) {
            return json_error(StatusCode::BAD_REQUEST, "地址必须为有效的 host:port");
        }
        let path = state.database.clone();
        let values = targets.clone();
        let revision =
            match tokio::task::spawn_blocking(move || db::save_latency(&path, &values)).await {
                Ok(Ok(revision)) => revision,
                _ => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "保存延迟设置失败"),
            };
        settings.latency = targets.clone();
        settings.latency_revision = revision;
        state.agent_config.send_replace((targets.clone(), revision));
        json_value(StatusCode::OK, &targets)
    })
    .await
}

async fn admin_save_site(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(site): Json<SiteSettings>,
) -> Response {
    if !is_admin(&state, &headers).await {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    }
    finish_mutation(async move {
        let site = SiteSettings {
            name: site.name.trim().to_owned(),
            description: site.description.trim().to_owned(),
            footer: site.footer.trim().to_owned(),
        };
        if site.name.is_empty()
            || site.name.chars().count() > 40
            || site.description.chars().count() > 120
            || site.footer.chars().count() > 120
        {
            return json_error(StatusCode::BAD_REQUEST, "站点文字超出长度限制");
        }
        let mut settings = state.settings.write().await;
        let path = state.database.clone();
        let value = site.clone();
        if !matches!(
            tokio::task::spawn_blocking(move || db::save_site(&path, &value)).await,
            Ok(Ok(()))
        ) {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "保存站点设置失败");
        }
        settings.site = site.clone();
        json_value(StatusCode::OK, &site)
    })
    .await
}

fn valid_targets(targets: &LatencyTargets) -> bool {
    [&targets.telecom, &targets.unicom, &targets.mobile]
        .into_iter()
        .all(|target| valid_target(target))
}

async fn is_admin(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = bearer_token(headers) else {
        return false;
    };
    if token.len() > 128 {
        return false;
    }
    let now = Instant::now();
    let mut auth = state.auth.lock().await;
    let sessions = &mut auth.sessions;
    sessions.retain(|_, expiry| *expiry > now);
    sessions.get(token).is_some_and(|expiry| *expiry > now)
}

fn random_session() -> String {
    let mut secret = [0_u8; 32];
    OsRng.fill_bytes(&mut secret);
    URL_SAFE_NO_PAD.encode(secret)
}

async fn browser_socket(State(state): State<AppState>, websocket: WebSocketUpgrade) -> Response {
    websocket
        .read_buffer_size(4 * 1024)
        .write_buffer_size(4 * 1024)
        .max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .on_upgrade(move |socket| handle_browser(socket, state))
}

async fn handle_browser(socket: WebSocket, state: AppState) {
    let (mut writer, mut reader) = socket.split();
    let initial = state.snapshot.read().await.clone();
    if !matches!(
        timeout(
            Duration::from_secs(5),
            writer.send(Message::Text(initial.to_string().into()))
        )
        .await,
        Ok(Ok(()))
    ) {
        return;
    }
    let mut receiver = state.broadcast.subscribe();
    let mut shutdown = state.shutdown.clone();
    loop {
        tokio::select! {
            _ = shutdown.changed() => return,
            update = receiver.recv() => match update {
                Ok(payload) => {
                    if !matches!(timeout(Duration::from_secs(5), writer.send(Message::Text(payload.to_string().into()))).await, Ok(Ok(()))) {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            },
            incoming = reader.next() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            }
        }
    }
}

async fn nodes_api(State(state): State<AppState>) -> Response {
    json_response(state.snapshot.read().await.to_string())
}

#[derive(Deserialize)]
struct HistoryQuery {
    hours: Option<u32>,
    kind: Option<String>,
}

async fn history_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Response {
    let hours = query.hours.unwrap_or(6);
    let kind = query.kind.as_deref().unwrap_or("resources");
    if !(1..=720).contains(&hours) || !matches!(kind, "resources" | "latency") {
        return json_error(
            StatusCode::BAD_REQUEST,
            "历史范围为 1–720 小时，类型为 resources 或 latency",
        );
    }
    if !state.nodes.read().await.contains_key(&id) {
        return json_error(StatusCode::NOT_FOUND, "节点不存在或已停用");
    }
    let Ok(permit) = state.history_slots.clone().try_acquire_owned() else {
        return json_error(StatusCode::TOO_MANY_REQUESTS, "历史查询繁忙，请稍后重试");
    };
    let path = state.database.clone();
    let latency = kind == "latency";
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        db::history(&path, &id, hours, latency, Utc::now().timestamp())
    })
    .await
    {
        Ok(Ok(history)) => json_value(StatusCode::OK, &history),
        _ => json_error(StatusCode::INTERNAL_SERVER_ERROR, "历史读取失败"),
    }
}

async fn health() -> impl IntoResponse {
    axum::Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn index() -> Response {
    static_response("text/html; charset=utf-8", include_str!("ui/index.html"))
}

async fn admin() -> Response {
    static_response("text/html; charset=utf-8", include_str!("ui/admin.html"))
}

async fn styles() -> Response {
    static_response(
        "text/css; charset=utf-8",
        concat!(
            include_str!("ui/app.css"),
            "\n",
            include_str!("ui/os-icons.css")
        ),
    )
}

async fn script() -> Response {
    static_response("text/javascript; charset=utf-8", include_str!("ui/app.js"))
}

async fn admin_script() -> Response {
    static_response(
        "text/javascript; charset=utf-8",
        include_str!("ui/admin.js"),
    )
}

async fn navigation_script() -> Response {
    static_response(
        "text/javascript; charset=utf-8",
        include_str!("ui/navigation.js"),
    )
}

async fn theme_script() -> Response {
    static_response(
        "text/javascript; charset=utf-8",
        include_str!("ui/theme.js"),
    )
}

async fn favicon() -> Response {
    static_response(
        "image/svg+xml",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="8" fill="#111"/><path d="M8 17h4l2-7 4 13 2-6h4" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##,
    )
}

fn static_response(content_type: &'static str, content: &'static str) -> Response {
    let mut response = Response::new(Body::from(content));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    security_headers(response.headers_mut());
    response
}

fn json_response(content: String) -> Response {
    let mut response = Response::new(Body::from(content));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    security_headers(response.headers_mut());
    response
}

fn json_value<T: Serialize>(status: StatusCode, value: &T) -> Response {
    match serde_json::to_string(value) {
        Ok(content) => {
            let mut response = json_response(content);
            *response.status_mut() = status;
            response
        }
        Err(_) => json_error(StatusCode::INTERNAL_SERVER_ERROR, "序列化响应失败"),
    }
}

fn json_error(status: StatusCode, message: &str) -> Response {
    let content = serde_json::json!({ "error": message }).to_string();
    let mut response = json_response(content);
    *response.status_mut() = status;
    response
}

fn empty(status: StatusCode) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    security_headers(response.headers_mut());
    response
}

fn plain(status: StatusCode, message: &'static str) -> Response {
    let mut response = Response::new(Body::from(message));
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    security_headers(response.headers_mut());
    response
}

fn security_headers(headers: &mut HeaderMap) {
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self' ws: wss:; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("安装 Ctrl-C 处理器失败");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("安装 SIGTERM 处理器失败")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Latency, Metrics};

    fn report() -> AgentReport {
        AgentReport {
            protocol: PROTOCOL_VERSION,
            agent_version: "0.1.0".into(),
            boot_id: "boot".into(),
            hostname: "host".into(),
            os: "Linux".into(),
            kernel: "6.1".into(),
            arch: "x86_64".into(),
            virtualization: "kvm".into(),
            cpu_name: "CPU".into(),
            cpu_cores: 1,
            mem_total: 1,
            swap_total: 0,
            disk_total: 1,
            metrics: Metrics {
                cpu: 5.0,
                latency: Latency::default(),
                ..Metrics::default()
            },
        }
    }

    #[test]
    fn rejects_invalid_cpu() {
        let mut value = report();
        assert!(valid_report(&value));
        value.metrics.cpu = 101.0;
        assert!(!valid_report(&value));
    }

    #[test]
    fn report_validation_bounds_untrusted_text_samples_and_nonfinite_numbers() {
        for cpu in [f32::NAN, f32::INFINITY, -1.0, 101.0] {
            let mut value = report();
            value.metrics.cpu = cpu;
            assert!(!valid_report(&value));
        }
        for delay in [f32::NAN, f32::NEG_INFINITY, -1.0, 60_001.0] {
            let mut value = report();
            value.metrics.latency.telecom = Some(delay);
            assert!(!valid_report(&value));
        }
        let mut value = report();
        value.hostname = "a".repeat(256);
        assert!(!valid_report(&value));
        value.hostname.clear();
        value.metrics.load[0] = f32::NAN;
        assert!(!valid_report(&value));
        value.metrics.load[0] = 0.0;
        value.metrics.latency_sample = Some(crate::model::LatencySample {
            id: 1,
            revision: 0,
            interval_seconds: Some(0),
            values: Latency::default(),
        });
        assert!(!valid_report(&value));
        value
            .metrics
            .latency_sample
            .as_mut()
            .unwrap()
            .interval_seconds = Some(30);
        assert!(valid_report(&value));
        value.metrics.latency_sample.as_mut().unwrap().id = u64::MAX;
        assert!(!valid_report(&value));
    }

    #[tokio::test]
    async fn cancelled_http_waiter_does_not_leave_old_key_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancel.db");
        let previous = db::create_node(&path, "n", "Node").unwrap();
        let node = db::load_nodes(&db::open(&path).unwrap()).unwrap().remove(0);
        let nodes = Arc::new(RwLock::new(HashMap::from([("n".to_owned(), node)])));
        let (written_tx, written_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let worker_nodes = nodes.clone();
        let worker_path = path.clone();
        let request = tokio::spawn(finish_mutation(async move {
            let mut guard = worker_nodes.write().await;
            let token = tokio::task::spawn_blocking(move || {
                db::rotate_node_token(&worker_path, "n").unwrap().unwrap()
            })
            .await
            .unwrap();
            written_tx.send(()).unwrap();
            resume_rx.await.unwrap();
            guard.get_mut("n").unwrap().token_hash = db::token_hash(&token);
            drop(guard);
            done_tx.send(()).unwrap();
            empty(StatusCode::NO_CONTENT)
        }));
        written_rx.await.unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        resume_tx.send(()).unwrap();
        timeout(Duration::from_secs(2), done_rx)
            .await
            .unwrap()
            .unwrap();
        let stored = db::load_nodes(&db::open(&path).unwrap()).unwrap().remove(0);
        let guard = nodes.read().await;
        assert_eq!(guard["n"].token_hash, stored.token_hash);
        assert_ne!(guard["n"].token_hash, db::token_hash(&previous));
    }

    #[test]
    fn traffic_uses_a_baseline_and_survives_reboots() {
        assert_eq!(traffic_delta("", 0, "boot-a", 50_000), 0);
        assert_eq!(traffic_delta("boot-a", 50_000, "boot-a", 51_250), 1_250);
        assert_eq!(traffic_delta("boot-a", 51_250, "boot-b", 400), 400);
        assert_eq!(traffic_delta("boot-b", 400, "boot-b", 100), 0);
    }

    #[tokio::test]
    async fn queued_old_connection_and_repeated_latency_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let token = db::create_node(&path, "n", "Node").unwrap();
        let mut node = db::load_nodes(&db::open(&path).unwrap()).unwrap().remove(0);
        node.connection_id = 2;
        let nodes = Arc::new(RwLock::new(HashMap::from([("n".into(), node)])));
        let (tx, rx) = mpsc::channel(4);
        let (persist, mut events) = mpsc::channel(4);
        let task = tokio::spawn(process_reports(
            nodes.clone(),
            Arc::new(RwLock::new(AppSettings::default())),
            rx,
            persist,
        ));
        for connection_id in [1, 2, 2] {
            let mut report = report();
            report.metrics.latency_sample = Some(crate::model::LatencySample {
                id: 1,
                revision: 0,
                interval_seconds: Some(30),
                values: Latency::default(),
            });
            tx.send(IncomingReport {
                node_id: "n".into(),
                connection_id,
                token_hash: db::token_hash(&token),
                report,
            })
            .await
            .unwrap();
        }
        drop(tx);
        task.await.unwrap();
        assert!(events.recv().await.unwrap().latency.is_some());
        assert!(events.recv().await.unwrap().latency.is_none());
        assert!(events.recv().await.is_none());
    }
}
