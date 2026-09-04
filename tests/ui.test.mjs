import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../src/ui/app.js', import.meta.url), 'utf8').replace(/^export function mountStatus\(\) \{\r?\n/, '');
function context(pathname = '/') {
  const elements = new Map();
  const context = vm.createContext({document:{querySelector:selector => {
    if (!elements.has(selector)) elements.set(selector, {hidden:false,innerHTML:'',clientWidth:640,classList:{toggle(){}},querySelector(){return null;},replaceChildren(){},append(){}});
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
test('system labels shorten release descriptions and OS marks replace unsupported expiry', () => {
  const sandbox = context();
  assert.equal(vm.runInContext('systemLabel({os:"Debian GNU/Linux 12 (bookworm)",virtualization:"virtualized",arch:"x86_64"})', sandbox), 'Debian 12 · vm · x86_64');
  assert.equal(vm.runInContext('systemLabel({os:"Ubuntu 24.04.3 LTS",virtualization:"unknown"})', sandbox), 'Ubuntu 24.04 · unknown');
  for (const [os, mark] of [['Debian GNU/Linux 12','debian'],['Ubuntu 24.04 LTS','ubuntu'],['Alpine Linux v3.24','alpine'],['Rocky Linux 9','linux'],['','linux']]) {
    sandbox.os = os;
    assert.ok(vm.runInContext('osMark(os)', sandbox).includes('os-' + mark));
  }
  assert.ok(!source.includes('expiryLabel'));
  const icons = readFileSync(new URL('../src/ui/os-icons.css', import.meta.url), 'utf8');
  assert.ok(icons.includes('data:image/svg+xml'));
  assert.ok(!icons.includes('https://'));
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
test('latency renders once, reuses its controls and scopes selections to all configured targets', () => {
  const sandbox = context('/node/n');
  vm.runInContext(`const calls=[]; let current=null;
    const root=document.querySelector('#charts'); root.querySelector=()=>current;
    root.replaceChildren=(node)=>{current=node;calls.push('swap');};
    latencyChart=()=>({dataset:{scope:latencyScope()},updateHistory:()=>calls.push('update')});
    activeTab='latency';historyData={latency:[{at:100}],targets:{telecom:'old.example:80'}};
    drawHistory(); const oldScope=current.dataset.scope; drawHistory();
    historyData.targets.telecom='new.example:80';drawHistory();`, sandbox);
  assert.equal(vm.runInContext('JSON.stringify(calls)', sandbox), '["swap","update","swap"]');
  assert.notEqual(vm.runInContext('oldScope', sandbox), vm.runInContext('current.dataset.scope', sandbox));
  assert.equal(vm.runInContext('latencySeries.map(s=>s.label).join(",")', sandbox), '电信,联通,移动');
});

test('history is built before one atomic swap and height-only resizes do not redraw', () => {
  const sandbox = context('/node/n');
  vm.runInContext(`const operations=[];const root=document.querySelector('#charts');
    root.replaceChildren=(...nodes)=>operations.push('swap:'+nodes.length);
    chart=()=>{operations.push('build');return {};};historyData={resources:[{at:100}]};
    drawHistory();resizeHistory();`, sandbox);
  assert.equal(vm.runInContext('operations.join(",")', sandbox), 'build,build,build,build,swap:4');
  vm.runInContext('root.clientWidth=390;resizeHistory()', sandbox);
  assert.equal(vm.runInContext('operations.filter(x=>x==="swap:4").length', sandbox), 2);
  assert.ok(!source.includes('setTimeout(drawHistory, 150)'));
});

test('zoom uses absolute time, clamps expired windows and never exceeds the query range', () => {
  const sandbox = context();
  for (const [window, expected] of [[null,{from:100,to:700}],[{from:300,to:400},{from:300,to:400}],[{from:0,to:200},{from:100,to:300}],[{from:650,to:850},{from:500,to:700}],[{from:300,to:301},{from:300,to:330}]]) {
    sandbox.window = window;
    assert.deepEqual(JSON.parse(vm.runInContext('JSON.stringify(latencyRange({from:100,to:700,step:30},window))', sandbox)), expected);
  }
});

test('peak clipping only caps the viewport per carrier and preserves raw readings and failures', () => {
  const sandbox = context();
  vm.runInContext(`const points=Array.from({length:20},(_,i)=>Object.freeze({telecom:i===19?1200:i===0?null:30+i%3,unicom:200+i%3,mobile:0}));
    const full=latencyScale(points,latencySeries,false), clipped=latencyScale(points,latencySeries,true);`, sandbox);
  assert.ok(vm.runInContext('full.max>=1200 && clipped.max<1200 && clipped.max>=202 && clipped.min===0', sandbox));
  assert.equal(vm.runInContext('chartReading(points[19],latencySeries[0])', sandbox), '电信：1200 ms');
  assert.equal(vm.runInContext('chartValue(points[0],latencySeries[0])', sandbox), null);
  assert.ok(vm.runInContext('latencyScale([],latencySeries,true).max>0', sandbox));
  assert.ok(vm.runInContext('latencyScale([{telecom:0}],latencySeries,true).max>0', sandbox));
  assert.ok(vm.runInContext('latencyScale(points,[],true).ticks.every(Number.isFinite)', sandbox));
});

test('background history errors preserve existing charts instead of collapsing the document', async () => {
  const sandbox = context('/node/n');
  sandbox.fetch = async () => {throw new Error('network unavailable');};
  vm.runInContext('historyData={resources:[{at:100}]};document.querySelector("#charts").innerHTML="existing";', sandbox);
  await vm.runInContext('loadHistory()', sandbox);
  assert.equal(vm.runInContext('document.querySelector("#charts").innerHTML', sandbox), 'existing');
});

test('combined renderer keeps three line styles, null gaps and shared keyboard selection without inline styles', () => {
  const sandbox = context();
  const element = () => ({style:{},hidden:false,attributes:{},clientWidth:640,offsetWidth:150,
    setAttribute(key,value){this.attributes[key]=value;},removeAttribute(key){delete this.attributes[key];}});
  const svg = element(), tooltip = element(), cursor = element(), line = element(), dots = [element(),element(),element()];
  svg.events = {}; svg.addEventListener = (key,fn)=>{svg.events[key]=fn;};
  svg.getBoundingClientRect = ()=>({left:0,width:640});
  const block = element();
  block.querySelector = key=>({'svg':svg,'.chart-tooltip':tooltip,'.chart-cursor':cursor,'.cursor-line':line}[key]);
  block.querySelectorAll = ()=>dots;
  sandbox.document.createElement = ()=>block;
  vm.runInContext(`historyData={from:100,to:400,step:100};
    const points=[{at:100,telecom:20,unicom:30,mobile:40},{at:200,telecom:null,unicom:31,mobile:0},{at:300,telecom:22,unicom:32,mobile:42}];
    chart('网络延迟',points,latencySeries,undefined,'combined',{width:640,latency:true,format:milliseconds});`, sandbox);
  for (const style of ['primary','secondary','tertiary']) assert.ok(block.innerHTML.includes('line '+style));
  assert.equal((block.innerHTML.match(/class="line primary"/g)||[]).length,2,'Null points must split the line');
  assert.ok(!block.innerHTML.includes('class="area"') && !block.innerHTML.includes(' style='));
  assert.equal(svg.style.height,'400px');
  svg.events.keydown({key:'ArrowRight',preventDefault(){}});
  assert.ok(tooltip.textContent.includes('联通：31 ms') && tooltip.textContent.includes('移动：0 ms'));
  assert.ok(!tooltip.textContent.includes('电信：') && !tooltip.textContent.includes('失败'));
  assert.equal(vm.runInContext('chartSelections.get("combined")',sandbox),200);
  svg.events.keydown({key:'ArrowRight',preventDefault(){}});
  for (const label of ['电信：22 ms','联通：32 ms','移动：42 ms']) assert.ok(tooltip.textContent.includes(label));
  svg.events.keydown({key:'Escape',preventDefault(){}});
  assert.equal(tooltip.hidden,true);
  assert.equal(vm.runInContext('chartSelections.has("combined")',sandbox),false);
});
test('chart hit testing does not invent readings inside missing history', () => {
  const sandbox = context();
  assert.equal(vm.runInContext('chartHit([{at:100},{at:160},{at:400}], 162, 60)', sandbox), 1);
  assert.equal(vm.runInContext('chartHit([{at:100},{at:160},{at:400}], 280, 60)', sandbox), -1);
  assert.equal(vm.runInContext('nearestPoint([], 0)', sandbox), -1);
});
test('latency failures are gaps without failure tooltips; valid zero readings remain selectable', () => {
  const sandbox = context();
  vm.runInContext('const series={key:"telecom",label:"电信",format:v=>Number.isFinite(v)?v.toFixed(1)+" ms":"失败",failure:"telecom_failures"}', sandbox);
  assert.equal(vm.runInContext('chartReading({telecom:32,telecom_failures:0,count:4},series)', sandbox), '电信：32.0 ms');
  assert.equal(vm.runInContext('chartReading({telecom:32,telecom_failures:1,count:4},series)', sandbox), '电信：32.0 ms');
  assert.equal(vm.runInContext('chartReading({telecom:null,telecom_failures:4,count:4},series)', sandbox), null);
  assert.equal(vm.runInContext('chartReading({telecom:-1,count:1},series)', sandbox), null);
  assert.equal(vm.runInContext('chartReading({telecom:0,count:1},series)', sandbox), '电信：0.0 ms');
  assert.equal(vm.runInContext('chartReading({cpu:0},{key:"cpu",label:"CPU",format:pct})', sandbox), 'CPU：0.0%');
  assert.ok(!source.includes('已固定'));
  assert.ok(!source.includes('class="failure"'));
});
test('latency statistics are optional and weighted by individual samples, not buckets', () => {
  const sandbox = context();
  const html = vm.runInContext('latencyStats([{count:1,telecom_failures:1},{count:9,telecom_failures:0}],{label:"电信",failure:"telecom_failures"})', sandbox);
  assert.ok(html.startsWith('<details class="latency-stats">'));
  assert.ok(html.includes('失败率</dt><dd>10.0%'));
  assert.ok(html.includes('检测次数</dt><dd>10'));
  assert.ok(html.includes('成功次数</dt><dd>9'));
  assert.ok(!html.includes(' open'));
  assert.ok(vm.runInContext('latencyStats([],{label:"电信",failure:"telecom_failures"})', sandbox).includes('失败率</dt><dd>—'));
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
