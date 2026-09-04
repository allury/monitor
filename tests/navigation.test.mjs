import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../src/ui/navigation.js',import.meta.url),'utf8').replace(/^import .+;\r?\n/gm,'');
function context() {
  let surface='status',url=new URL('http://localhost/'),fail=false,error=null;
  const log={mounted:[],disposed:[],routes:[],fetches:[],scrolls:0},listeners={};
  const location={};
  for(const key of ['href','pathname','origin']) Object.defineProperty(location,key,{get:()=>url[key]});
  const mount=kind=>{log.mounted.push([kind,url.pathname]);return {kind,route:path=>log.routes.push(path),dispose:()=>log.disposed.push(kind)};};
  const meta={content:''};
  const body={className:'',replaceChildren(...nodes){surface=nodes[0].kind;},append(node){error=node;}};
  const sandbox=vm.createContext({
    URL,AbortController,AbortSignal,Event,location,
    mountStatus:()=>mount('status'),mountAdmin:()=>mount('admin'),
    history:{pushState(_s,_t,next){url=new URL(next);},replaceState(_s,_t,next){url=new URL(next);}},
    window:{scrollTo(){log.scrolls++;},addEventListener(name,callback){listeners[name]=callback;}},
    document:{body,title:'Monitor',querySelector(selector){
      if(selector==='#home-view')return surface==='status'?{}:null;
      if(selector==='#navigation-error')return error;
      if(selector.startsWith('meta'))return meta;
      return null;
    },createElement(){return {setAttribute(){},remove(){error=null;}};},importNode:node=>node,dispatchEvent(){},addEventListener(name,callback){listeners[name]=callback;}},
    DOMParser:class{parseFromString(kind){return {title:kind,body:{className:kind,childNodes:[{kind}]},querySelectorAll:()=>[],querySelector:selector=>selector==='#admin-view'?kind==='admin':selector==='#home-view'?kind==='status':meta};}},
    fetch:async path=>{log.fetches.push(path);return {ok:!fail,text:async()=>path==='/admin'?'admin':'status'};},
  });
  vm.runInContext(source,sandbox);
  return {sandbox,log,listeners,location,fail:()=>fail=true,pop:path=>url=new URL(path,'http://localhost'),getError:()=>error};
}
test('home/detail transitions retain the mounted page and avoid document fetch/reload',async()=>{
  const {sandbox,log,location}=context();
  await vm.runInContext('navigate(new URL("http://localhost/node/n"))',sandbox);
  assert.equal(location.pathname,'/node/n');
  await vm.runInContext('navigate(new URL("http://localhost/"))',sandbox);
  assert.equal(location.pathname,'/');
  assert.deepEqual(log.routes,['/node/n','/']);
  assert.equal(log.fetches.length,0);assert.equal(log.mounted.length,1);assert.equal(log.disposed.length,0);
});
test('crossing the admin boundary disposes old requests and mounts against the new URL',async()=>{
  const {sandbox,log,location,listeners,pop}=context();
  await vm.runInContext('navigate(new URL("http://localhost/admin"))',sandbox);
  assert.equal(location.pathname,'/admin');
  await vm.runInContext('navigate(new URL("http://localhost/node/n"))',sandbox);
  assert.equal(location.pathname,'/node/n');
  assert.deepEqual(log.mounted,[['status','/'],['admin','/admin'],['status','/node/n']]);
  assert.deepEqual(log.disposed,['status','admin']);
  pop('/');await listeners.popstate();assert.equal(log.routes.at(-1),'/');
});
test('failed navigation keeps the working page and reports a retryable error',async()=>{
  const {sandbox,log,fail,location,getError}=context();fail();
  await vm.runInContext('navigate(new URL("http://localhost/admin"))',sandbox);
  assert.equal(location.pathname,'/');assert.equal(log.disposed.length,0);
  assert.ok(getError().textContent.includes('当前页面已保留'));
});
test('new-tab, download and external clicks keep normal browser behavior',()=>{
  const {listeners,log}=context();
  for(const spec of [{href:'https://elsewhere.example/'},{href:'http://localhost/admin',target:'_blank'},{href:'http://localhost/admin',download:true},{href:'http://localhost/admin',ctrlKey:true}]){
    let prevented=false;
    listeners.click({button:0,ctrlKey:!!spec.ctrlKey,preventDefault(){prevented=true;},target:{closest:()=>({href:spec.href,target:spec.target,hasAttribute:name=>name==='download'&&spec.download})}});
    assert.equal(prevented,false);
  }
  assert.equal(log.fetches.length,0);
});
