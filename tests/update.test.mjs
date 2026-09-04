import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,chmodSync,rmSync,readdirSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';

const linux = process.platform === 'linux';
const source = readFileSync(new URL('../deploy/install-agent.sh',import.meta.url),'utf8');
function fixture() {
  const root = mkdtempSync(join(tmpdir(),'monitor-update-test-'));
  const bin = join(root,'installed'),etc = join(root,'etc'),commands = join(root,'commands'),assets = join(root,'assets');
  for (const dir of [bin,etc,commands,assets]) mkdirSync(dir);
  const installed = join(bin,'monitor-agent'),next = join(root,'next');
  writeFileSync(installed,'original-binary'); chmodSync(installed,0o755);
  writeFileSync(next,'updated-binary');
  // Only fixed absolute paths are redirected. The installed service and token
  // are fixtures; tests never operate on the runner's real monitoring service.
  const script = join(root,'install-agent.sh');
  writeFileSync(script,source.replaceAll('/usr/local/bin/',bin+'/').replaceAll('/etc/',etc+'/'));
  mkdirSync(join(etc,'systemd/system/monitor-agent.service.d'),{recursive:true});
  const protectedFiles = [join(etc,'monitor-agent.token'),join(etc,'systemd/system/monitor-agent.service'),join(etc,'systemd/system/monitor-agent.service.d/10-server.conf')];
  for (const file of protectedFiles) writeFileSync(file,'keep-this-private-fixture');
  const executable = (name,body) => { const path=join(commands,name); writeFileSync(path,'#!/bin/sh\nset -eu\n'+body); chmodSync(path,0o755); };
  executable('id',"printf '0\\n'\n");
  executable('uname',"if [ \"${1:-}\" = '-m' ]; then printf '%s\\n' \"${MOCK_ARCH:-x86_64}\"; else printf 'Linux\\n'; fi\n");
  executable('sleep','exit 0\n');
  executable('systemctl',`case "$1" in
show) printf '%s\\n' '{ path=${installed} ; argv[]=${installed} --server https://original.example ; }' ;;
restart)
  printf 'restart\\n' >> "$MOCK_ROOT/restarts"
  if [ "\${FAIL_RESTART_ONCE:-0}" = 1 ] && [ ! -f "$MOCK_ROOT/restart-failed" ]; then
    touch "$MOCK_ROOT/restart-failed"; exit 1
  fi ;;
is-active) exit 0 ;;
*) echo "Unexpected service mutation: $1" >&2; exit 2 ;;
esac\n`);
  executable('curl',`if [ "\${FAIL_DOWNLOAD:-0}" = 1 ]; then exit 22; fi
destination=; url=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) destination=$2; shift 2 ;;
    *) url=$1; shift ;;
  esac
done
cp -- "$MOCK_ASSETS/\${url##*/}" "$destination"\n`);
  for (const arch of ['amd64','arm64']) {
    const name='monitor-agent-linux-'+arch,contents='downloaded-'+arch;
    writeFileSync(join(assets,name),contents);
    writeFileSync(join(assets,name+'.sha256'),createHash('sha256').update(contents).digest('hex')+'  '+name+'\n');
  }
  const run = (args,env={}) => spawnSync('sh',[script,...args],{encoding:'utf8',env:{...process.env,PATH:commands+':'+process.env.PATH,MOCK_ROOT:root,MOCK_ASSETS:assets,...env}});
  const unchanged = () => {
    for (const file of protectedFiles) assert.equal(readFileSync(file,'utf8'),'keep-this-private-fixture');
    assert.deepEqual(readdirSync(bin),['monitor-agent']);
  };
  return {root,bin,next,installed,assets,run,unchanged,close:()=>rmSync(root,{recursive:true,force:true})};
}
test('manual update replaces only the program and restarts without reading a token', {skip:!linux}, () => {
  const f=fixture();
  try {
    const r=f.run(['--update','--binary',f.next]);
    assert.equal(r.status,0,r.stderr);
    assert.equal(readFileSync(f.installed,'utf8'),'updated-binary');
    assert.equal(readFileSync(join(f.root,'restarts'),'utf8'),'restart\n');
    assert.ok(!(r.stdout+r.stderr).includes('keep-this-private-fixture')); f.unchanged();
  } finally {f.close();}
});
test('failed restart restores the original binary and never changes configuration', {skip:!linux}, () => {
  const f=fixture();
  try {
    const r=f.run(['--update','--binary',f.next],{FAIL_RESTART_ONCE:'1'});
    assert.notEqual(r.status,0);
    assert.equal(readFileSync(f.installed,'utf8'),'original-binary');
    assert.equal(readFileSync(join(f.root,'restarts'),'utf8'),'restart\nrestart\n'); f.unchanged();
  } finally {f.close();}
});
test('download and checksum failures preserve the existing installation', {skip:!linux}, () => {
  const f=fixture();
  try {
    assert.notEqual(f.run(['--update'],{FAIL_DOWNLOAD:'1'}).status,0);
    writeFileSync(join(f.assets,'monitor-agent-linux-amd64'),'corrupted-download');
    assert.notEqual(f.run(['--update']).status,0);
    assert.equal(readFileSync(f.installed,'utf8'),'original-binary'); f.unchanged();
  } finally {f.close();}
});
test('verified downloads select the installed architecture', {skip:!linux}, () => {
  const f=fixture();
  try {
    const r=f.run(['--update'],{MOCK_ARCH:'aarch64'});
    assert.equal(r.status,0,r.stderr);
    assert.equal(readFileSync(f.installed,'utf8'),'downloaded-arm64'); f.unchanged();
  } finally {f.close();}
});
test('update refuses credential overrides and missing installations', {skip:!linux}, () => {
  const f=fixture();
  try {
    assert.notEqual(f.run(['--update','--token','dummy']).status,0);
    assert.notEqual(f.run(['--update','--server','http://example.com']).status,0);
    f.unchanged(); rmSync(f.installed);
    assert.notEqual(f.run(['--update']).status,0);
    assert.deepEqual(readdirSync(f.bin),[]);
  } finally {f.close();}
});
