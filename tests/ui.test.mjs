import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../src/ui/app.js', import.meta.url), 'utf8').replace(/^export function mountStatus\(\) \{\r?\n/, '');
function context(pathname = '/') {
  const context = vm.createContext({document:{querySelector:() => ({hidden:false})},location:{pathname},AbortController});
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
test('chart hit testing does not invent readings inside missing history', () => {
  const sandbox = context();
  assert.equal(vm.runInContext('chartHit([{at:100},{at:160},{at:400}], 162, 60)', sandbox), 1);
  assert.equal(vm.runInContext('chartHit([{at:100},{at:160},{at:400}], 280, 60)', sandbox), -1);
  assert.equal(vm.runInContext('nearestPoint([], 0)', sandbox), -1);
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
test('malformed detail links do not crash the page', () => {
  assert.equal(vm.runInContext('detailId', context('/node/%E0%A4%A')), '%E0%A4%A');
  assert.equal(vm.runInContext('detailId', context('/node/hk-1')), 'hk-1');
});
