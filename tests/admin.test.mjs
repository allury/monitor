import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../src/ui/admin.js', import.meta.url), 'utf8').replace(/^export function mountAdmin\(\) \{\r?\n/, '');
function context(status = 200, response = {id:'n',name:'Node',token:null,token_status:'hash_only'}) {
  const elements = new Map();
  const element = selector => {
    if (!elements.has(selector)) elements.set(selector, {textContent:'',value:'',type:'password',hidden:false,open:false,
      classList:{add(){},remove(){}},setAttribute(){},showModal(){this.open=true;},close(){this.open=false;}});
    return elements.get(selector);
  };
  const sandbox = vm.createContext({
    document:{querySelector:element}, window:{location:{origin:'https://monitor.example.com'}},
    sessionStorage:{getItem:() => 'fixture',removeItem(){}}, AbortController, AbortSignal, DOMException,
    fetch:async () => ({status,ok:status>=200&&status<300,json:async () => response}),
  });
  vm.runInContext(source.slice(0,source.indexOf('document.querySelector("#login-form").addEventListener')),sandbox);
  return {sandbox,element};
}
test('credential fields fall back safely without treating hashes or masks as tokens', () => {
  const {sandbox} = context();
  for (const field of ['token','secret','client_secret','key']) {
    assert.equal(vm.runInContext(`nodeToken({${field}:"n.fixture"})`,sandbox),'n.fixture');
  }
  assert.equal(vm.runInContext('nodeToken({token:"",secret:"n.legacy"})',sandbox),'n.legacy');
  assert.equal(vm.runInContext('nodeToken({token:"******",secret:"n.legacy"})',sandbox),'n.legacy');
  assert.equal(vm.runInContext('nodeToken({token_hash:"abcdef"})',sandbox),'');
  assert.equal(vm.runInContext('nodeToken({token:"abcdef",token_status:"hash_only"})',sandbox),'');
  assert.equal(vm.runInContext('nodeToken({token:"[redacted]",key:42})',sandbox),'');
});
test('update command is visible even when credential lookup fails', async () => {
  const {sandbox,element} = context(500,{error:'暂时无法读取'});
  await vm.runInContext('openCommands({id:"n",name:"Node"})',sandbox);
  assert.ok(element('#node-dialog').open);
  const command = element('#node-update-command').textContent;
  assert.ok(command.endsWith('--update'));
  assert.ok(!command.includes('--token') && !command.includes('--server'));
  assert.ok(element('#credential-controls').hidden);
  assert.ok(element('#node-command-error').textContent.includes('更新命令仍可复制'));
});
test('hash-only nodes keep update controls while fresh credentials are cleared on close', async () => {
  const {sandbox,element} = context();
  await vm.runInContext('openCommands({id:"n",name:"Node"})',sandbox);
  assert.ok(element('#node-update-command').textContent.includes('--update'));
  assert.ok(element('#node-token-note').textContent.includes('不可逆摘要'));
  await vm.runInContext('openCommands({id:"n",name:"Node",token:"n.fixture"}, false)',sandbox);
  assert.equal(element('#node-token').value,'n.fixture');
  assert.equal(element('#node-token').type,'password');
  assert.ok(element('#node-install-command').textContent.includes("--server 'https://monitor.example.com' --token 'n.fixture'"));
  vm.runInContext('clearCommands()',sandbox);
  assert.equal(element('#node-token').value,'');
  assert.equal(element('#node-install-command').textContent,'');
});
