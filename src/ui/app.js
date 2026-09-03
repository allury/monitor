const nodesRoot = document.querySelector("#nodes");
const connection = document.querySelector("#connection");
const histories = new Map();
let fallbackTimer = null;
let reconnectTimer = null;
let stopped = false;

const savedTheme = localStorage.getItem("monitor-theme");
const wantsDark = matchMedia("(prefers-color-scheme: dark)").matches;
document.documentElement.classList.toggle("dark", savedTheme ? savedTheme === "dark" : wantsDark);
document.querySelector("#theme-toggle").addEventListener("click", () => {
  const dark = document.documentElement.classList.toggle("dark");
  localStorage.setItem("monitor-theme", dark ? "dark" : "light");
});

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function bytes(value) {
  if (!Number.isFinite(value) || value < 0) return "—";
  if (value < 1024) return `${Math.round(value)} B`;
  const units = ["KiB", "MiB", "GiB", "TiB", "PiB"];
  let current = value;
  let unit = -1;
  do { current /= 1024; unit += 1; } while (current >= 1024 && unit < units.length - 1);
  const digits = current >= 100 ? 0 : current >= 10 ? 1 : 2;
  return `${current.toFixed(digits)} ${units[unit]}`;
}

function rate(value) { return `${bytes(value)}/s`; }
function ratio(used, total) { return total > 0 ? Math.min(100, used / total * 100) : 0; }
function pct(value) { return `${Math.max(0, value || 0).toFixed(1)}%`; }

function uptime(seconds) {
  if (!seconds) return "—";
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor(seconds % 86400 / 3600);
  const minutes = Math.floor(seconds % 3600 / 60);
  if (days) return `${days} 天 ${hours} 小时`;
  if (hours) return `${hours} 小时 ${minutes} 分`;
  return `${minutes} 分`;
}

function lastSeen(timestamp, online) {
  if (online) return "刚刚更新";
  if (!timestamp) return "等待首次上报";
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - timestamp);
  if (delta < 60) return `${delta} 秒前`;
  if (delta < 3600) return `${Math.floor(delta / 60)} 分前`;
  return `${Math.floor(delta / 3600)} 小时前`;
}

function latency(label, value) {
  return `<span class="latency"><em>${label}</em>${Number.isFinite(value) ? `${Math.round(value)} ms` : "—"}</span>`;
}

function sparkline(values) {
  if (values.length < 2) return "";
  const max = Math.max(1, ...values);
  const points = values.map((value, index) => {
    const x = index / (values.length - 1) * 100;
    const y = 19 - value / max * 17;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  });
  return `<svg class="sparkline" viewBox="0 0 100 20" preserveAspectRatio="none" aria-hidden="true"><path d="M${points.join(" L")}"/></svg>`;
}

function remember(node) {
  if (!node.online || !node.metrics) return;
  const values = histories.get(node.id) || [];
  values.push((node.metrics.net_rx || 0) + (node.metrics.net_tx || 0));
  if (values.length > 40) values.shift();
  histories.set(node.id, values);
}

function card(node) {
  const metrics = node.metrics || {};
  const lat = metrics.latency || {};
  const cpu = Math.max(0, Math.min(100, metrics.cpu || 0));
  const memory = ratio(metrics.mem_used, node.mem_total);
  const disk = ratio(metrics.disk_used, node.disk_total);
  const system = [node.os, node.arch, node.virtualization].filter(Boolean);
  const chips = [node.arch, node.virtualization, `${node.cpu_cores || 0} 核`]
    .filter(Boolean)
    .map(value => `<span class="chip">${escapeHtml(value)}</span>`)
    .join("");
  const classes = node.online ? "node-card" : "node-card offline-mask";
  return `<article class="${classes}">
    <div class="node-head">
      <div class="node-title-row">
        <div class="node-title"><h2>${escapeHtml(node.name)}</h2><p title="${escapeHtml(system.join(" · "))}">${escapeHtml(node.os || "等待系统信息")}</p></div>
        <span class="status ${node.online ? "online" : ""}">${node.online ? "在线" : "离线"}</span>
      </div>
      <div class="chips">${chips}</div>
    </div>
    <div class="node-body">
      <div class="resource-grid">
        <div class="metric"><div class="metric-label"><span>CPU</span><b>${pct(cpu)}</b></div><progress max="100" value="${cpu}"></progress></div>
        <div class="metric"><div class="metric-label"><span>内存</span><b>${pct(memory)}</b></div><progress max="100" value="${memory}"></progress></div>
        <div class="metric"><div class="metric-label"><span>硬盘</span><b>${pct(disk)}</b></div><progress max="100" value="${disk}"></progress></div>
      </div>
      <div class="detail-row"><span>实时网速</span><div class="network-values"><span class="down">${rate(metrics.net_rx)}</span><span class="up">${rate(metrics.net_tx)}</span></div></div>
      <div class="detail-row"><span>三网延迟</span><div class="latencies">${latency("电信", lat.telecom)}${latency("联通", lat.unicom)}${latency("移动", lat.mobile)}</div></div>
      <div class="traffic-grid">
        <div class="traffic-item"><span>今日</span><b>${bytes((node.day_rx || 0) + (node.day_tx || 0))}</b></div>
        <div class="traffic-item"><span>本月</span><b>${bytes((node.month_rx || 0) + (node.month_tx || 0))}</b></div>
        <div class="traffic-item"><span>累计</span><b>${bytes((node.total_rx || 0) + (node.total_tx || 0))}</b></div>
      </div>
      <div class="node-foot"><span>${lastSeen(node.last_seen, node.online)} · 已运行 ${uptime(metrics.uptime)}</span>${sparkline(histories.get(node.id) || [])}</div>
    </div>
  </article>`;
}

function render(payload) {
  const nodes = Array.isArray(payload.nodes) ? payload.nodes : [];
  const site = payload.site || {};
  if (site.name) {
    document.querySelector(".brand > span:last-child").textContent = site.name;
    document.title = site.name;
  }
  document.querySelector('meta[name="description"]').content = site.description || "";
  document.querySelector(".footer").textContent = site.footer || "";
  nodes.forEach(remember);
  const online = nodes.filter(node => node.online);
  const totals = nodes.reduce((sum, node) => {
    sum.today += (node.day_rx || 0) + (node.day_tx || 0);
    sum.total += (node.total_rx || 0) + (node.total_tx || 0);
    return sum;
  }, {today: 0, total: 0});
  const speeds = online.reduce((sum, node) => {
    sum.rx += node.metrics?.net_rx || 0;
    sum.tx += node.metrics?.net_tx || 0;
    return sum;
  }, {rx: 0, tx: 0});

  document.querySelector("#node-count").textContent = online.length;
  document.querySelector("#offline-label").textContent = nodes.length === online.length ? "全部在线" : `${nodes.length - online.length} 台离线`;
  document.querySelector("#live-speed").textContent = `↓ ${rate(speeds.rx)}  ↑ ${rate(speeds.tx)}`;
  document.querySelector("#today-traffic").textContent = bytes(totals.today);
  document.querySelector("#total-traffic").textContent = bytes(totals.total);
  document.querySelector("#updated-at").textContent = `共 ${nodes.length} 台 · ${new Date((payload.generated_at || Date.now() / 1000) * 1000).toLocaleTimeString("zh-CN", {hour12: false})} 更新`;

  if (!nodes.length) {
    nodesRoot.innerHTML = `<article class="empty-card"><div class="empty-icon" aria-hidden="true">⌁</div><h2>还没有节点</h2><p>在控制端创建节点密钥，再运行一次探针。</p></article>`;
  } else {
    nodesRoot.innerHTML = nodes.map(card).join("");
  }
}

function setConnection(mode, label) {
  connection.className = `connection ${mode}`;
  connection.querySelector("span").textContent = label;
}

async function poll() {
  try {
    const response = await fetch("/api/nodes", {cache: "no-store"});
    if (!response.ok) throw new Error(String(response.status));
    render(await response.json());
    setConnection("online", "轮询中");
  } catch {
    setConnection("offline", "已断开");
  }
}

function startFallback() {
  if (fallbackTimer) return;
  poll();
  fallbackTimer = setInterval(poll, 5000);
}

function stopFallback() {
  if (!fallbackTimer) return;
  clearInterval(fallbackTimer);
  fallbackTimer = null;
}

function connect() {
  if (stopped) return;
  setConnection("", "连接中");
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  let socket;
  try { socket = new WebSocket(`${scheme}://${location.host}/api/ws`); }
  catch { startFallback(); return; }
  socket.addEventListener("open", () => {
    stopFallback();
    setConnection("online", "实时");
  });
  socket.addEventListener("message", event => {
    try { render(JSON.parse(event.data)); } catch { /* 等待下一帧 */ }
  });
  socket.addEventListener("error", () => socket.close());
  socket.addEventListener("close", () => {
    if (stopped) return;
    startFallback();
    clearTimeout(reconnectTimer);
    reconnectTimer = setTimeout(connect, 5000);
  });
}

addEventListener("beforeunload", () => {
  stopped = true;
  clearInterval(fallbackTimer);
  clearTimeout(reconnectTimer);
});

connect();
