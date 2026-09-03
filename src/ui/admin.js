let session = sessionStorage.getItem("monitor-admin-session") || "";
let currentState = null;
let toastTimer = null;

const loginView = document.querySelector("#login-view");
const adminView = document.querySelector("#admin-view");
const logoutButton = document.querySelector("#logout");

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

async function api(path, options = {}) {
  const headers = {...(options.headers || {})};
  if (session) headers.authorization = `Bearer ${session}`;
  if (options.body) headers["content-type"] = "application/json";
  const response = await fetch(`/api/admin${path}`, {...options, headers});
  if (response.status === 401 && path !== "/login") showLogin();
  const body = response.status === 204 ? null : await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(body?.error || `请求失败 (${response.status})`);
  return body;
}

function showLogin() {
  session = "";
  sessionStorage.removeItem("monitor-admin-session");
  loginView.classList.remove("hidden");
  adminView.classList.add("hidden");
  logoutButton.classList.add("hidden");
}

function showAdmin() {
  loginView.classList.add("hidden");
  adminView.classList.remove("hidden");
  logoutButton.classList.remove("hidden");
}

function toast(message) {
  const element = document.querySelector("#toast");
  element.textContent = message;
  element.classList.remove("hidden");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => element.classList.add("hidden"), 2600);
}

function renderNodes(nodes) {
  const root = document.querySelector("#node-list");
  if (!nodes.length) {
    root.innerHTML = '<p class="save-status">还没有节点。</p>';
    return;
  }
  root.innerHTML = `<table class="node-table"><thead><tr><th>名称</th><th>ID</th><th>状态</th><th>操作</th></tr></thead><tbody>${nodes.map(node => `<tr>
    <td>${escapeHtml(node.name)}</td><td><code>${escapeHtml(node.id)}</code></td>
    <td><span class="inline-status ${node.online ? "online" : ""}">${node.online ? "在线" : "离线"}</span></td>
    <td><button class="button danger small" type="button" data-revoke="${escapeHtml(node.id)}">停用</button></td>
  </tr>`).join("")}</tbody></table>`;
}

function renderState(state) {
  currentState = state;
  renderNodes(state.nodes || []);
  const latency = state.settings.latency;
  document.querySelector("#latency-telecom").value = latency.telecom;
  document.querySelector("#latency-unicom").value = latency.unicom;
  document.querySelector("#latency-mobile").value = latency.mobile;
  const site = state.settings.site;
  document.querySelector("#site-name").value = site.name;
  document.querySelector("#site-description").value = site.description;
  document.querySelector("#site-footer").value = site.footer;
  document.querySelector(".brand > span:last-child").textContent = site.name;
  document.title = `管理 · ${site.name}`;
}

async function loadState() {
  try {
    const state = await api("/state");
    showAdmin();
    renderState(state);
  } catch (error) {
    if (session) toast(error.message);
  }
}

document.querySelector("#login-form").addEventListener("submit", async event => {
  event.preventDefault();
  const error = document.querySelector("#login-error");
  error.textContent = "";
  try {
    const result = await api("/login", {
      method: "POST",
      body: JSON.stringify({password: document.querySelector("#password").value}),
    });
    session = result.token;
    sessionStorage.setItem("monitor-admin-session", session);
    document.querySelector("#password").value = "";
    await loadState();
  } catch (reason) {
    error.textContent = reason.message;
  }
});

logoutButton.addEventListener("click", async () => {
  try { await api("/logout", {method: "POST"}); } catch { /* 本地退出仍然生效 */ }
  showLogin();
});

document.querySelectorAll(".tab").forEach(tab => tab.addEventListener("click", () => {
  document.querySelectorAll(".tab").forEach(item => item.classList.toggle("active", item === tab));
  document.querySelectorAll(".panel").forEach(panel => panel.classList.toggle("active", panel.id === tab.dataset.panel));
}));

document.querySelector("#node-form").addEventListener("submit", async event => {
  event.preventDefault();
  try {
    const result = await api("/nodes", {
      method: "POST",
      body: JSON.stringify({
        id: document.querySelector("#node-id").value.trim(),
        name: document.querySelector("#node-name").value.trim(),
      }),
    });
    document.querySelector("#node-token").textContent = result.token;
    document.querySelector("#token-box").classList.add("show");
    document.querySelector("#node-id").value = "";
    document.querySelector("#node-name").value = "";
    await loadState();
    toast("节点已创建");
  } catch (error) { toast(error.message); }
});

document.querySelector("#copy-token").addEventListener("click", async () => {
  const value = document.querySelector("#node-token").textContent;
  try { await navigator.clipboard.writeText(value); toast("密钥已复制"); }
  catch { toast("复制失败，请手动选择密钥"); }
});

document.querySelector("#node-list").addEventListener("click", async event => {
  const button = event.target.closest("[data-revoke]");
  if (!button) return;
  const id = button.dataset.revoke;
  if (!confirm(`停用节点 ${id}？旧数据会保留。`)) return;
  try {
    await api(`/nodes/${encodeURIComponent(id)}`, {method: "DELETE"});
    await loadState();
    toast("节点已停用");
  } catch (error) { toast(error.message); }
});

document.querySelector("#latency-form").addEventListener("submit", async event => {
  event.preventDefault();
  const status = document.querySelector("#latency-status");
  status.textContent = "保存中…";
  try {
    await api("/latency", {
      method: "PUT",
      body: JSON.stringify({
        telecom: document.querySelector("#latency-telecom").value.trim(),
        unicom: document.querySelector("#latency-unicom").value.trim(),
        mobile: document.querySelector("#latency-mobile").value.trim(),
      }),
    });
    status.textContent = "已保存并下发";
  } catch (error) { status.textContent = error.message; }
});

document.querySelector("#site-form").addEventListener("submit", async event => {
  event.preventDefault();
  const status = document.querySelector("#site-status");
  status.textContent = "保存中…";
  try {
    const site = await api("/site", {
      method: "PUT",
      body: JSON.stringify({
        name: document.querySelector("#site-name").value,
        description: document.querySelector("#site-description").value,
        footer: document.querySelector("#site-footer").value,
      }),
    });
    document.querySelector(".brand > span:last-child").textContent = site.name;
    document.title = `管理 · ${site.name}`;
    status.textContent = "已保存";
  } catch (error) { status.textContent = error.message; }
});

if (session) loadState(); else showLogin();
