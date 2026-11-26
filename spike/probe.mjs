import http from 'node:http';
import { randomBytes, randomUUID, timingSafeEqual } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HOST = '127.0.0.1';
const PORT = 8787;
const BUDGET = 350;
const EVENTS = {
  codex: new Set(['SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PermissionRequest', 'PostToolUse', 'Stop', 'Interrupt', 'SessionEnd', 'PreModelRequest', 'PostModelRequest']),
  claude: new Set(['SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PermissionRequest', 'PostToolUse', 'PostToolUseFailure', 'PermissionDenied', 'Notification', 'Stop', 'StopFailure', 'SessionEnd', 'Elicitation', 'ElicitationResult'])
};
const NOTICES = new Set(['permission_prompt', 'idle_prompt', 'elicitation_dialog', 'elicitation_url_dialog']);
const id = v => typeof v === 'string' && /^[A-Za-z0-9_.:-]{1,128}$/.test(v);
const tokenDir = join(homedir(), '.agent-monitor-spike');
const tokenPath = () => process.env.AM_SPIKE_TOKEN_FILE || join(tokenDir, 'token');
function token() {
  const value = readFileSync(tokenPath(), 'utf8').trim();
  if (!/^[a-f0-9]{64}$/.test(value)) throw Error('Invalid token');
  return value;
}
export function sanitize(source, event, hostId, raw) {
  if (!Object.hasOwn(EVENTS, source) || !EVENTS[source].has(event) || !id(hostId) || !raw || raw.hook_event_name !== event || !id(raw.session_id)) return null;
  const result = { source, event, hostId, sessionId: raw.session_id, observationId: randomUUID(), observedAt: new Date().toISOString(), confidence: 'provisional' };
  if (id(raw.turn_id)) result.turnId = raw.turn_id;
  if (event === 'Stop' && typeof raw.stop_hook_active === 'boolean') result.stopHookActive = raw.stop_hook_active;
  if (event === 'Notification' && NOTICES.has(raw.notification_type)) result.notificationType = raw.notification_type;
  return result;
}
export function valid(e) {
  if (!e || typeof e !== 'object' || Array.isArray(e)) return false;
  const keys = ['source','event','hostId','sessionId','observationId','observedAt','confidence','turnId','stopHookActive','notificationType'];
  return Object.keys(e).every(k => keys.includes(k)) && Object.hasOwn(EVENTS, e.source) && EVENTS[e.source].has(e.event) && [e.hostId,e.sessionId,e.observationId].every(id) && typeof e.observedAt === 'string' && e.observedAt.length <= 30 && Number.isFinite(Date.parse(e.observedAt)) && e.confidence === 'provisional' && (!Object.hasOwn(e,'turnId') || id(e.turnId)) && (!Object.hasOwn(e,'stopHookActive') || (e.event === 'Stop' && typeof e.stopHookActive === 'boolean')) && (!Object.hasOwn(e,'notificationType') || (e.event === 'Notification' && NOTICES.has(e.notificationType)));
}
export async function readJson(stream, limit) {
  const chunks = []; let size = 0;
  for await (const chunk of stream) {
    size += chunk.length;
    if (size > limit) throw Error('Input too large');
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}
export function collector(secret, onEvent, hang = false) {
  const server = http.createServer(async (req,res) => {
    const reply = status => { res.writeHead(status, {'Connection':'close','Cache-Control':'no-store'}); res.end(); };
    const expectedHost = HOST + ':' + server.address().port;
    if (req.headers.host !== expectedHost || req.headers.origin) return reply(403);
    if (req.method !== 'POST' || req.url !== '/observe') return reply(404);
    const expected = Buffer.from('Bearer ' + secret), actual = Buffer.from(req.headers.authorization || '');
    if (actual.length !== expected.length || !timingSafeEqual(actual,expected)) return reply(401);
    if (req.headers['content-type'] !== 'application/json') return reply(415);
    try {
      const e = await readJson(req, 4096);
      if (!valid(e)) return reply(400);
      if (hang) return;
      onEvent(e); reply(204);
    } catch { if (!res.destroyed) reply(400); }
  });
  server.setTimeout(1000, socket => socket.destroy());
  server.maxConnections = 32;
  return server;
}
async function deliver(e) {
  if (!valid(e)) return;
  await fetch('http://' + HOST + ':' + PORT + '/observe', { method:'POST', headers:{'Authorization':'Bearer ' + token(),'Content-Type':'application/json'},body:JSON.stringify(e),signal:AbortSignal.timeout(BUDGET),redirect:'error' });
}
async function relay(exe, script, event) {
  if (!exe || !script) return;
  await new Promise(resolve => {
    const child = spawn(exe,[script,'relay'],{shell:false,stdio:['pipe','ignore','ignore'],windowsHide:true});
    const timer = setTimeout(() => { child.kill(); resolve(); }, BUDGET - 50);
    const done = () => { clearTimeout(timer); resolve(); };
    child.on('error',done); child.on('close',done); child.stdin.on('error',done);
    child.stdin.end(JSON.stringify(event));
  });
}
async function main() {
  const [mode,source,event,hostId,exe,script] = process.argv.slice(2);
  if (mode === 'init') {
    if (process.env.AM_SPIKE_TOKEN_FILE) throw Error('Use default token location for init');
    mkdirSync(tokenDir,{recursive:true,mode:0o700});
    writeFileSync(tokenPath(),randomBytes(32).toString('hex')+'\n',{flag:'wx',mode:0o600});
    console.log('Token created. Verify user-only access before use.'); return;
  }
  if (mode === 'serve') {
    const server = collector(token(), e => console.log(JSON.stringify(e)), source === '--hang');
    server.listen(PORT,HOST,() => console.log('Experimental collector listening on ' + HOST + ':' + PORT)); return;
  }
  if (mode !== 'hook' && mode !== 'relay') throw Error('Use init, serve, hook or relay');
  // The only candidate structured output is an empty object for Codex Stop.
  const finish = () => { if (mode === 'hook' && source === 'codex' && event === 'Stop') process.stdout.write('{}\n',() => process.exit(0)); else process.exit(0); };
  const deadline = setTimeout(finish,BUDGET);
  try {
    const input = await readJson(process.stdin,mode === 'hook' ? 1024*1024 : 4096);
    const e = mode === 'hook' ? sanitize(source,event,hostId,input) : input;
    if (valid(e)) { if (mode === 'hook' && exe) await relay(exe,script,e); else await deliver(e); }
  } catch { /* Never emit input, errors, or decisions to a provider. */ }
  clearTimeout(deadline); finish();
}
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main().catch(() => { console.error('Probe setup failed; check configuration.'); process.exitCode = 1; });
