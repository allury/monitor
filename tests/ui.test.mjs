import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../src/ui/app.js', import.meta.url), 'utf8');
function context() {
  const context = vm.createContext({document:{querySelector:() => ({hidden:false})},location:{pathname:'/'}});
  vm.runInContext(source.slice(0, source.indexOf('for (const button of')), context);
  return context;
}
test('formatting distinguishes missing, zero and real values', () => {
  const sandbox = context();
  assert.equal(vm.runInContext('bytes(null)', sandbox), '—');
  assert.equal(vm.runInContext('bytes(0)', sandbox), '0 B');
  assert.equal(vm.runInContext('bytes(1024)', sandbox), '1 KiB');
  assert.equal(vm.runInContext('rate(undefined)', sandbox), '—');
  assert.equal(vm.runInContext('pct(0)', sandbox), '0.0%');
  assert.equal(vm.runInContext('ratio(undefined, 100)', sandbox), null);
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
