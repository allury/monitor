export function mountStatus() {
const $ = selector => document.querySelector(selector);
const nodesRoot = $("#nodes");
const lifetime = new AbortController(), speedSamples = [], chartSelections = new Map();
let payload = null, receivedAt = 0, socket = null, fallbackTimer = null, reconnectTimer = null;
let stopped = false, polling = false, activeTab = "resources", hours = 6, historyData = null, historyRequest = null;
let detailId = null, detailCapacity = null, chartWidth = 0;
const latencyView = {hidden:new Set(), window:null, clip:false};
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
// Keep bytes() unchanged for the existing monthly-traffic block.
function decimalBytes(value, minimum = 0, maximum = 4) {
  if (!Number.isFinite(value) || value < 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let index = minimum;
  value /= 1000 ** index;
  while (value >= 1000 && index < maximum) { value /= 1000; index++; }
  const digits = index === 0 || value >= 100 ? 0 : value >= 10 ? 1 : 2;
  return String(Number(value.toFixed(digits))) + " " + units[index];
}
const capacity = value => decimalBytes(value, 2, 3);
const transfer = value => decimalBytes(value, 2, 4);
const rate = value => Number.isFinite(value) && value >= 0 ? decimalBytes(value, 0, 2) + "/s" : "—";
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
function shortOs(value) {
  let os = String(value || "").replace(/\([^)]*\)/g, "").replace(/GNU\/Linux/gi, "").replace(/\bLTS\b/gi, "").replace(/\s+/g, " ").trim();
  const version = os.match(/^(.*?)\s+v?(\d+)(?:\.(\d+))?/i);
  if (version) {
    const name = version[1].trim();
    const minor = /^(Ubuntu|Alpine(?: Linux)?)$/i.test(name) && version[3] ? "." + version[3] : "";
    os = name + " " + version[2] + minor;
  }
  return os;
}
const shortVirtualization = value => value === "virtualized" ? "vm" : value;
function systemLabel(node) {
  return [shortOs(node.os), shortVirtualization(node.virtualization), node.arch].filter(Boolean).join(" · ") || "待上报";
}
function osMark(value) {
  const os = String(value || "").toLowerCase();
  const [key, label] = os.includes("debian") ? ["debian", "Debian"] : os.includes("ubuntu") ? ["ubuntu", "Ubuntu"] : os.includes("alpine") ? ["alpinelinux", "Alpine Linux"] : ["linux", "Linux"];
  return '<span class="os-mark os-' + key + '" role="img" aria-label="' + label + '" title="' + label + '"></span>';
}
function recordSpeed(data) {
  if (speedSamples.length && data.generated_at <= speedSamples.at(-1).at) return;
  const online = data.nodes.filter(node => node.online && data.generated_at - node.last_seen <= 12);
  const sum = key => online.reduce((total, node) => total + Math.max(0, Number(node.metrics?.[key]) || 0), 0);
  speedSamples.push({at:data.generated_at, rx:sum("net_rx"), tx:sum("net_tx")});
  while (speedSamples.length > 61 || speedSamples[0].at < data.generated_at - 120) speedSamples.shift();
}
function speedTrend() {
  if (speedSamples.length < 2) return "";
  const end = speedSamples.at(-1).at, max = Math.max(1, ...speedSamples.flatMap(p => [p.rx, p.tx]));
  const paths = ["rx", "tx"].map((key, index) => {
    let previous = 0;
    const path = speedSamples.map(p => {
      const move = p.at - previous > 12 ? "M" : "L"; previous = p.at;
      return move + (2 + (p.at - end + 120) / 120 * 296).toFixed(2) + "," + (42 - p[key] / max * 38).toFixed(2);
    }).join(" ");
    return '<path class="trend-line ' + (index ? "secondary" : "") + '" d="' + path + '"/>';
  }).join("");
  return '<svg viewBox="0 0 300 46" preserveAspectRatio="none" role="img" aria-label="下载实线、上传虚线；仅显示本页收到的实时采样">' + paths + '</svg>';
}
function metric(label, value, sub, digits = 1) {
  const number = Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : 0;
  const percentage = Number.isFinite(value) ? value.toFixed(digits) + "%" : "—";
  return '<div class="metric resource-metric"><div class="metric-label"><span>' + escapeHtml(label) + "</span><b>" + percentage + '</b></div><progress aria-label="' + escapeHtml(label) + '" max="100" value="' + number + '"></progress><span class="metric-sub">' + escapeHtml(sub) + "</span></div>";
}
function card(node) {
  const m = node.metrics || {}, online = isOnline(node);
  return '<div class="node-title-row"><h2>' + escapeHtml(node.name) + "</h2>" + status(node) + '</div><div class="node-subtitle"><p class="node-system" title="' + escapeHtml([node.os, node.virtualization, node.arch].filter(Boolean).join(" · ")) + '">' + escapeHtml(systemLabel(node)) + '</p>' + osMark(node.os) + '</div><div class="resource-grid">' +
    metric("CPU " + (node.cpu_cores || "—") + " 核", m.cpu, m.load?.map(n => n.toFixed(2)).join(" ") || "—") +
    metric("内存", ratio(m.mem_used, node.mem_total), capacity(m.mem_used) + " / " + capacity(node.mem_total), 0) +
    metric("硬盘", ratio(m.disk_used, node.disk_total), capacity(m.disk_used) + " / " + capacity(node.disk_total), 0) +
    '<div class="metric"><div class="metric-label"><span>本月流量</span></div><strong class="month-value">' + bytes(node.month_rx + node.month_tx) + '</strong><span class="metric-sub">↓ ' + bytes(node.month_rx) + " · ↑ " + bytes(node.month_tx) + '</span></div></div><div class="node-network"><div class="duplex">' + duplex(online ? m.net_rx : null, online ? m.net_tx : null, rate) + '</div><div class="duplex network-totals">' + duplex(node.total_rx, node.total_tx, transfer) + "</div></div>";
}
function detail(node) {
  const m = node.metrics || {};
  const facts = [
    ["系统", [shortOs(node.os), node.kernel].filter(Boolean).join(" · ")],
    ["CPU", (node.cpu_name || "—") + " × " + (node.cpu_cores || "—")],
    ["内存 / 硬盘", capacity(node.mem_total) + " / " + capacity(node.disk_total)],
    ["架构", [node.arch, shortVirtualization(node.virtualization), (m.processes ?? "—") + " 进程"].filter(Boolean).join(" · ")],
    ["今日流量", "↓ " + bytes(node.day_rx) + " · ↑ " + bytes(node.day_tx)],
    ["本月流量", "↓ " + bytes(node.month_rx) + " · ↑ " + bytes(node.month_tx)],
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
  $(".footer").textContent = site.footer || "";
  $(".footer").hidden = !site.footer?.trim();
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
  $("#today-traffic").innerHTML = duplex(total("day_rx"), total("day_tx"), transfer);
  $("#total-traffic").innerHTML = duplex(total("total_rx"), total("total_tx"), transfer);
  $("#live-speed").innerHTML = duplex(isStale() ? null : online.reduce((sum, n) => sum + (n.metrics?.net_rx || 0), 0), isStale() ? null : online.reduce((sum, n) => sum + (n.metrics?.net_tx || 0), 0), rate);
  $("#speed-trend").innerHTML = speedTrend();
  $("#speed-trend").classList.toggle("stale", isStale());
  if (!nodes.length) { nodesRoot.innerHTML = '<div class="empty-card"><h2>暂无节点</h2></div>'; return; }
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

function nearestPoint(points, at) {
  if (!points.length) return -1;
  let low = 0, high = points.length - 1;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (points[middle].at < at) low = middle + 1; else high = middle;
  }
  return low > 0 && Math.abs(points[low - 1].at - at) <= Math.abs(points[low].at - at) ? low - 1 : low;
}
function chartHit(points, at, step) {
  const index = nearestPoint(points, at);
  return index >= 0 && Math.abs(points[index].at - at) <= step * .75 ? index : -1;
}
function chartValue(point, series) {
  const value = point?.[series.key];
  return Number.isFinite(value) && (!series.failure || value >= 0) ? value : null;
}
function chartReading(point, series) {
  const value = chartValue(point, series);
  return series.failure && value === null ? null : series.label + "：" + series.format(value);
}
function latencyStats(points, series) {
  const total = points.reduce((sum, point) => sum + Math.max(0, Number(point.count) || 0), 0);
  const failed = points.reduce((sum, point) => sum + Math.min(Math.max(0, Number(point.count) || 0), Math.max(0, Number(point[series.failure]) || 0)), 0);
  const loss = total ? (failed / total * 100).toFixed(1) + "%" : "—";
  return '<details class="latency-stats"><summary aria-label="' + escapeHtml(series.label + "检测统计") + '"><span aria-hidden="true">i</span></summary><dl class="latency-stat-panel"><div><dt>失败率</dt><dd>' + loss + '</dd></div><div><dt>检测次数</dt><dd>' + total + '</dd></div><div><dt>成功次数</dt><dd>' + (total - failed) + '</dd></div></dl></details>';
}
function chart(title, points, series, maximum, selectionKey = title, options = {}) {
  const width = options.width || Math.max(280, $("#charts").clientWidth), height = options.latency ? (width < 500 ? 300 : 400) : 168;
  const left = width < 500 ? 58 : 72, right = width - 8, top = 10, bottom = height - 30;
  const {from, to} = options.window || historyData, step = historyData.step;
  const min = options.scale?.min || 0, max = options.scale?.max || Math.max(1, maximum || 0, ...points.flatMap(p => series.map(s => chartValue(p, s) ?? 0)));
  const y = value => bottom - (value - min) / (max - min) * (bottom - top);
  const x = at => left + (at - from) / Math.max(1, to - from) * (right - left);
  const format = options.format || series[0].format;
  let body = "";
  const values = options.scale?.ticks || Array.from({length:5}, (_, i) => min + (max - min) * i / 4);
  for (const value of values) {
    const yy = y(value);
    body += '<line class="grid-line" x1="' + left + '" x2="' + right + '" y1="' + yy + '" y2="' + yy + '"/><text x="' + (left - 8) + '" y="' + (yy + 4) + '" text-anchor="end">' + escapeHtml(format(value)) + "</text>";
  }
  const ticks = width < 500 ? 3 : 6;
  for (let i = 0; i < ticks; i++) {
    const at = from + (to - from) * i / (ticks - 1);
    const date = new Date(at * 1000), label = to - from <= 86400 ? date.toLocaleTimeString("zh-CN", {hour:"2-digit",minute:"2-digit",hour12:false}) : date.toLocaleDateString("zh-CN",{month:"numeric",day:"numeric"});
    body += '<text x="' + x(at) + '" y="' + (height - 8) + '" text-anchor="' + (i === 0 ? "start" : i === ticks - 1 ? "end" : "middle") + '">' + label + "</text>";
  }
  // Clip only the drawing. The original values remain available in the tooltip.
  if (options.latency) body += '<defs><clipPath id="latency-plot-clip"><rect x="' + left + '" y="' + top + '" width="' + (right - left) + '" height="' + (bottom - top) + '"/></clipPath></defs><g clip-path="url(#latency-plot-clip)">';
  series.forEach((s, seriesIndex) => {
    let segments = [], segment = [], prev = 0;
    for (const point of points) {
      const value = chartValue(point, s);
      if (point.at - prev > step * 1.6 || value === null) { if (segment.length) segments.push(segment); segment = []; }
      if (value !== null) segment.push([x(point.at), y(value)]);
      prev = point.at;
    }
    if (segment.length) segments.push(segment);
    for (const seg of segments) {
      const path = seg.map(([xx, yy], i) => (i ? "L" : "M") + xx.toFixed(2) + "," + yy.toFixed(2)).join(" ");
      if (!options.latency && series.length === 1) body += '<path class="area" d="' + path + "L" + seg.at(-1)[0] + "," + bottom + "L" + seg[0][0] + "," + bottom + 'Z"/>';
      body += '<path class="line ' + (s.style || (seriesIndex ? "secondary" : "")) + '" d="' + path + '"/>';
      if (seg.length === 1) body += '<circle cx="' + seg[0][0] + '" cy="' + seg[0][1] + '" r="2" fill="currentColor"/>';
    }
  });
  if (options.latency) body += '</g>';
  const el = document.createElement("section");
  el.className = "chart-block";
  const guide = '<g class="chart-cursor" hidden><line class="cursor-line" y1="' + top + '" y2="' + bottom + '"/>' + series.map(() => '<circle class="cursor-dot" r="3.5"/>').join("") + '</g>';
  el.innerHTML = (options.latency ? "" : '<div class="chart-heading"><h2>' + escapeHtml(title) + '</h2></div>' + (series.length > 1 ? '<div class="chart-legend">' + series.map((s, i) => '<span class="' + (i ? "secondary" : "") + '">' + escapeHtml(s.label) + "</span>").join("") + "</div>" : "")) + '<svg class="chart" viewBox="0 0 ' + width + " " + height + '" role="img" tabindex="0" aria-label="' + escapeHtml(title + "，点击固定读数，左右方向键选择，Escape 取消") + '">' + body + guide + '</svg><div class="chart-tooltip" hidden></div>';
  const svg = el.querySelector("svg"), tooltip = el.querySelector(".chart-tooltip"), cursor = el.querySelector(".chart-cursor"), cursorLine = el.querySelector(".cursor-line"), dots = el.querySelectorAll(".cursor-dot");
  if (options.latency) svg.style.height = height + "px";
  let selected = 0, pinned = false, selectedAt = null;
  const hide = () => { tooltip.hidden = true; cursor.setAttribute("hidden", ""); };
  const show = (index, at = points[index]?.at) => {
    if (!Number.isFinite(at)) return;
    selected = index; selectedAt = at;
    const point = points[index], xx = x(at);
    const readings = point ? series.map(s => chartReading(point, s)).filter(Boolean) : [];
    if (series.every(s => s.failure) && !readings.length) { hide(); return false; }
    cursor.removeAttribute("hidden");
    cursorLine.setAttribute("x1", xx); cursorLine.setAttribute("x2", xx);
    dots.forEach((dot, i) => {
      const value = chartValue(point, series[i]);
      if (value !== null && value >= min && value <= max) {
        dot.removeAttribute("hidden"); dot.setAttribute("cx", xx); dot.setAttribute("cy", y(value));
      } else dot.setAttribute("hidden", "");
    });
    tooltip.hidden = false;
    tooltip.textContent = new Date(at * 1000).toLocaleString("zh-CN", {hour12:false}) + "\n" + (readings.length ? readings.join("\n") : "暂无数据");
    tooltip.style.left = Math.max(0, Math.min(el.clientWidth - tooltip.offsetWidth, xx / width * svg.clientWidth + 12)) + "px";
    return true;
  };
  const position = e => {
    const rect = svg.getBoundingClientRect(), px = (e.clientX - rect.left) * width / rect.width;
    const at = from + Math.max(0, Math.min(1, (px - left) / (right - left))) * (to - from);
    const index = chartHit(points, at, step);
    return {index,at:index < 0 ? Math.round(at) : points[index].at};
  };
  const clear = () => { pinned = false; chartSelections.delete(selectionKey); hide(); };
  svg.addEventListener("pointermove", e => {
    if (pinned || e.pointerType === "touch") return;
    const {index,at} = position(e); show(index, at);
  });
  svg.addEventListener("click", e => {
    const {index,at} = position(e);
    if (pinned && selected === index && Math.abs(selectedAt - at) < step / 2) { clear(); return; }
    if (show(index, at)) { pinned = true; chartSelections.set(selectionKey, at); } else clear();
  });
  svg.addEventListener("pointerleave", () => { if (!pinned) hide(); });
  svg.addEventListener("blur", () => { if (!pinned) hide(); });
  svg.addEventListener("keydown", e => {
    if (e.key === "Escape") { e.preventDefault(); clear(); return; }
    if (!["ArrowLeft","ArrowRight","Home","End"].includes(e.key) || !points.length) return;
    e.preventDefault();
    const index = e.key === "Home" ? 0 : e.key === "End" ? points.length - 1 : Math.max(0, Math.min(points.length - 1, selected + (e.key === "ArrowLeft" ? -1 : 1)));
    if (show(index)) { pinned = true; chartSelections.set(selectionKey, points[index].at); } else clear();
  });
  const saved = chartSelections.get(selectionKey);
  if (Number.isFinite(saved) && saved >= from && saved <= to) {
    pinned = true;
    // Position after insertion; a detached SVG has no layout width yet.
    requestAnimationFrame(() => { if (el.isConnected && !show(chartHit(points, saved, step), saved)) clear(); });
  }
  return el;
}
const milliseconds = value => Number.isFinite(value) ? Number(value.toFixed(1)) + " ms" : "—";
const latencySeries = [["telecom","电信","primary"],["unicom","联通","secondary"],["mobile","移动","tertiary"]].map(([key,label,style]) => ({key,label,style,format:milliseconds,failure:key + "_failures"}));
function latencyRange(data, window) {
  if (!window) return {from:data.from,to:data.to};
  const span = Math.min(data.to - data.from, Math.max(data.step, window.to - window.from));
  const from = Math.max(data.from, Math.min(data.to - span, window.from));
  return {from,to:from + span};
}
function latencyScale(points, series, clip) {
  let low = Infinity, high = 0;
  for (const s of series) {
    const values = points.map(p => chartValue(p, s)).filter(v => v !== null).sort((a, b) => a - b);
    if (!values.length) continue;
    low = Math.min(low, values[0]);
    let ceiling = values.at(-1);
    // Per-target IQR avoids clipping a consistently slower carrier as an outlier.
    // This is an optional viewport cap, never a rewrite of successful samples.
    if (clip && values.length >= 8) {
      const q1 = values[Math.floor((values.length - 1) * .25)], q3 = values[Math.floor((values.length - 1) * .75)];
      ceiling = Math.min(ceiling, q3 + 1.5 * Math.max(1, q3 - q1));
    }
    high = Math.max(high, ceiling);
  }
  if (!Number.isFinite(low)) return {min:0,max:1,ticks:[0,.25,.5,.75,1]};
  const span = Math.max(1, high - low), rough = span / 4, magnitude = 10 ** Math.floor(Math.log10(rough));
  const step = [1,2,2.5,5,10].find(n => n * magnitude >= rough) * magnitude;
  const min = Math.max(0, Math.floor((low - span * .05) / step) * step), max = Math.ceil((high + span * .05) / step) * step;
  const ticks = Array.from({length:Math.round((max - min) / step) + 1}, (_, i) => min + i * step);
  return {min,max,ticks};
}
function latencyScope() {
  return JSON.stringify(latencySeries.map(s => [s.key, historyData.targets?.[s.key] || ""]));
}
function latencyChart(points, width) {
  const el = document.createElement("section"), scope = latencyScope();
  el.className = "latency-chart"; el.dataset.scope = scope;
  el.innerHTML = '<div class="latency-plot"></div><div class="latency-zoom"><div class="zoom-window" role="slider" tabindex="0" aria-label="移动时间窗口"></div><input type="range" min="0" max="1000" step="1" value="0" aria-label="起始时间"><input type="range" min="0" max="1000" step="1" value="1000" aria-label="结束时间"></div><div class="latency-legend" role="group" aria-label="延迟检测目标">' + latencySeries.map(s => '<div class="latency-legend-item"><button type="button" class="' + s.style + '" data-series="' + s.key + '" aria-pressed="true">' + s.label + '</button><span class="legend-stats"></span></div>').join("") + '</div>';
  const plot = el.querySelector(".latency-plot"), track = el.querySelector(".latency-zoom"), windowHandle = el.querySelector(".zoom-window");
  const inputs = el.querySelectorAll('input[type="range"]'), buttons = el.querySelectorAll("[data-series]"), stats = el.querySelectorAll(".legend-stats");
  const draw = () => {
    const window = latencyRange(historyData, latencyView.window), duration = Math.max(1, historyData.to - historyData.from);
    const visible = points.filter(p => p.at >= window.from && p.at <= window.to), series = latencySeries.filter(s => !latencyView.hidden.has(s.key));
    const next = chart("网络延迟", visible, series, undefined, scope, {width,window,latency:true,scale:latencyScale(visible,series,latencyView.clip),format:milliseconds});
    const focused = plot.contains(document.activeElement);
    plot.replaceChildren(next);
    if (focused) next.querySelector("svg").focus({preventScroll:true});
    track.style.marginLeft = (width < 500 ? 58 : 72) + "px";
    const start = (window.from - historyData.from) / duration * 1000, end = (window.to - historyData.from) / duration * 1000;
    inputs[0].value = start; inputs[1].value = end;
    windowHandle.style.left = start / 10 + "%"; windowHandle.style.width = (end - start) / 10 + "%";
    windowHandle.setAttribute("aria-valuemin", "0"); windowHandle.setAttribute("aria-valuemax", String(Math.round(1000 - end + start))); windowHandle.setAttribute("aria-valuenow", String(Math.round(start)));
    const dates = [window.from,window.to].map(at => new Date(at * 1000).toLocaleString("zh-CN", {hour12:false}));
    windowHandle.setAttribute("aria-valuetext", dates.join(" — "));
    inputs.forEach((input, i) => input.setAttribute("aria-valuetext", dates[i]));
    buttons.forEach((button, i) => {
      button.setAttribute("aria-pressed", String(!latencyView.hidden.has(latencySeries[i].key)));
      const open = stats[i].querySelector("details")?.open;
      stats[i].innerHTML = latencyStats(visible, latencySeries[i]);
      stats[i].querySelector("details").open = !!open;
    });
  };
  const setWindow = window => {
    const range = latencyRange(historyData, window);
    latencyView.window = range.from <= historyData.from && range.to >= historyData.to ? null : range;
    draw();
  };
  inputs.forEach((input, i) => input.addEventListener("input", () => {
    const current = latencyRange(historyData, latencyView.window), duration = historyData.to - historyData.from;
    const at = historyData.from + Number(input.value) / 1000 * duration;
    setWindow(i ? {from:current.from,to:Math.max(current.from + historyData.step, at)} : {from:Math.min(current.to - historyData.step, at),to:current.to});
  }));
  buttons.forEach(button => button.addEventListener("click", () => {
    const key = button.dataset.series;
    if (latencyView.hidden.has(key)) latencyView.hidden.delete(key); else latencyView.hidden.add(key);
    draw();
  }));
  let drag = null;
  windowHandle.addEventListener("pointerdown", event => {
    if (event.button !== 0) return;
    drag = {id:event.pointerId,x:event.clientX,range:latencyRange(historyData, latencyView.window),duration:historyData.to - historyData.from,width:track.clientWidth};
    windowHandle.setPointerCapture(event.pointerId); windowHandle.focus({preventScroll:true});
  });
  windowHandle.addEventListener("pointermove", event => {
    if (!drag || drag.id !== event.pointerId || !drag.width) return;
    const offset = (event.clientX - drag.x) / drag.width * drag.duration;
    setWindow({from:drag.range.from + offset,to:drag.range.to + offset});
  });
  for (const name of ["pointerup","pointercancel","lostpointercapture"]) windowHandle.addEventListener(name, () => { drag = null; });
  windowHandle.addEventListener("keydown", event => {
    if (!["ArrowLeft","ArrowRight","Home","End"].includes(event.key)) return;
    event.preventDefault();
    const range = latencyRange(historyData, latencyView.window), span = range.to - range.from;
    const shift = Math.max(historyData.step, (historyData.to - historyData.from) / 100);
    const from = event.key === "Home" ? historyData.from : event.key === "End" ? historyData.to - span : range.from + (event.key === "ArrowLeft" ? -shift : shift);
    setWindow({from,to:from + span});
  });
  track.addEventListener("dblclick", () => setWindow(null));
  el.updateHistory = (nextPoints, nextWidth) => { points = nextPoints; width = nextWidth; draw(); };
  draw();
  return el;
}
function drawHistory() {
  if (!historyData || !detailId) return;
  const root = $("#charts"), points = activeTab === "resources" ? historyData.resources : historyData.latency;
  chartWidth = Math.max(280, root.clientWidth);
  if (!points?.length) { root.innerHTML = '<p class="chart-empty">暂无数据</p>'; return; }
  const node = payload?.nodes?.find(n => n.id === detailId);
  if (activeTab === "resources") {
    // Build detached, then swap once. Emptying before chart() reads layout would
    // collapse the document and clamp mobile scrollY to zero during refresh.
    root.replaceChildren(
      chart("CPU", points, [{key:"cpu",label:"CPU",format:pct}]),
      chart("内存 · " + capacity(node?.mem_total), points, [{key:"mem_used",label:"内存",format:capacity}], node?.mem_total),
      chart("网络速率", points, [{key:"net_rx",label:"下载",format:rate},{key:"net_tx",label:"上传",format:rate}]),
      chart("硬盘 · " + capacity(node?.disk_total), points, [{key:"disk_used",label:"硬盘",format:capacity}], node?.disk_total)
    );
  } else {
    const current = root.querySelector(".latency-chart");
    if (current?.dataset.scope === latencyScope()) current.updateHistory(points, chartWidth);
    else root.replaceChildren(latencyChart(points, chartWidth));
  }
}
function resizeHistory() {
  if (detailId && Math.max(280, $("#charts").clientWidth) !== chartWidth) drawHistory();
}
async function loadHistory() {
  if (!detailId) return;
  historyRequest?.abort();
  const request = new AbortController(); historyRequest = request;
  if (!historyData) $("#charts").innerHTML = '<p class="chart-empty">加载中…</p>';
  try {
    const response = await fetch("/api/nodes/" + encodeURIComponent(detailId) + "/history?hours=" + hours + "&kind=" + activeTab, {cache:"no-store",signal:request.signal});
    if (!response.ok) throw new Error(response.status === 404 ? "节点不存在或已停用" : "历史读取失败，请稍后重试");
    const data = await response.json();
    if (stopped || request !== historyRequest || request.signal.aborted) return;
    historyData = data;
    drawHistory();
  } catch (error) {
    if (stopped || error.name === "AbortError") return;
    // A transient background failure must not remove the history being viewed.
    if (!historyData) $("#charts").innerHTML = '<p class="chart-empty">' + escapeHtml(error.message) + "</p>";
  }
}
for (const button of document.querySelectorAll("[data-tab],[data-hours]")) button.addEventListener("click", () => {
  const selector = button.dataset.tab ? "[data-tab]" : "[data-hours]";
  for (const el of document.querySelectorAll(selector)) { el.classList.toggle("selected", el === button); el.setAttribute("aria-pressed", String(el === button)); }
  if (button.dataset.tab) activeTab = button.dataset.tab; else hours = Number(button.dataset.hours);
  $("#latency-options").hidden = activeTab !== "latency";
  historyData = null; latencyView.window = null; chartSelections.clear(); loadHistory();
});
$("#latency-clip").addEventListener("change", event => { latencyView.clip = event.target.checked; drawHistory(); });
let resizeTimer;
addEventListener("resize", () => { clearTimeout(resizeTimer); resizeTimer = setTimeout(resizeHistory, 150); }, {signal:lifetime.signal});

function setConnection(mode, label) { $("#connection").className = "connection " + mode; $("#connection span").textContent = label; }
function accept(data, mode) {
  if (!data || !Array.isArray(data.nodes) || !Number.isFinite(data.generated_at)) return;
  if (payload && data.generated_at < payload.generated_at) return;
  if (!payload || data.generated_at > payload.generated_at) receivedAt = Date.now();
  recordSpeed(data); payload = data; render(); setConnection(isStale() ? "offline" : "online", isStale() ? "数据过期" : mode);
}
async function poll() {
  if (polling || stopped) return;
  polling = true;
  try {
    const response = await fetch("/api/nodes", {cache:"no-store",signal:AbortSignal.any([lifetime.signal,AbortSignal.timeout(6000)])});
    if (!response.ok) throw new Error();
    const data = await response.json();
    if (!stopped) accept(data, "轮询中");
  } catch { if (!stopped) setConnection("offline", "已断开"); } finally { polling = false; }
}
function startFallback() { if (!fallbackTimer) { poll(); fallbackTimer = setInterval(poll, 5000); } }
function connect() {
  if (stopped) return;
  try { socket = new WebSocket((location.protocol === "https:" ? "wss://" : "ws://") + location.host + "/api/ws"); }
  catch { startFallback(); reconnectTimer = setTimeout(connect, 5000); return; }
  const current = socket;
  current.addEventListener("message", event => {
    if (stopped || current !== socket) return;
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
const historyTimer = setInterval(() => { if (detailId && !document.hidden) loadHistory(); }, 30000);
function dispose() {
  stopped = true; lifetime.abort(); historyRequest?.abort(); clearTimeout(resizeTimer); clearInterval(fallbackTimer); clearInterval(freshnessTimer); clearInterval(historyTimer); clearTimeout(reconnectTimer); socket?.close();
}
function route(pathname) {
  let next = null;
  if (pathname.startsWith("/node/")) {
    try { next = decodeURIComponent(pathname.slice(6)); } catch { next = pathname.slice(6); }
  }
  if (next === detailId) return;
  detailId = next; detailCapacity = null; historyData = null; latencyView.window = null; historyRequest?.abort(); chartSelections.clear();
  $("#home-view").hidden = !!detailId; $("#detail-view").hidden = !detailId;
  $("#charts").replaceChildren(); $("#node-detail").replaceChildren();
  render(); loadHistory();
}
startFallback(); connect(); loadHistory();
return {kind:"status", route, dispose};
}
