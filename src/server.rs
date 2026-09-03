use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{DefaultBodyLimit, Path, State, WebSocketUpgrade};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::time::{interval, MissedTickBehavior};
use tracing::{error, info, warn};

use crate::db;
use crate::model::{
    AgentReport, AppSettings, HealthResponse, LatencyTargets, NodeView, PersistEvent,
    ServerMessage, SiteSettings, StatusResponse, StoredNode, PROTOCOL_VERSION,
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
    agent_config: broadcast::Sender<LatencyTargets>,
    reports: mpsc::Sender<IncomingReport>,
    database: PathBuf,
    admin_hash: Arc<Vec<u8>>,
    sessions: Arc<RwLock<HashMap<String, Instant>>>,
    settings: Arc<RwLock<AppSettings>>,
}

#[derive(Debug)]
struct IncomingReport {
    node_id: String,
    report: AgentReport,
}

pub async fn run(options: ServerOptions) -> Result<()> {
    let connection = db::open(&options.database)?;
    if let Some(password) = db::ensure_admin(&connection)? {
        println!("管理员密钥（仅显示一次）：{password}");
    }
    let admin_hash = Arc::new(db::admin_hash(&connection)?);
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
    let initial = render_snapshot(&nodes.read().await, &initial_site)?;
    let snapshot = Arc::new(RwLock::new(initial));
    let (broadcast, _) = broadcast::channel::<Arc<str>>(16);
    let (agent_config, _) = broadcast::channel::<LatencyTargets>(8);
    let (reports_tx, reports_rx) = mpsc::channel::<IncomingReport>(4096);
    let persist_tx = start_database_writer(options.database.clone())?;

    tokio::spawn(process_reports(nodes.clone(), reports_rx, persist_tx));
    tokio::spawn(refresh_snapshots(
        nodes.clone(),
        snapshot.clone(),
        broadcast.clone(),
        settings.clone(),
    ));

    let state = AppState {
        nodes,
        snapshot,
        broadcast,
        agent_config,
        reports: reports_tx,
        database: options.database.clone(),
        admin_hash,
        sessions: Arc::new(RwLock::new(HashMap::new())),
        settings,
    };
    let app = Router::new()
        .route("/", get(index))
        .route("/admin", get(admin))
        .route("/assets/app.css", get(styles))
        .route("/assets/app.js", get(script))
        .route("/assets/admin.js", get(admin_script))
        .route("/favicon.svg", get(favicon))
        .route("/api/health", get(health))
        .route("/api/nodes", get(nodes_api))
        .route("/api/ws", get(browser_socket))
        .route("/api/agent", get(agent_socket))
        .route("/api/admin/login", post(admin_login))
        .route("/api/admin/logout", post(admin_logout))
        .route("/api/admin/state", get(admin_state))
        .route("/api/admin/nodes", post(admin_create_node))
        .route("/api/admin/nodes/{id}", delete(admin_revoke_node))
        .route("/api/admin/latency", put(admin_save_latency))
        .route("/api/admin/site", put(admin_save_site))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(options.listen)
        .await
        .with_context(|| format!("无法监听 {}", options.listen))?;
    info!(listen = %options.listen, database = %options.database.display(), node_count, "控制端已启动");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn start_database_writer(path: PathBuf) -> Result<SyncSender<PersistEvent>> {
    let (sender, receiver) = sync_channel::<PersistEvent>(8192);
    thread::Builder::new()
        .name("monitor-db".to_owned())
        .spawn(move || {
            let mut connection = match db::open(&path) {
                Ok(connection) => connection,
                Err(error) => {
                    error!(%error, "数据库线程启动失败");
                    return;
                }
            };
            let mut last_prune = 0_i64;
            while let Ok(first) = receiver.recv() {
                let mut batch = Vec::with_capacity(256);
                batch.push(first);
                while batch.len() < 512 {
                    match receiver.try_recv() {
                        Ok(event) => batch.push(event),
                        Err(_) => break,
                    }
                }
                if let Err(error) = db::persist_batch(&mut connection, &batch) {
                    error!(%error, count = batch.len(), "写入监控数据失败");
                }
                let now = Utc::now().timestamp();
                if now - last_prune >= 3600 {
                    if let Err(error) = db::prune_history(&connection, now) {
                        warn!(%error, "清理历史数据失败");
                    }
                    last_prune = now;
                }
            }
        })
        .context("无法启动数据库线程")?;
    Ok(sender)
}

async fn process_reports(
    nodes: Arc<RwLock<HashMap<String, StoredNode>>>,
    mut receiver: mpsc::Receiver<IncomingReport>,
    persist: SyncSender<PersistEvent>,
) {
    while let Some(incoming) = receiver.recv().await {
        let now = Utc::now();
        let timestamp = now.timestamp();
        let month_key = now.format("%Y-%m").to_string();
        let day_key = now.format("%Y-%m-%d").to_string();
        let mut guard = nodes.write().await;
        let Some(node) = guard.get_mut(&incoming.node_id) else {
            continue;
        };
        let report = incoming.report;
        let first_report = node.boot_id.is_empty();
        let same_boot = !first_report && node.boot_id == report.boot_id;
        let delta_rx = if first_report {
            0
        } else if same_boot {
            report
                .metrics
                .net_rx_total
                .saturating_sub(node.last_rx_counter)
        } else {
            report.metrics.net_rx_total
        };
        let delta_tx = if first_report {
            0
        } else if same_boot {
            report
                .metrics
                .net_tx_total
                .saturating_sub(node.last_tx_counter)
        } else {
            report.metrics.net_tx_total
        };

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
        };
        drop(guard);

        match persist.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => warn!("数据库队列已满，本次快照仅保留在内存中"),
            Err(TrySendError::Disconnected(_)) => error!("数据库线程已经停止"),
        }
    }
}

async fn refresh_snapshots(
    nodes: Arc<RwLock<HashMap<String, StoredNode>>>,
    snapshot: Arc<RwLock<Arc<str>>>,
    sender: broadcast::Sender<Arc<str>>,
    settings: Arc<RwLock<AppSettings>>,
) {
    let mut ticker = interval(Duration::from_secs(2));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        let site = settings.read().await.site.clone();
        let rendered = render_snapshot(&nodes.read().await, &site);
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
    let allowed = {
        let guard = state.nodes.read().await;
        guard
            .get(node_id)
            .map(|node| {
                let provided = db::token_hash(token);
                provided.as_slice().ct_eq(node.token_hash.as_slice()).into()
            })
            .unwrap_or(false)
    };
    if !allowed {
        return plain(StatusCode::UNAUTHORIZED, "invalid bearer token");
    }

    let node_id = node_id.to_owned();
    websocket
        .read_buffer_size(4 * 1024)
        .write_buffer_size(4 * 1024)
        .max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .on_upgrade(move |socket| handle_agent(socket, state, node_id))
}

async fn handle_agent(socket: WebSocket, state: AppState, node_id: String) {
    let (mut writer, mut reader) = socket.split();
    let initial = state.settings.read().await.latency.clone();
    if send_targets(&mut writer, initial).await.is_err() {
        return;
    }
    let mut config_updates = state.agent_config.subscribe();
    loop {
        tokio::select! {
            incoming = reader.next() => {
                let report = match incoming {
                    Some(Ok(Message::Text(text))) => serde_json::from_str::<AgentReport>(&text),
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                    _ => continue,
                };
                let Ok(report) = report else { return };
                if !valid_report(&report) { return; }
                if state.reports.send(IncomingReport {
                    node_id: node_id.clone(),
                    report,
                }).await.is_err() {
                    return;
                }
            }
            update = config_updates.recv() => match update {
                Ok(targets) => {
                    if send_targets(&mut writer, targets).await.is_err() { return; }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            }
        }
    }
}

async fn send_targets(
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    targets: LatencyTargets,
) -> Result<(), axum::Error> {
    let payload = serde_json::to_string(&ServerMessage::LatencyTargets { targets })
        .expect("序列化固定配置不应失败");
    writer.send(Message::Text(payload.into())).await
}

fn valid_report(report: &AgentReport) -> bool {
    report.protocol == PROTOCOL_VERSION
        && report.agent_version.len() <= 32
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
        && report.metrics.load.iter().all(|value| value.is_finite())
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

async fn admin_login(State(state): State<AppState>, Json(request): Json<LoginRequest>) -> Response {
    if request.password.len() > 256 {
        return json_error(StatusCode::UNAUTHORIZED, "管理员密钥错误");
    }
    let provided = db::token_hash(&request.password);
    let allowed: bool = provided
        .as_slice()
        .ct_eq(state.admin_hash.as_slice())
        .into();
    if !allowed {
        tokio::time::sleep(Duration::from_millis(350)).await;
        return json_error(StatusCode::UNAUTHORIZED, "管理员密钥错误");
    }
    let token = random_session();
    state.sessions.write().await.insert(
        token.clone(),
        Instant::now() + Duration::from_secs(12 * 3600),
    );
    json_value(StatusCode::OK, &LoginResponse { token })
}

async fn admin_logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = bearer_token(&headers) {
        state.sessions.write().await.remove(token);
    }
    empty(StatusCode::NO_CONTENT)
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

async fn admin_create_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateNodeRequest>,
) -> Response {
    if !is_admin(&state, &headers).await {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    }
    let result = (|| -> Result<(String, StoredNode)> {
        let token = db::create_node(&state.database, &request.id, &request.name)?;
        let connection = db::open(&state.database)?;
        let node = db::load_nodes(&connection)?
            .into_iter()
            .find(|node| node.id == request.id)
            .context("创建后的节点不存在")?;
        Ok((token, node))
    })();
    match result {
        Ok((token, node)) => {
            state
                .nodes
                .write()
                .await
                .insert(node.id.clone(), node.clone());
            json_value(
                StatusCode::CREATED,
                &CreateNodeResponse {
                    id: node.id,
                    name: node.name,
                    token,
                },
            )
        }
        Err(error) => json_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

async fn admin_revoke_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !is_admin(&state, &headers).await {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    }
    match db::revoke_node(&state.database, &id) {
        Ok(true) => {
            state.nodes.write().await.remove(&id);
            empty(StatusCode::NO_CONTENT)
        }
        Ok(false) => json_error(StatusCode::NOT_FOUND, "节点不存在"),
        Err(error) => json_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

async fn admin_save_latency(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(targets): Json<LatencyTargets>,
) -> Response {
    if !is_admin(&state, &headers).await {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    }
    if !valid_targets(&targets) {
        return json_error(StatusCode::BAD_REQUEST, "地址必须为有效的 host:port");
    }
    if let Err(error) = db::save_latency(&state.database, &targets) {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
    }
    state.settings.write().await.latency = targets.clone();
    let _ = state.agent_config.send(targets.clone());
    json_value(StatusCode::OK, &targets)
}

async fn admin_save_site(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(site): Json<SiteSettings>,
) -> Response {
    if !is_admin(&state, &headers).await {
        return json_error(StatusCode::UNAUTHORIZED, "请先登录");
    }
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
    if let Err(error) = db::save_site(&state.database, &site) {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
    }
    state.settings.write().await.site = site.clone();
    json_value(StatusCode::OK, &site)
}

fn valid_targets(targets: &LatencyTargets) -> bool {
    [&targets.telecom, &targets.unicom, &targets.mobile]
        .into_iter()
        .all(|target| {
            !target.is_empty()
                && target.len() <= 255
                && !target
                    .chars()
                    .any(|character| character.is_whitespace() || "/?#".contains(character))
                && target.rsplit_once(':').is_some_and(|(host, port)| {
                    !host.is_empty() && port.parse::<u16>().is_ok_and(|value| value > 0)
                })
        })
}

async fn is_admin(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = bearer_token(headers) else {
        return false;
    };
    if token.len() > 128 {
        return false;
    }
    let now = Instant::now();
    let mut sessions = state.sessions.write().await;
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
    if writer
        .send(Message::Text(initial.to_string().into()))
        .await
        .is_err()
    {
        return;
    }
    let mut receiver = state.broadcast.subscribe();
    loop {
        tokio::select! {
            update = receiver.recv() => match update {
                Ok(payload) => {
                    if writer.send(Message::Text(payload.to_string().into())).await.is_err() {
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
    static_response("text/css; charset=utf-8", include_str!("ui/app.css"))
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
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
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
}
