export function mountAdmin() {
const lifetime = new AbortController();
let alive = true, commandNode = null, commandRequest = null;
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

function shellQuote(value) {
  return `'${String(value).replaceAll("'", `'"'"'`)}'`;
}

function agentInstallCommand(token) {
  const installer = "https://github.com/allury/monitor/releases/latest/download/install-agent.sh";
  return `curl -fsSL ${installer} | sudo sh -s -- --server ${shellQuote(window.location.origin)} --token ${shellQuote(token)}`;
}

function agentUpdateCommand() {
  return "curl -fsSL https://github.com/allury/monitor/releases/latest/download/install-agent.sh | sudo sh -s -- --update";
}

function nodeToken(node) {
  if (["hash_only", "encrypted", "redacted"].includes(node?.token_status)) return "";
  for (const key of ["token", "secret", "client_secret", "key"]) {
    const value = node?.[key];
    if (typeof value !== "string") continue;
    const token = value.trim();
    if (token && !/[*•●…]/.test(token) && !/^(?:\[?redacted\]?|\[?masked\]?|null|undefined)$/i.test(token)) return token;
  }
  return "";
}

function clearCommands() {
  commandRequest?.abort(); commandRequest = null; commandNode = null;
  document.querySelector("#node-token").value = "";
  document.querySelector("#node-token").type = "password";
  document.querySelector("#node-install-command").textContent = "";
  document.querySelector("#credential-controls").hidden = true;
}

function renderCredentials(node) {
  commandNode = node;
  const token = nodeToken(node);
  document.querySelector("#node-token").value = token;
  document.querySelector("#node-token").type = "password";
  document.querySelector("#toggle-node-token").textContent = "查看密钥";
  document.querySelector("#toggle-node-token").setAttribute("aria-pressed", "false");
  document.querySelector("#credential-controls").hidden = !token;
  document.querySelector("#node-install-command").textContent = token ? agentInstallCommand(token) : "";
  document.querySelector("#node-token-note").textContent = token
    ? "密钥仅在创建或重置时显示。请保存首次安装命令，关闭后不会在主控保留明文。"
    : "主控只保存不可逆摘要，无法再次显示原密钥。已安装探针可直接使用上方更新命令；新机器安装且未保存原密钥时，才需要重置。";
}

async function openCommands(node, lookup = true) {
  clearCommands(); commandNode = node;
  const dialog = document.querySelector("#node-dialog");
  document.querySelector("#command-node-name").textContent = (node.name || node.id) + " · 安装与更新";
  // Updating an existing installation never depends on a plaintext credential or API success.
  document.querySelector("#node-update-command").textContent = agentUpdateCommand();
  document.querySelector("#node-command-error").textContent = "";
  document.querySelector("#reset-node-token").disabled = false;
  renderCredentials(node);
  if (!dialog.open) dialog.showModal();
  if (!lookup || nodeToken(node)) return;
  const request = new AbortController(); commandRequest = request;
  try {
    const detail = await api(`/nodes/${encodeURIComponent(node.id)}`, {signal:request.signal});
    if (commandRequest === request && dialog.open) renderCredentials(detail);
  } catch (error) {
    if (error.name !== "AbortError" && alive && dialog.open) document.querySelector("#node-command-error").textContent = error.message + "。更新命令仍可复制；它不会更改密钥或恢复已停用的节点。";
  }
}

async function copyText(value, message) {
  if (!value) return;
  try {
    if (!navigator.clipboard || !window.isSecureContext) throw new Error();
    await navigator.clipboard.writeText(value);
  } catch {
    if (!alive) return;
    const input = document.createElement("textarea"), focused = document.activeElement;
    input.value = value; input.className = "sr-only";
    const dialog = document.querySelector("#node-dialog");
    (dialog.open ? dialog : document.body).append(input);
    input.select();
    let copied = false;
    try { copied = document.execCommand("copy"); } catch { /* 提示用户手动复制 */ }
    input.remove(); focused?.focus();
    if (!copied) { toast("无法自动复制，请选择文字手动复制；密钥可先点击查看"); return; }
  }
  toast(message);
}

async function api(path, options = {}) {
  const headers = {...(options.headers || {})};
  if (session) headers.authorization = `Bearer ${session}`;
  if (options.body) headers["content-type"] = "application/json";
  const signal = AbortSignal.any([lifetime.signal, options.signal || AbortSignal.timeout(10000)]);
  const response = await fetch(`/api/admin${path}`, {...options, headers, cache:"no-store", signal});
  if (response.status === 401 && path !== "/login") showLogin();
  const body = response.status === 204 ? null : await response.json().catch(() => ({}));
  if (!alive) throw new DOMException("Page closed", "AbortError");
  if (!response.ok) throw new Error(body?.error || `请求失败 (${response.status})`);
  return body;
}

function showLogin() {
  if (!alive) return;
  session = "";
  sessionStorage.removeItem("monitor-admin-session");
  loginView.classList.remove("hidden");
  adminView.classList.add("hidden");
  logoutButton.classList.add("hidden");
  document.querySelector("#node-dialog").close(); clearCommands(); currentState = null;
  clearPasswordFields();
  document.querySelector("#password").value = "";
}

function showAdmin() {
  loginView.classList.add("hidden");
  adminView.classList.remove("hidden");
  logoutButton.classList.remove("hidden");
}

function toast(message) {
  if (!alive) return;
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
    <td><div class="node-actions"><button class="button secondary small" type="button" data-commands="${escapeHtml(node.id)}">安装 / 更新</button><button class="button danger small" type="button" data-revoke="${escapeHtml(node.id)}">停用</button></div></td>
  </tr>`).join("")}</tbody></table>`;
}

function renderState(state) {
  currentState = state;
  renderNodes(state.nodes || []);
  document.querySelector("#agent-upgrade-note").hidden = !(state.nodes || []).some(node => node.agent_version && !Number.isInteger(node.metrics?.latency_sample?.interval_seconds));
  const latency = state.settings.latency;
  document.querySelector("#latency-telecom").value = latency.telecom;
  document.querySelector("#latency-unicom").value = latency.unicom;
  document.querySelector("#latency-mobile").value = latency.mobile;
  document.querySelector("#latency-interval").value = latency.interval_seconds ?? 30;
  const site = state.settings.site;
  document.querySelector("#site-name").value = site.name;
  document.querySelector("#site-description").value = site.description;
  document.querySelector("#site-footer").value = site.footer || "";
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

function latencyFormValues() {
  const interval_seconds = Number(document.querySelector("#latency-interval").value);
  if (!Number.isInteger(interval_seconds) || interval_seconds < 10 || interval_seconds > 3600) throw new Error("检测间隔必须为 10–3600 秒");
  return {
    telecom: document.querySelector("#latency-telecom").value.trim(),
    unicom: document.querySelector("#latency-unicom").value.trim(),
    mobile: document.querySelector("#latency-mobile").value.trim(),
    interval_seconds,
  };
}

function clearPasswordFields() {
  for (const id of ["current-password", "new-password", "confirm-password"]) document.querySelector(`#${id}`).value = "";
}

function passwordFormValues() {
  const current_password = document.querySelector("#current-password").value;
  const new_password = document.querySelector("#new-password").value;
  const confirm_password = document.querySelector("#confirm-password").value;
  const length = Array.from(new_password).length;
  if (length < 15 || length > 128) throw new Error("新密码需为 15–128 个字符");
  if (new_password !== confirm_password) throw new Error("两次新密码不一致");
  if (new_password === current_password) throw new Error("新密码不能与当前密码相同");
  return {current_password, new_password, confirm_password};
}

document.querySelector("#login-form").addEventListener("submit", async event => {
  event.preventDefault();
  const button = event.currentTarget.querySelector('[type="submit"]');
  const input = document.querySelector("#password");
  if (button.disabled) return;
  button.disabled = true;
  const error = document.querySelector("#login-error");
  error.textContent = "";
  try {
    const result = await api("/login", {
      method: "POST",
      body: JSON.stringify({password: input.value}),
    });
    session = result.token;
    sessionStorage.setItem("monitor-admin-session", session);
    input.value = "";
    await loadState();
  } catch (reason) {
    if (alive) error.textContent = reason.message;
  } finally {
    input.value = "";
    button.disabled = false;
  }
});

document.querySelector("#password-form").addEventListener("submit", async event => {
  event.preventDefault();
  const button = event.currentTarget.querySelector('[type="submit"]');
  if (button.disabled) return;
  const status = document.querySelector("#password-status");
  button.disabled = true;
  status.textContent = "";
  try {
    await api("/password", {method:"PUT", body:JSON.stringify(passwordFormValues())});
    showLogin();
    document.querySelector("#login-error").textContent = "密码已修改，请重新登录";
    document.querySelector("#password").focus();
  } catch (error) {
    if (alive) status.textContent = error.message;
  } finally {
    if (alive) clearPasswordFields();
    button.disabled = false;
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
    await openCommands(result, false);
    document.querySelector("#node-id").value = "";
    document.querySelector("#node-name").value = "";
    await loadState();
    toast("节点已创建");
  } catch (error) { toast(error.message); }
});

document.querySelector("#copy-install-command").addEventListener("click", () => copyText(document.querySelector("#node-install-command").textContent, "安装命令已复制"));
document.querySelector("#copy-update-command").addEventListener("click", () => copyText(agentUpdateCommand(), "更新命令已复制"));
document.querySelector("#copy-node-token").addEventListener("click", () => copyText(nodeToken(commandNode), "密钥已复制"));
document.querySelector("#close-node-dialog").addEventListener("click", () => document.querySelector("#node-dialog").close());
document.querySelector("#node-dialog").addEventListener("close", clearCommands);
document.querySelector("#toggle-node-token").addEventListener("click", () => {
  const input = document.querySelector("#node-token"), button = document.querySelector("#toggle-node-token"), show = input.type === "password";
  input.type = show ? "text" : "password";
  button.textContent = show ? "隐藏密钥" : "查看密钥"; button.setAttribute("aria-pressed", String(show));
});
document.querySelector("#reset-node-token").addEventListener("click", async () => {
  const id = commandNode?.id;
  if (!id || !confirm(`重置节点 ${id} 的密钥？旧连接会断开，需要执行新安装命令。普通更新不需要重置。`)) return;
  const button = document.querySelector("#reset-node-token"); button.disabled = true;
  commandRequest?.abort(); commandRequest = null;
  try {
    const result = await api(`/nodes/${encodeURIComponent(id)}/token`, {method:"POST"});
    if (commandNode?.id === id && document.querySelector("#node-dialog").open) renderCredentials(result);
    await loadState(); toast("密钥已重置，请保存并执行新安装命令");
  } catch (error) { toast(error.message); }
  finally { button.disabled = false; }
});

document.querySelector("#node-list").addEventListener("click", async event => {
  const commands = event.target.closest("[data-commands]");
  if (commands) {
    const node = currentState?.nodes?.find(node => node.id === commands.dataset.commands);
    if (node) await openCommands(node);
    return;
  }
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
      body: JSON.stringify(latencyFormValues()),
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
return {kind:"admin", dispose() {
  clearCommands(); clearPasswordFields(); document.querySelector("#password").value = "";
  alive = false; lifetime.abort(); clearTimeout(toastTimer); currentState = null; session = "";
}};
}
