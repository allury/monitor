import assert from 'node:assert/strict';
import {spawn, execFileSync} from 'node:child_process';
import {mkdtemp, readFile, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import net from 'node:net';
import {randomBytes} from 'node:crypto';

const [serverBinary, agentBinary] = process.argv.slice(2).map(value => resolve(value));
assert.ok(serverBinary && agentBinary, 'Provide the server and agent binaries');
const directory = await mkdtemp(join(tmpdir(), 'monitor-integration-'));
const database = join(directory, 'monitor.db');
const wait = ms => new Promise(r => setTimeout(r, ms));
async function until(check, message, duration = 10000) {
  const deadline = Date.now() + duration;
  while (Date.now() < deadline) { if (await check()) return; await wait(150); }
  throw new Error(message);
}
async function port() {
  const listener = net.createServer();
  await new Promise(r => listener.listen(0, '127.0.0.1', r));
  const port = listener.address().port;
  await new Promise(r => listener.close(r));
  return port;
}
const serverPort = await port();
const base = 'http://127.0.0.1:' + serverPort;
let session = '', server, agent, probes, serverOutput = '', agentOutput = '', nodeToken = '';
const sandbox = process.env.MONITOR_TEST_SANDBOX === '1';
const sandboxUnit = 'monitor-integration-' + randomBytes(8).toString('hex');
const sandboxBinary = '/run/' + sandboxUnit;
let sandboxStarted = false, sandboxInstalled = false;
const sockets = [];
function stopAgent() {
  if (sandboxStarted) {
    try { execFileSync('sudo', ['systemctl', 'stop', sandboxUnit], {stdio:'ignore',timeout:15000}); } catch { /* best-effort test cleanup */ }
    sandboxStarted = false;
  }
  agent?.kill('SIGTERM');
}
async function startAgent(token, target) {
  const arguments_ = ['--server',base,'--telecom',target,'--unicom',target,'--mobile',target];
  if (!sandbox) return spawn(agentBinary, arguments_, {env:{...process.env,MONITOR_TOKEN:token},stdio:'ignore'});
  // Exercise the installed service's restrictions, including its credential channel.
  const credential = join(directory, 'agent.token');
  await writeFile(credential, token, {mode:0o600});
  execFileSync('sudo', ['install','-m','0755',agentBinary,sandboxBinary], {stdio:'ignore'});
  sandboxInstalled = true;
  const unit = await readFile(new URL('../deploy/monitor-agent.service', import.meta.url), 'utf8');
  const properties = unit.split(/\r?\n/).filter(line => /^(DynamicUser|NoNewPrivileges|Protect\w*|Private\w*|ReadOnlyPaths|InaccessiblePaths|SystemCallFilter|Restrict\w*|LockPersonality|MemoryDenyWriteExecute|CapabilityBoundingSet|SystemCallArchitectures|UMask)=/.test(line));
  sandboxStarted = true;
  return spawn('sudo', ['systemd-run','--quiet','--wait','--pipe','--collect','--unit=' + sandboxUnit,
    ...properties.map(value => '--property=' + value), '--property=RuntimeMaxSec=100',
    '--property=LoadCredential=token:' + credential,
    '/bin/sh', '-c', 'export MONITOR_TOKEN_FILE="$CREDENTIALS_DIRECTORY/token"; exec "$@"',
    'monitor-sandbox', sandboxBinary, ...arguments_], {stdio:['ignore','pipe','pipe']});
}
async function api(path, body, method = body ? 'POST' : 'GET') {
  const response = await fetch(base + path, {method, headers: {'content-type':'application/json', authorization:'Bearer ' + session}, body: body ? JSON.stringify(body) : undefined});
  const value = response.status === 204 ? null : await response.json();
  return {status:response.status, value};
}
async function nodes() { return (await api('/api/nodes')).value.nodes; }
function ws(token) {
  return new Promise((resolve, reject) => {
    const socket = net.connect(serverPort, '127.0.0.1');
    sockets.push(socket);
    let pending = Buffer.alloc(0), ready = false;
    const timer = setTimeout(() => { socket.destroy(); reject(new Error('WebSocket handshake timeout')); }, 5000);
    const client = {socket, closed:false, send(report) {
      const data = Buffer.from(JSON.stringify(report)), mask = randomBytes(4);
      const header = Buffer.alloc(data.length < 126 ? 2 : 4);
      header[0] = 0x81;
      if (data.length < 126) header[1] = 0x80 | data.length;
      else { header[1] = 0x80 | 126; header.writeUInt16BE(data.length, 2); }
      const encoded = Buffer.from(data);
      for (let i = 0; i < encoded.length; i++) encoded[i] ^= mask[i % 4];
      socket.write(Buffer.concat([header, mask, encoded]));
    }};
    socket.on('connect', () => socket.write('GET /api/agent HTTP/1.1\r\nHost: 127.0.0.1:' + serverPort + '\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: ' + randomBytes(16).toString('base64') + '\r\nAuthorization: Bearer ' + token + '\r\n\r\n'));
    socket.on('data', data => {
      pending = Buffer.concat([pending, data]);
      if (!ready) {
        const end = pending.indexOf('\r\n\r\n'); if (end < 0) return;
        clearTimeout(timer);
        const status = Number(pending.toString('ascii', 0, end).split(' ')[1]);
        if (status !== 101) { socket.destroy(); reject(new Error('HTTP ' + status)); return; }
        pending = pending.subarray(end + 4); ready = true; resolve(client);
      }
      while (pending.length >= 2) {
        let size = pending[1] & 127, head = 2;
        if (size === 126) { if (pending.length < 4) return; size = pending.readUInt16BE(2); head = 4; }
        if (size === 127) { if (pending.length < 10) return; size = Number(pending.readBigUInt64BE(2)); head = 10; }
        if (pending.length < head + size) return;
        if ((pending[0] & 15) === 8) { client.closed = true; socket.end(); }
        pending = pending.subarray(head + size);
      }
    });
    socket.on('close', () => { client.closed = true; clearTimeout(timer); });
    socket.on('error', error => { clearTimeout(timer); if (!ready) reject(error); });
  });
}
try {
  server = spawn(serverBinary, ['--listen','127.0.0.1:' + serverPort,'--db',database], {stdio:['ignore','pipe','pipe']});
  server.stdout.on('data', chunk => serverOutput += chunk.toString());
  await until(() => serverOutput.includes('管理员密钥（仅显示一次）：'), 'Controller did not initialize');
  const password = serverOutput.match(/管理员密钥（仅显示一次）：(\S+)/)[1];
  const login = await api('/api/admin/login', {password});
  assert.equal(login.status, 200); session = login.value.token; serverOutput = '';
  const created = await api('/api/admin/nodes', {id:'test',name:'测试节点'});
  assert.equal(created.status, 201);
  assert.equal((await api('/api/admin/nodes', {id:'test',name:'duplicate'})).status, 400);
  let connections = 0;
  probes = net.createServer(socket => { connections++; socket.end(); });
  await new Promise(r => probes.listen(0, '127.0.0.1', r));
  const target = '127.0.0.1:' + probes.address().port;
  assert.equal((await api('/api/admin/latency', {telecom:target,unicom:target,mobile:target}, 'PUT')).status, 200);
  nodeToken = created.value.token;
  agent = await startAgent(nodeToken, target);
  agent.stdout?.on('data', data => agentOutput = (agentOutput + data.toString()).slice(-8000));
  agent.stderr?.on('data', data => agentOutput = (agentOutput + data.toString()).slice(-8000));
  await until(async () => (await nodes())[0]?.online, 'Agent did not become online');
  const firstSeen = (await nodes())[0].last_seen;
  await wait(63500);
  const live = (await nodes())[0];
  assert.ok(live.online && live.last_seen - firstSeen >= 55, 'Status reporting stalled');
  assert.ok(live.mem_total > 0 && live.disk_total > 0 && live.metrics.mem_used > 0, 'Read-only agent cannot collect required metrics');
  assert.ok(connections >= 9 && connections <= 12, 'TCP probes must run every 30 seconds, not every status report');
  const latency = await api('/api/nodes/test/history?kind=latency&hours=1');
  assert.equal(latency.status, 200);
  assert.ok(latency.value.latency.length >= 3 && latency.value.latency.length <= 4, 'Each new latency round must be stored exactly once');
  assert.ok(latency.value.latency.every(p => p.count === 1 && p.telecom !== null));
  assert.equal((await api('/api/nodes/test/history?hours=721')).status, 400);
  assert.equal((await api('/api/nodes/test/history?kind=unknown')).status, 400);
  const resource = await api('/api/nodes/test/history?kind=resources&hours=1');
  assert.ok(resource.value.resources.length >= 1);
  const document = await fetch(base + '/node/test');
  assert.equal(document.status, 200);
  assert.ok((await document.text()).includes('id="detail-view"'));
  assert.equal((await fetch(base + '/assets/theme.js')).headers.get('cache-control'), 'no-cache');
  const rotated = await api('/api/admin/nodes/test/token', {});
  assert.equal(rotated.status, 200);
  await assert.rejects(ws(created.value.token), /HTTP 401/);
  await until(async () => !(await nodes())[0]?.online, 'Old connection survived token rotation');
  stopAgent();
  const first = await ws(rotated.value.token);
  const second = await ws(rotated.value.token);
  await until(() => first.closed, 'New connection did not close the previous connection');
  second.send({protocol:1,agent_version:'0.2.0',boot_id:'fixture',hostname:'fixture',os:'Linux',kernel:'test',arch:'x86_64',virtualization:'unknown',cpu_name:'test',cpu_cores:1,mem_total:1024,swap_total:0,disk_total:1024,metrics:{cpu:5,load:[0,0,0],mem_used:10,swap_used:0,disk_used:10,net_rx:0,net_tx:0,net_rx_total:0,net_tx_total:0,tcp:0,udp:0,processes:1,uptime:1,latency:{telecom:null,unicom:null,mobile:null}}});
  await until(async () => (await nodes())[0]?.online, 'Previous connection cleanup removed the new connection');
  assert.equal((await api('/api/admin/nodes/test', null, 'DELETE')).status, 204);
  await until(() => second.closed, 'Revocation did not close the live connection');
  await assert.rejects(ws(rotated.value.token), /HTTP 401/);
  assert.equal((await api('/api/nodes/test/history?hours=1')).status, 404);
  for (const socket of sockets) socket.destroy();
  const exited = new Promise(r => server.once('exit', r));
  server.kill('SIGTERM');
  assert.equal(await Promise.race([exited, wait(10000).then(() => 'timeout')]), 0, 'Controller did not drain and stop cleanly');
  console.log('PASS: status cadence, 30-second probes, history, deep links, credential rotation, connection takeover, revocation, graceful shutdown' + (sandbox ? ', hardened systemd agent' : ''));
} catch (error) {
  if (agentOutput) console.error(agentOutput.replaceAll(nodeToken, '[redacted]'));
  throw error;
} finally {
  stopAgent(); server?.kill('SIGTERM');
  for (const socket of sockets) socket.destroy();
  probes?.close();
  if (sandboxInstalled) execFileSync('sudo', ['unlink', sandboxBinary], {stdio:'ignore'});
}
