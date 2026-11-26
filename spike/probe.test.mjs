import test from 'node:test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { spawn } from 'node:child_process';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { sanitize, valid, collector } from './probe.mjs';

const raw = {hook_event_name:'Stop',session_id:'s1',turn_id:'t1',stop_hook_active:true,prompt:'secret',tool_input:{command:'secret'},cwd:'/private',transcript_path:'/private',last_assistant_message:'secret'};
test('only allowlisted metadata leaves the adapter; Stop stays provisional', () => {
  const e = sanitize('codex','Stop','native',raw);
  assert.equal(e.confidence,'provisional'); assert.equal(e.stopHookActive,true); assert.equal(e.turnId,'t1');
  assert.equal(valid(e),true); assert.doesNotMatch(JSON.stringify(e),/secret|private/);
  assert.equal(valid({...e,prompt:'secret'}),false); assert.equal(valid({...e,confidence:'confirmed'}),false);
  assert.equal(sanitize('codex','PermissionRequest','native',raw),null);
  assert.equal(sanitize('claude','SubagentStop','native',{...raw,hook_event_name:'SubagentStop'}),null);
});
test('permission request and Claude notification never assert current state', () => {
  for (const [event,extra] of [['PermissionRequest',{}],['Notification',{notification_type:'permission_prompt',message:'secret'}]]) {
    const e=sanitize('claude',event,'wsl-ubuntu',{...raw,...extra,hook_event_name:event});
    assert.equal(e.confidence,'provisional'); assert.equal(valid(e),true); assert.doesNotMatch(JSON.stringify(e),/secret/);
  }
});
test('collector accepts only authenticated local allowlisted events', async t => {
  const seen=[], server=collector('a'.repeat(64),e=>seen.push(e));
  server.listen(0,'127.0.0.1'); await once(server,'listening'); t.after(()=>server.close());
  const url='http://127.0.0.1:'+server.address().port+'/observe';
  const e=sanitize('codex','Stop','native',raw);
  const headers={'Authorization':'Bearer '+'a'.repeat(64),'Content-Type':'application/json'};
  const send=(body,extra={})=>fetch(url,{method:'POST',headers:{...headers,...extra},body:JSON.stringify(body)});
  assert.equal((await send(e)).status,204);
  assert.equal((await send(e,{Authorization:'Bearer wrong'})).status,401);
  assert.equal((await send(e,{Origin:'https://example.com'})).status,403);
  assert.equal((await send({...e,extra:'secret'})).status,400);
  assert.equal((await send(e,{ 'Content-Type':'text/plain'})).status,415);
  assert.equal(seen.length,1);
});
const script=fileURLToPath(new URL('./probe.mjs',import.meta.url));
async function hook(args,input,env) {
  const child=spawn(process.execPath,[script,'hook',...args],{env:{...process.env,...env},stdio:['pipe','pipe','pipe']});
  const out=[],err=[]; child.stdout.on('data',c=>out.push(c)); child.stderr.on('data',c=>err.push(c));
  child.stdin.on('error',()=>{}); if(input!==null) child.stdin.end(input);
  const start=performance.now(); const [code]=await once(child,'close');
  return {code,out:Buffer.concat(out).toString(),err:Buffer.concat(err).toString(),ms:performance.now()-start};
}
test('hook failure and incomplete input keep candidate neutral output', async t => {
  const dir=await mkdtemp(join(tmpdir(),'am-spike-test-')); t.after(()=>rm(dir,{recursive:true,force:true}));
  const env={AM_SPIKE_TOKEN_FILE:join(dir,'missing')};
  for(const [provider,event,input] of [['codex','Stop','{bad'],['claude','Stop',JSON.stringify(raw)],['codex','Stop',null]]) {
    const r=await hook([provider,event,'native'],input,env);
    assert.equal(r.code,0); assert.equal(r.err,''); assert.equal(r.out,provider==='codex'?'{}\n':''); assert.ok(r.ms<1500,'watchdog regression');
  }
  await writeFile(env.AM_SPIKE_TOKEN_FILE,'a'.repeat(64));
  const r=await hook(['codex','Stop','native',join(dir,'missing-exe'),'unused'],JSON.stringify(raw),env);
  assert.equal(r.code,0); assert.equal(r.err,''); assert.equal(r.out,'{}\n');
});
