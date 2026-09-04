const $ = selector => document.querySelector(selector);
const nodesRoot = $("#nodes");
let payload = null, receivedAt = 0, socket = null, fallbackTimer = null, reconnectTimer = null;
let stopped = false, polling = false, activeTab = "resources", hours = 6, historyData = null, historyRequest = null;
let detailId = null, detailCapacity = null;
if (location.pathname.startsWith("/node/")) {
  try { detailId = decodeURIComponent(location.pathname.slice(6)); }
  catch { detailId = location.pathname.slice(6); }
}
$("#home-view").hidden = !!detailId;
$("#detail-view").hidden = !detailId;

function escapeHtml(value) {
  return String(value ?? "").replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;").replaceAll("'", "&#039;");
}
function bytes(value) {
  if (!Number.isFinite(value) || value < 0) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
  let i = 0;
  while (value >= 1024 && i < units.length - 1) { value /= 1024; i++; }
  return String(Number(value.toFixed(i === 0 || value >= 100 ? 0 : value >= 10 ? 1 : 2))) + " " + units[i];
}
const rate = value => Number.isFinite(value) ? bytes(value) + "/s" : "—";
const pct = value => Number.isFinite(value) ? value.toFixed(1) + "%" : "—";
const ratio = (used, total) => total > 0 && Number.isFinite(used) ? Math.min(100, used / total * 100) : null;
function uptime(seconds) {
  if (!Number.isFinite(seconds)) return "—";
  const days = Math.floor(seconds / 86400), h = Math.floor(seconds % 86400 / 3600);
  return days ? days + " 天 " + h + " 小时" : h ? h + " 小时" : Math.floor(seconds / 60) + " 分钟";
}
const duplex = (rx, tx, format = bytes) => '<span><i aria-label="下载">↓</i>' + format(rx) + '</span><span><i aria-label="上传">↑</i>' + format(tx) + "</span>";
function isStale() { return !receivedAt || Date.now() - receivedAt > 12000; }
function isOnline(node) {
  const now = (payload?.generated_at || 0) + (Date.now() - receivedAt) / 1000;
  return !isStale() && node.online && now - node.last_seen <= 12;
}
function status(node) {
  const online = isOnline(node), stale = isStale();
  return '<span class="status ' + (stale ? "stale" : online ? "online" : "") + '">' + (stale ? "数据过期" : online ? "在线 " + uptime(node.metrics?.uptime) : "离线") + "</span>";
}
function metric(label, value, sub) {
  const number = Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : 0;
  return '<div class="metric"><div class="metric-label"><span>' + escapeHtml(label) + "</span><b>" + pct(value) + '</b></div><progress aria-label="' + escapeHtml(label) + '" max="100" value="' + number + '"></progress><span class="metric-sub">' + escapeHtml(sub) + "</span></div>";
}
function card(node) {
  const m = node.metrics || {}, online = isOnline(node);
  return '<div class="node-title-row"><h2>' + escapeHtml(node.name) + "</h2>" + status(node) + '</div><p class="node-system" title="' + escapeHtml([node.os, node.virtualization, node.arch].filter(Boolean).join(" · ")) + '">' + escapeHtml([node.os, node.virtualization, node.arch].filter(Boolean).join(" · ") || "等待首次上报") + '</p><div class="resource-grid">' +
    metric("CPU " + (node.cpu_cores || "—") + " 核", m.cpu, m.load?.map(n => n.toFixed(2)).join(" ") || "—") +
    metric("内存", ratio(m.mem_used, node.mem_total), bytes(m.mem_used) + " / " + bytes(node.mem_total)) +
    metric("硬盘", ratio(m.disk_used, node.disk_total), bytes(m.disk_used) + " / " + bytes(node.disk_total)) +
    '<div class="metric"><div class="metric-label"><span>本月流量</span></div><strong class="month-value">' + bytes(node.month_rx + node.month_tx) + '</strong><span class="metric-sub">↓ ' + bytes(node.month_rx) + " · ↑ " + bytes(node.month_tx) + '</span></div></div><div class="node-network"><div class="duplex">' + duplex(online ? m.net_rx : null, online ? m.net_tx : null, rate) + '</div><div class="duplex">' + duplex(node.total_rx, node.total_tx) + "</div></div>";
}
function detail(node) {
  const m = node.metrics || {};
  const facts = [
    ["系统", [node.os, node.kernel].filter(Boolean).join(" · ")],
    ["CPU", (node.cpu_name || "—") + " × " + (node.cpu_cores || "—")],
    ["内存 / 硬盘", bytes(node.mem_total) + " / " + bytes(node.disk_total)],
    ["架构", [node.arch, node.virtualization, (m.processes ?? "—") + " 进程"].filter(Boolean).join(" · ")],
    ["今日流量", "↓ " + bytes(node.day_rx) + " · ↑ " + bytes(node.day_tx)],
    ["本月流量 · UTC", "↓ " + bytes(node.month_rx) + " · ↑ " + bytes(node.month_tx)],
    ["连接", "TCP " + (m.tcp ?? "—") + " · UDP " + (m.udp ?? "—")],
    ["Swap", bytes(m.swap_used) + " / " + bytes(node.swap_total)],
    ["累计流量", "↓ " + bytes(node.total_rx) + " · ↑ " + bytes(node.total_tx)],
  ];
  $("#node-detail").innerHTML = '<div class="detail-title"><h1>' + escapeHtml(node.name) + "</h1>" + status(node) + '<span class="badge">agent ' + escapeHtml(node.agent_version || "—") + '</span></div><dl class="detail-facts">' + facts.map(([label, value]) => "<div><dt>" + label + "</dt><dd>" + escapeHtml(value || "—") + "</dd></div>").join("") + "</dl>";
}
function render() {
  if (!payload) return;
  const nodes = payload.nodes || [], site = payload.site || {}, online = nodes.filter(isOnline);
  $(".brand > span:last-child").textContent = site.name || "Monitor";
  document.title = (detailId ? (nodes.find(n => n.id === detailId)?.name || "节点") + " · " : "") + (site.name || "Monitor");
  $('meta[name="description"]').content = site.description || "";
  $("#site-description").textContent = site.description || "";
  $("#site-description").hidden = !site.description || !!detailId;
  $(".footer").textContent = site.footer || "";
  const updated = new Date(payload.generated_at * 1000).toLocaleTimeString("zh-CN", {hour12:false});
  $("#updated-at").textContent = isStale() ? "连接已断开 · 保留 " + updated + " 的数据" : updated + " 更新";
  if (detailId) {
    const node = nodes.find(n => n.id === detailId);
    if (node) {
      detail(node);
      const capacity = node.mem_total + ":" + node.disk_total;
      if (capacity !== detailCapacity) { detailCapacity = capacity; drawHistory(); }
    }
    else $("#node-detail").innerHTML = '<div class="empty-card"><h2>节点不存在或已停用</h2></div>';
    return;
  }
  $("#node-count").textContent = isStale() ? "— / " + nodes.length : online.length + " / " + nodes.length;
  $("#offline-label").textContent = isStale() ? "数据过期" : online.length === nodes.length && nodes.length ? "全部在线" : nodes.length - online.length + " 台离线";
  const busiest = online.reduce((a, b) => (a?.metrics?.cpu ?? -1) > (b.metrics?.cpu ?? -1) ? a : b, null);
  $("#busy-value").textContent = pct(busiest?.metrics?.cpu);
  $("#busy-node").textContent = busiest?.name || "—";
  const total = field => nodes.reduce((sum, n) => sum + (n[field] || 0), 0);
  $("#today-traffic").innerHTML = duplex(total("day_rx"), total("day_tx"));
  $("#total-traffic").innerHTML = duplex(total("total_rx"), total("total_tx"));
  $("#live-speed").innerHTML = duplex(isStale() ? null : online.reduce((sum, n) => sum + (n.metrics?.net_rx || 0), 0), isStale() ? null : online.reduce((sum, n) => sum + (n.metrics?.net_tx || 0), 0), rate);
  if (!nodes.length) { nodesRoot.innerHTML = '<div class="empty-card"><h2>还没有节点</h2><p>在后台添加节点后，执行生成的探针安装命令。</p></div>'; return; }
  nodesRoot.querySelector(".empty-card")?.remove();
  const existing = new Map([...nodesRoot.querySelectorAll(".node-card")].map(el => [el.dataset.id, el]));
  for (const node of nodes) {
    let el = existing.get(node.id);
    if (!el) { el = document.createElement("a"); el.dataset.id = node.id; el.href = "/node/" + encodeURIComponent(node.id); nodesRoot.append(el); }
    existing.delete(node.id);
    el.className = "node-card" + (isOnline(node) ? "" : " offline-mask");
    el.setAttribute("aria-label", node.name + "，查看详情");
    el.innerHTML = card(node);
  }
  for (const el of existing.values()) el.remove();
}

function chart(title, points, series, maximum) {
  const width = Math.max(280, $("#charts").clientWidth), height = 168, left = width < 500 ? 58 : 72, right = width - 8, top = 10, bottom = 138;
  const max = Math.max(1, maximum || 0, ...points.flatMap(p => series.map(s => Number.isFinite(p[s.key]) ? p[s.key] : 0)));
  const y = value => bottom - Math.min(max, Math.max(0, value)) / max * (bottom - top);
  const x = at => left + (at - historyData.from) / Math.max(1, historyData.to - historyData.from) * (right - left);
  let body = "";
  for (let i = 0; i <= 4; i++) {
    const value = max * i / 4, yy = y(value);
    body += '<line class="grid-line" x1="' + left + '" x2="' + right + '" y1="' + yy + '" y2="' + yy + '"/><text x="' + (left - 8) + '" y="' + (yy + 4) + '" text-anchor="end">' + escapeHtml(series[0].format(value)) + "</text>";
  }
  const ticks = width < 500 ? 3 : 6;
  for (let i = 0; i < ticks; i++) {
    const at = historyData.from + (historyData.to - historyData.from) * i / (ticks - 1);
    const date = new Date(at * 1000), label = hours <= 24 ? date.toLocaleTimeString("zh-CN", {hour:"2-digit",minute:"2-digit",hour12:false}) : date.toLocaleDateString("zh-CN",{month:"numeric",day:"numeric"});
    body += '<text x="' + x(at) + '" y="160" text-anchor="' + (i === 0 ? "start" : i === ticks - 1 ? "end" : "middle") + '">' + label + "</text>";
  }
  series.forEach((s, seriesIndex) => {
    let segments = [], segment = [], prev = 0;
    for (const point of points) {
      if (point.at - prev > historyData.step * 1.6 || !Number.isFinite(point[s.key])) { if (segment.length) segments.push(segment); segment = []; }
      if (Number.isFinite(point[s.key])) segment.push([x(point.at), y(point[s.key])]);
      prev = point.at;
    }
    if (segment.length) segments.push(segment);
    for (const seg of segments) {
      const path = seg.map(([xx, yy], i) => (i ? "L" : "M") + xx.toFixed(2) + "," + yy.toFixed(2)).join(" ");
      if (series.length === 1) body += '<path class="area" d="' + path + "L" + seg.at(-1)[0] + "," + bottom + "L" + seg[0][0] + "," + bottom + 'Z"/>';
      body += '<path class="line ' + (seriesIndex ? "secondary" : "") + '" d="' + path + '"/>';
      if (seg.length === 1) body += '<circle cx="' + seg[0][0] + '" cy="' + seg[0][1] + '" r="2" fill="currentColor"/>';
    }
    if (s.failure) for (const p of points) if (p[s.failure] > 0) body += '<line class="failure" x1="' + x(p.at) + '" x2="' + x(p.at) + '" y1="' + (bottom - 5) + '" y2="' + bottom + '"/>';
  });
  const el = document.createElement("section");
  el.className = "chart-block";
  el.innerHTML = "<h2>" + escapeHtml(title) + "</h2>" + (series.length > 1 ? '<div class="chart-legend">' + series.map((s, i) => '<span class="' + (i ? "secondary" : "") + '">' + s.label + "</span>").join("") + "</div>" : "") + '<svg class="chart" viewBox="0 0 ' + width + " " + height + '" role="img" tabindex="0" aria-label="' + escapeHtml(title + "，使用左右方向键查看数据") + '">' + body + '</svg><div class="chart-tooltip" hidden></div>';
  const svg = el.querySelector("svg"), tooltip = el.querySelector(".chart-tooltip");
  let selected = 0;
  const show = index => {
    const p = points[index]; if (!p) return;
    tooltip.hidden = false;
    tooltip.textContent = new Date(p.at * 1000).toLocaleString("zh-CN", {hour12:false}) + " · " + series.map(s => s.label + " " + s.format(p[s.key]) + (s.failure ? " · 失败 " + p[s.failure] + "/" + p.count : "")).join(" · ");
  };
  svg.addEventListener("pointermove", e => {
    const rect = svg.getBoundingClientRect(), px = (e.clientX - rect.left) * width / rect.width;
    const at = historyData.from + Math.max(0, Math.min(1, (px - left) / (right - left))) * (historyData.to - historyData.from);
    selected = points.reduce((best, p, i) => Math.abs(p.at - at) < Math.abs(points[best].at - at) ? i : best, 0); show(selected);
  });
  svg.addEventListener("pointerleave", () => tooltip.hidden = true);
  svg.addEventListener("blur", () => tooltip.hidden = true);
  svg.addEventListener("keydown", e => {
    if (!["ArrowLeft","ArrowRight"].includes(e.key)) return;
    e.preventDefault(); selected = Math.max(0, Math.min(points.length - 1, selected + (e.key === "ArrowLeft" ? -1 : 1))); show(selected);
  });
  return el;
}
function drawHistory() {
  if (!historyData || !detailId) return;
  const root = $("#charts"), points = activeTab === "resources" ? historyData.resources : historyData.latency;
  root.replaceChildren();
  if (!points?.length) { root.innerHTML = '<p class="chart-empty">此时间范围暂无记录</p>'; return; }
  const node = payload?.nodes?.find(n => n.id === detailId);
  if (activeTab === "resources") {
    root.append(chart("CPU", points, [{key:"cpu",label:"CPU",format:pct}]));
    root.append(chart("内存 · " + bytes(node?.mem_total), points, [{key:"mem_used",label:"内存",format:bytes}], node?.mem_total));
    root.append(chart("网络速率", points, [{key:"net_rx",label:"下载",format:rate},{key:"net_tx",label:"上传",format:rate}]));
    root.append(chart("硬盘 · " + bytes(node?.disk_total), points, [{key:"disk_used",label:"硬盘",format:bytes}], node?.disk_total));
  } else for (const [key, label] of [["telecom","电信"],["unicom","联通"],["mobile","移动"]]) {
    root.append(chart(label + " · " + (historyData.targets?.[key] || "—"), points, [{key,label,format:v => Number.isFinite(v) ? v.toFixed(1) + " ms" : "失败",failure:key + "_failures"}]));
  }
}
async function loadHistory() {
  if (!detailId) return;
  historyRequest?.abort();
  const request = new AbortController(); historyRequest = request;
  if (!historyData) $("#charts").innerHTML = '<p class="chart-empty">正在读取历史…</p>';
  try {
    const response = await fetch("/api/nodes/" + encodeURIComponent(detailId) + "/history?hours=" + hours + "&kind=" + activeTab, {cache:"no-store",signal:request.signal});
    if (!response.ok) throw new Error(response.status === 404 ? "节点不存在或已停用" : "历史读取失败，请稍后重试");
    const data = await response.json();
    if (request !== historyRequest) return;
    historyData = data;
    $("#history-note").textContent = activeTab === "latency" ? "每 30 秒检测一轮 · TCP 建连时间 · 红线表示检测失败 · 仅显示当前目标的历史" : "每分钟记录 · 历史保留 30 天";
    if (data.step > (activeTab === "latency" ? 30 : 60)) $("#history-note").textContent += " · 当前按 " + Math.round(data.step / 60) + " 分钟汇总";
    drawHistory();
  } catch (error) {
    if (error.name === "AbortError") return;
    $("#charts").innerHTML = '<p class="chart-empty">' + escapeHtml(error.message) + "</p>";
  }
}
for (const button of document.querySelectorAll("[data-tab],[data-hours]")) button.addEventListener("click", () => {
  const selector = button.dataset.tab ? "[data-tab]" : "[data-hours]";
  for (const el of document.querySelectorAll(selector)) { el.classList.toggle("selected", el === button); el.setAttribute("aria-pressed", String(el === button)); }
  if (button.dataset.tab) activeTab = button.dataset.tab; else hours = Number(button.dataset.hours);
  historyData = null; loadHistory();
});
let resizeTimer;
addEventListener("resize", () => { clearTimeout(resizeTimer); resizeTimer = setTimeout(drawHistory, 150); });

function setConnection(mode, label) { $("#connection").className = "connection " + mode; $("#connection span").textContent = label; }
function accept(data, mode) {
  if (!data || !Array.isArray(data.nodes) || !Number.isFinite(data.generated_at)) return;
  if (payload && data.generated_at < payload.generated_at) return;
  if (!payload || data.generated_at > payload.generated_at) receivedAt = Date.now();
  payload = data; render(); setConnection(isStale() ? "offline" : "online", isStale() ? "数据过期" : mode);
}
async function poll() {
  if (polling || stopped) return;
  polling = true;
  try {
    const response = await fetch("/api/nodes", {cache:"no-store",signal:AbortSignal.timeout(6000)});
    if (!response.ok) throw new Error();
    accept(await response.json(), "轮询中");
  } catch { setConnection("offline", "已断开"); } finally { polling = false; }
}
function startFallback() { if (!fallbackTimer) { poll(); fallbackTimer = setInterval(poll, 5000); } }
function connect() {
  if (stopped) return;
  try { socket = new WebSocket((location.protocol === "https:" ? "wss://" : "ws://") + location.host + "/api/ws"); }
  catch { startFallback(); reconnectTimer = setTimeout(connect, 5000); return; }
  const current = socket;
  current.addEventListener("message", event => {
    try { accept(JSON.parse(event.data), "实时"); clearInterval(fallbackTimer); fallbackTimer = null; } catch { /* 等待有效快照 */ }
  });
  current.addEventListener("error", () => current.close());
  current.addEventListener("close", () => {
    if (stopped || current !== socket) return;
    startFallback(); clearTimeout(reconnectTimer); reconnectTimer = setTimeout(connect, 5000);
  });
}
const freshnessTimer = setInterval(() => {
  if (isStale()) { setConnection("offline", "数据过期"); render(); startFallback(); if (socket?.readyState === WebSocket.OPEN) socket.close(); }
}, 3000);
const historyTimer = detailId ? setInterval(() => { if (!document.hidden) loadHistory(); }, 30000) : null;
addEventListener("pagehide", () => {
  stopped = true; historyRequest?.abort(); clearInterval(fallbackTimer); clearInterval(freshnessTimer); clearInterval(historyTimer); clearTimeout(reconnectTimer); socket?.close();
});
addEventListener("pageshow", event => { if (event.persisted) location.reload(); });
startFallback(); connect(); loadHistory();
