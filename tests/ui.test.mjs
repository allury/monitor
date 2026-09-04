import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../src/ui/app.js', import.meta.url), 'utf8').replace(/^export function mountStatus\(\) \{\r?\n/, '');
function context(pathname = '/') {
  const elements = new Map();
  const context = vm.createContext({document:{querySelector:selector => {
    if (!elements.has(selector)) elements.set(selector, {hidden:false,innerHTML:'',clientWidth:640,classList:{toggle(){}},replaceChildren(){},append(){}});
    return elements.get(selector);
  }},location:{pathname},AbortController});
  vm.runInContext(source.slice(0, source.indexOf('for (const button of')), context);
  return context;
}
test('formatting distinguishes missing, zero and real values', () => {
  const sandbox = context();
  assert.equal(vm.runInContext('bytes(null)', sandbox), '—');
  assert.equal(vm.runInContext('bytes(0)', sandbox), '0 B');
  assert.equal(vm.runInContext('bytes(1024)', sandbox), '1 KiB');
  assert.equal(vm.runInContext('rate(undefined)', sandbox), '—');
  assert.equal(vm.runInContext('rate(1500)', sandbox), '1.5 KB/s');
  assert.equal(vm.runInContext('capacity(1500000000)', sandbox), '1.5 GB');
  assert.equal(vm.runInContext('transfer(0)', sandbox), '0 MB');
  assert.equal(vm.runInContext('pct(0)', sandbox), '0.0%');
  assert.equal(vm.runInContext('ratio(undefined, 100)', sandbox), null);
});
test('monthly markup and binary units are unchanged while resource percentages are integers', () => {
  const sandbox = context();
  const html = vm.runInContext('card({name:"n",os:"Debian GNU/Linux 12 (bookworm)",virtualization:"kvm",arch:"x86_64",mem_total:1000000000,disk_total:1000000000,metrics:{mem_used:294000000,disk_used:191000000},month_rx:1048576,month_tx:2097152,total_rx:1000000000,total_tx:2000000000})', sandbox);
  assert.ok(html.includes('Debian 12 · kvm · x86_64'));
  assert.ok(html.includes('<b>29%</b>') && html.includes('<b>19%</b>'));
  assert.ok(html.includes('<div class="metric"><div class="metric-label"><span>本月流量</span></div><strong class="month-value">3 MiB</strong><span class="metric-sub">↓ 1 MiB · ↑ 2 MiB</span></div>'));
  assert.ok(html.includes('1 GB'));
});
test('system labels shorten release descriptions without inventing virtualization or expiry dates', () => {
  const sandbox = context();
  assert.equal(vm.runInContext('systemLabel({os:"Debian GNU/Linux 12 (bookworm)",virtualization:"virtualized",arch:"x86_64"})', sandbox), 'Debian 12 · vm · x86_64');
  assert.equal(vm.runInContext('systemLabel({os:"Ubuntu 24.04.3 LTS",virtualization:"unknown"})', sandbox), 'Ubuntu 24.04 · unknown');
  assert.equal(vm.runInContext('expiryLabel(null)', sandbox), '∞');
  assert.equal(vm.runInContext('expiryLabel("not-a-date")', sandbox), '∞');
  assert.equal(vm.runInContext('expiryLabel(172800, 0)', sandbox), '2 天后到期');
});
test('live speed trend is bounded, ignores duplicate snapshots and breaks across gaps', () => {
  const sandbox = context();
  vm.runInContext('for(let i=0;i<100;i++) recordSpeed({generated_at:1000+i*2,nodes:[]}); recordSpeed({generated_at:1198,nodes:[]})', sandbox);
  assert.equal(vm.runInContext('speedSamples.length', sandbox), 61);
  vm.runInContext('recordSpeed({generated_at:1220,nodes:[]})', sandbox);
  assert.ok(vm.runInContext('speedTrend()', sandbox).includes('M298.00'));
});
test('pending speed samples stay empty without instructions or fabricated readings', () => {
  const sandbox = context();
  assert.equal(vm.runInContext('speedTrend()', sandbox), '');
  vm.runInContext('recordSpeed({generated_at:1000,nodes:[]})', sandbox);
  assert.equal(vm.runInContext('speedTrend()', sandbox), '');
});
test('public templates omit explanatory blocks while retaining navigation and metadata', () => {
  const template = readFileSync(new URL('../src/ui/index.html', import.meta.url), 'utf8');
  for (const text of ['history-note','site-description','back-link','等待实时采样','等待节点数据']) assert.ok(!template.includes(text), text);
  for (const text of ['history-note','每 30 秒检测一轮','TCP 建连时间','历史保留 30 天','当前按 ','添加节点后']) assert.ok(!source.includes(text), text);
  assert.ok(template.includes('class="brand" href="/"'));
  assert.ok(template.includes('meta name="description"'));
  assert.ok(template.includes('id="charts"'));
});
test('empty home keeps a concise state, metadata and optional custom footer', () => {
  const sandbox = context();
  vm.runInContext('payload={generated_at:Date.now()/1000,nodes:[],site:{name:"Monitor",description:"Custom metadata",footer:"Custom footer"}};receivedAt=Date.now();render()', sandbox);
  assert.equal(vm.runInContext('document.querySelector("#nodes").innerHTML', sandbox), '<div class="empty-card"><h2>暂无节点</h2></div>');
  assert.equal(vm.runInContext('document.querySelector(\'meta[name="description"]\').content', sandbox), 'Custom metadata');
  assert.equal(vm.runInContext('document.querySelector(".footer").textContent', sandbox), 'Custom footer');
  assert.equal(vm.runInContext('document.querySelector(".footer").hidden', sandbox), false);
});
test('details use compact system labels without removing monthly or other metric values', () => {
  const sandbox = context('/node/n');
  vm.runInContext('detail({name:"n",os:"Debian GNU/Linux 12 (bookworm)",kernel:"6.1.0",virtualization:"virtualized",arch:"x86_64",metrics:{processes:111,tcp:41,udp:3,swap_used:0},month_rx:1048576,month_tx:2097152,swap_total:0})', sandbox);
  const html = vm.runInContext('document.querySelector("#node-detail").innerHTML', sandbox);
  assert.ok(html.includes('Debian 12 · 6.1.0'));
  assert.ok(html.includes('x86_64 · vm · 111 进程'));
  assert.ok(html.includes('<dt>本月流量</dt><dd>↓ 1 MiB · ↑ 2 MiB</dd>'));
  assert.ok(html.includes('TCP 41 · UDP 3'));
  assert.ok(html.includes('<dt>Swap</dt><dd>0 B / 0 B</dd>'));
  assert.ok(!html.includes('bookworm') && !html.includes('GNU/Linux') && !html.includes('UTC'));
});
test('latency headings stay compact but selections remain scoped to the configured target', () => {
  const sandbox = context('/node/n');
  vm.runInContext('const calls=[];chart=(title,points,series,maximum,selectionKey)=>{calls.push({title,selectionKey});return {};};activeTab="latency";historyData={latency:[{at:100}],targets:{telecom:"old.example:80",unicom:"u.example:80",mobile:"m.example:80"}};drawHistory();historyData.targets.telecom="new.example:80";drawHistory()', sandbox);
  const calls = JSON.parse(vm.runInContext('JSON.stringify(calls)', sandbox));
  assert.deepEqual(calls.map(call=>call.title), ['电信','联通','移动','电信','联通','移动']);
  assert.equal(calls[0].selectionKey, 'telecom:old.example:80');
  assert.equal(calls[3].selectionKey, 'telecom:new.example:80');
});
test('chart hit testing does not invent readings inside missing history', () => {
  const sandbox = context();
  assert.equal(vm.runInContext('chartHit([{at:100},{at:160},{at:400}], 162, 60)', sandbox), 1);
  assert.equal(vm.runInContext('chartHit([{at:100},{at:160},{at:400}], 280, 60)', sandbox), -1);
  assert.equal(vm.runInContext('nearestPoint([], 0)', sandbox), -1);
});
test('chart readings omit interaction hints and zero-failure clutter but retain real failures', () => {
  const sandbox = context();
  vm.runInContext('const series={key:"telecom",label:"电信",format:v=>Number.isFinite(v)?v.toFixed(1)+" ms":"失败",failure:"telecom_failures"}', sandbox);
  assert.equal(vm.runInContext('chartReading({telecom:32,telecom_failures:0,count:4},series)', sandbox), '电信：32.0 ms');
  assert.equal(vm.runInContext('chartReading({telecom:32,telecom_failures:1,count:4},series)', sandbox), '电信：32.0 ms · 失败 1/4');
  assert.equal(vm.runInContext('chartReading({telecom:null,telecom_failures:4,count:4},series)', sandbox), '电信：失败 4/4');
  assert.equal(vm.runInContext('chartReading({cpu:0},{key:"cpu",label:"CPU",format:pct})', sandbox), 'CPU：0.0%');
  assert.ok(!source.includes('已固定'));
});
test('untrusted node text is escaped in cards and details', () => {
  const sandbox = context();
  const html = vm.runInContext('card({name:"<img src=x onerror=alert(1)>",os:"<script>bad</script>",cpu_cores:1,metrics:{},month_rx:0,month_tx:0,total_rx:0,total_tx:0})', sandbox);
  assert.ok(html.includes('&lt;img'));
  assert.ok(!html.includes('<img'));
  assert.ok(!html.includes('<script>'));
});
test('stale browser data never continues to label nodes online', () => {
  const sandbox = context();
  vm.runInContext('payload={generated_at:Date.now()/1000}; receivedAt=Date.now()-13000', sandbox);
  assert.equal(vm.runInContext('isOnline({online:true,last_seen:Date.now()/1000})', sandbox), false);
  assert.ok(vm.runInContext('status({online:true})', sandbox).includes('数据过期'));
});
test('page-level update timestamp is removed without removing node freshness checks', () => {
  const template = readFileSync(new URL('../src/ui/index.html', import.meta.url), 'utf8');
  assert.ok(!template.includes('updated-at'));
  assert.ok(!source.includes('updated-at'));
  assert.ok(source.includes('isStale()'));
});
test('malformed detail links do not crash the page', () => {
  assert.equal(vm.runInContext('detailId', context('/node/%E0%A4%A')), '%E0%A4%A');
  assert.equal(vm.runInContext('detailId', context('/node/hk-1')), 'hk-1');
});
