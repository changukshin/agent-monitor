//! Disposable Phase 1 observation probe. Not a production collector.
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env, fs::{self, OpenOptions}, io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream}, path::PathBuf,
    process::{self, Command, Stdio}, sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}},
    thread, time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const PORT: u16 = 8787;
const BUDGET: Duration = Duration::from_millis(350);
const RELAY_BUDGET: Duration = Duration::from_millis(300);
const MAX_EVENT: usize = 4096;
const MAX_RAW: usize = 1024 * 1024;
const MAX_HEADERS: usize = 8192;
const MAX_CONNECTIONS: usize = 8;
const CONNECTION_BUDGET: Duration = Duration::from_millis(800);
static FINISHED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Ack, Input, Invalid, Token, Connect, Io, Http, RelayStart, RelayFailed, Timeout,
}
impl Outcome {
    fn name(self) -> &'static str {
        match self {
            Self::Ack => "ack", Self::Input => "input_error", Self::Invalid => "invalid_event",
            Self::Token => "token_error", Self::Connect => "connect_error", Self::Io => "io_error",
            Self::Http => "http_rejected", Self::RelayStart => "relay_start_error",
            Self::RelayFailed => "relay_failed", Self::Timeout => "timeout",
        }
    }
    fn code(self) -> i32 {
        match self {
            Self::Ack => 0, Self::Input => 10, Self::Invalid => 11, Self::Token => 12,
            Self::Connect => 13, Self::Io => 14, Self::Http => 15, Self::Timeout => 16,
            Self::RelayStart | Self::RelayFailed => 17,
        }
    }
    fn from_code(code: Option<i32>) -> Self {
        match code {
            Some(0) => Self::Ack, Some(10) => Self::Input, Some(11) => Self::Invalid,
            Some(12) => Self::Token, Some(13) => Self::Connect, Some(14) => Self::Io,
            Some(15) => Self::Http, Some(16) => Self::Timeout, _ => Self::RelayFailed,
        }
    }
}

fn event_allowed(source: &str, event: &str) -> bool {
    match source {
        "codex" => matches!(event, "SessionStart" | "UserPromptSubmit" | "PreToolUse" |
            "PermissionRequest" | "PostToolUse" | "Stop" | "Interrupt" | "SessionEnd" |
            "PreModelRequest" | "PostModelRequest"),
        "claude" => matches!(event, "SessionStart" | "UserPromptSubmit" | "PreToolUse" |
            "PermissionRequest" | "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" |
            "Notification" | "Stop" | "StopFailure" | "SessionEnd" | "Elicitation" | "ElicitationResult"),
        _ => false,
    }
}
fn id(v: &str) -> bool {
    !v.is_empty() && v.len() <= 128 &&
        v.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn notice(v: &str) -> bool {
    matches!(v, "permission_prompt" | "idle_prompt" | "elicitation_dialog" | "elicitation_url_dialog")
}
fn random_hex(n: usize) -> io::Result<String> {
    let mut bytes = vec![0u8; n];
    OsRng.try_fill_bytes(&mut bytes).map_err(|_| io::Error::other("random source unavailable"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Observation {
    source: String,
    event: String,
    host_id: String,
    session_id: String,
    observation_id: String,
    observed_at_ms: u64,
    confidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stop_hook_active: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notification_type: Option<String>,
}
impl Observation {
    fn valid(&self) -> bool {
        event_allowed(&self.source, &self.event) &&
            [&self.host_id, &self.session_id, &self.observation_id].iter().all(|s| id(s)) &&
            self.confidence == "provisional" &&
            self.turn_id.as_deref().map(id).unwrap_or(true) &&
            (self.stop_hook_active.is_none() || self.event == "Stop") &&
            self.notification_type.as_deref().map(|v| self.event == "Notification" && notice(v)).unwrap_or(true)
    }
}
fn sanitize(source: &str, event: &str, host: &str, raw: &Value) -> Result<Observation, Outcome> {
    let session = raw.get("session_id").and_then(Value::as_str).filter(|v| id(v)).ok_or(Outcome::Invalid)?;
    if !event_allowed(source, event) || !id(host) ||
        raw.get("hook_event_name").and_then(Value::as_str) != Some(event) { return Err(Outcome::Invalid); }
    Ok(Observation {
        source: source.into(), event: event.into(), host_id: host.into(), session_id: session.into(),
        observation_id: random_hex(16).map_err(|_| Outcome::Io)?, observed_at_ms: now_ms(), confidence: "provisional".into(),
        turn_id: raw.get("turn_id").and_then(Value::as_str).filter(|v| id(v)).map(str::to_owned),
        stop_hook_active: if event == "Stop" { raw.get("stop_hook_active").and_then(Value::as_bool) } else { None },
        notification_type: if event == "Notification" { raw.get("notification_type").and_then(Value::as_str)
            .filter(|v| notice(v)).map(str::to_owned) } else { None },
    })
}
fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, Outcome> {
    let mut bytes = Vec::new();
    reader.take((limit + 1) as u64).read_to_end(&mut bytes).map_err(|_| Outcome::Input)?;
    if bytes.len() > limit { return Err(Outcome::Input); }
    Ok(bytes)
}
fn token_path() -> io::Result<PathBuf> {
    if let Some(p) = env::var_os("AM_SPIKE_TOKEN_FILE") { return Ok(p.into()); }
    let home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .ok_or_else(|| io::Error::other("no home directory"))?;
    Ok(PathBuf::from(home).join(".agent-monitor-spike").join("token"))
}
fn token() -> Result<String, Outcome> {
    let path = token_path().map_err(|_| Outcome::Token)?;
    let file = fs::File::open(path).map_err(|_| Outcome::Token)?;
    let bytes = read_bounded(file, 128).map_err(|_| Outcome::Token)?;
    let value = std::str::from_utf8(&bytes).map_err(|_| Outcome::Token)?.trim();
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(Outcome::Token);
    }
    Ok(value.into())
}
fn init() -> io::Result<()> {
    if env::var_os("AM_SPIKE_TOKEN_FILE").is_some() { return Err(io::Error::other("init uses default path")); }
    let path = token_path()?;
    let parent = path.parent().ok_or_else(|| io::Error::other("invalid token directory"))?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; builder.mode(0o700); }
    builder.create(parent)?;
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; opts.mode(0o600); }
    let value = random_hex(32)?;
    let mut file = opts.open(path)?;
    writeln!(file, "{value}")?;
    println!("Token created. Verify directory and file ACLs before use.");
    Ok(())
}
// Opt-in diagnostics: an already-created private file, static stage labels and timing only.
// Never create a diagnostic file, log a path, payload, token, or provider stderr.
fn diag(start: Instant, stage: &str) {
    if let Some(path) = env::var_os("AM_SPIKE_DIAG_FILE") {
        if let Ok(mut file) = OpenOptions::new().append(true).open(path) {
            let _ = writeln!(file, "{} {} {}", now_ms(), start.elapsed().as_millis(), stage);
        }
    }
}
fn remaining(deadline: Instant) -> Result<Duration, Outcome> {
    deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or(Outcome::Timeout)
}
fn io_outcome(e: io::Error) -> Outcome {
    if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) { Outcome::Timeout } else { Outcome::Io }
}
fn read_headers(s: &mut TcpStream, deadline: Instant) -> Result<String, Outcome> {
    let mut bytes = Vec::new();
    let mut b = [0u8; 1];
    while bytes.len() < MAX_HEADERS {
        s.set_read_timeout(Some(remaining(deadline)?)).map_err(io_outcome)?;
        s.read_exact(&mut b).map_err(io_outcome)?;
        bytes.push(b[0]);
        if bytes.ends_with(b"\r\n\r\n") { return String::from_utf8(bytes).map_err(|_| Outcome::Invalid); }
    }
    Err(Outcome::Invalid)
}
fn read_body(s: &mut TcpStream, length: usize, deadline: Instant) -> Result<Vec<u8>, Outcome> {
    if length > MAX_EVENT { return Err(Outcome::Invalid); }
    let mut body = vec![0; length];
    let mut off = 0;
    while off < length {
        s.set_read_timeout(Some(remaining(deadline)?)).map_err(io_outcome)?;
        let n = s.read(&mut body[off..]).map_err(io_outcome)?;
        if n == 0 { return Err(Outcome::Io); }
        off += n;
    }
    Ok(body)
}
fn equal_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |x, (a,b)| x | (a ^ b)) == 0
}
// Narrow, one-request HTTP subset for the disposable loopback experiment.
// No chunking, redirects, keep-alive, browser access or public listener.
// Parse framing separately from policy. A rejected but well-framed request still
// has a bounded body to consume before closing the connection.
fn request_head(headers: &str, port: u16, secret: &str) -> Result<(usize, u16), u16> {
    let mut lines = headers.strip_suffix("\r\n\r\n").ok_or(400u16)?.split("\r\n");
    let request = lines.next().ok_or(400u16)?;
    let mut host = None; let mut auth = None; let mut len = None; let mut kind = None;
    let mut origin = false;
    for line in lines {
        let (key, val) = line.split_once(':').ok_or(400u16)?;
        // Reject ambiguous header names instead of treating them as unknown.
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_".contains(&b) || b == 0x60 || b == b'|' || b == b'~') {
            return Err(400);
        }
        let value = val.trim();
        let field = match key.to_ascii_lowercase().as_str() {
            "host" => &mut host, "authorization" => &mut auth, "content-length" => &mut len,
            "content-type" => &mut kind, "origin" => { origin = true; continue; },
            // Unsupported/ambiguous framing is never drained.
            "transfer-encoding" => return Err(403),
            _ => continue,
        };
        if field.replace(value).is_some() { return Err(400); }
    }
    let n = len.filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| v.parse::<usize>().ok()).filter(|n| *n <= MAX_EVENT).ok_or(400u16)?;
    let expected = format!("127.0.0.1:{port}");
    let status = if request != "POST /observe HTTP/1.1" { 404 }
        else if origin || host != Some(expected.as_str()) { 403 }
        else if !auth.and_then(|a| a.strip_prefix("Bearer ")).map(|a| equal_token(a, secret)).unwrap_or(false) { 401 }
        else if kind != Some("application/json") { 415 }
        else { 204 };
    Ok((n, status))
}
fn respond(s: &mut TcpStream, status: u16, deadline: Instant) {
    if let Ok(left) = remaining(deadline) {
        let _ = s.set_write_timeout(Some(left));
        let reason = match status { 204 => "No Content", 401 => "Unauthorized", 403 => "Forbidden",
            404 => "Not Found", 415 => "Unsupported Media Type", _ => "Bad Request" };
        let _ = write!(s, "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n");
    }
}
fn serve_one(mut s: TcpStream, port: u16, secret: &str, hang: bool, deadline: Instant, observe: impl FnOnce(Observation)) {
    let result = (|| {
        let headers = read_headers(&mut s, deadline).map_err(|_| 400u16)?;
        let (length, status) = request_head(&headers, port, secret)?;
        let body = read_body(&mut s, length, deadline).map_err(|_| 400u16)?;
        // Never deserialize or emit the body of a policy-rejected request.
        if status != 204 { return Err(status); }
        let e: Observation = serde_json::from_slice(&body).map_err(|_| 400u16)?;
        if !e.valid() || remaining(deadline).is_err() { return Err(400); }
        Ok(e)
    })();
    match result {
        Ok(e) if !hang => { observe(e); respond(&mut s, 204, deadline); },
        Ok(_) => thread::sleep(remaining(deadline).unwrap_or_default()),
        Err(status) => respond(&mut s, status, deadline),
    }
}
// A permit is released even when thread creation fails or a handler unwinds.
struct ConnectionPermit(Arc<AtomicUsize>);
impl ConnectionPermit {
    fn acquire(active: &Arc<AtomicUsize>, limit: usize) -> Option<Self> {
        active.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            if n < limit { Some(n + 1) } else { None }
        }).ok()?;
        Some(Self(Arc::clone(active)))
    }
}
impl Drop for ConnectionPermit {
    fn drop(&mut self) { self.0.fetch_sub(1, Ordering::AcqRel); }
}
fn dispatch_connection(s: TcpStream, port: u16, secret: Arc<str>, hang: bool,
    active: &Arc<AtomicUsize>, observe: impl FnOnce(Observation) + Send + 'static)
    -> Option<thread::JoinHandle<()>> {
    let deadline = Instant::now() + CONNECTION_BUDGET;
    // No application queue: an excess connection is dropped immediately.
    let permit = ConnectionPermit::acquire(active, MAX_CONNECTIONS)?;
    thread::Builder::new().spawn(move || {
        let _permit = permit;
        serve_one(s, port, &secret, hang, deadline, observe);
    }).ok()
}
fn send(e: &Observation, addr: SocketAddr, secret: &str, deadline: Instant) -> Outcome {
    let result = (|| {
        if !e.valid() { return Err(Outcome::Invalid); }
        let body = serde_json::to_vec(e).map_err(|_| Outcome::Invalid)?;
        if body.len() > MAX_EVENT { return Err(Outcome::Invalid); }
        let mut s = TcpStream::connect_timeout(&addr, remaining(deadline)?).map_err(|e| {
            if io_outcome(e) == Outcome::Timeout { Outcome::Timeout } else { Outcome::Connect }
        })?;
        s.set_write_timeout(Some(remaining(deadline)?)).map_err(io_outcome)?;
        let headers = format!("POST /observe HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {secret}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        s.write_all(headers.as_bytes()).map_err(io_outcome)?;
        s.set_write_timeout(Some(remaining(deadline)?)).map_err(io_outcome)?;
        s.write_all(&body).map_err(io_outcome)?;
        // One request per connection: signal that there is no further request data.
        s.shutdown(Shutdown::Write).map_err(io_outcome)?;
        let response = read_headers(&mut s, deadline)?;
        if response.lines().next().and_then(|l| l.split_whitespace().nth(1)) != Some("204") { return Err(Outcome::Http); }
        Ok(())
    })();
    result.map(|_| Outcome::Ack).unwrap_or_else(|e| e)
}
fn local_addr(port: u16) -> SocketAddr { SocketAddr::from(([127,0,0,1], port)) }
fn deliver(e: &Observation, deadline: Instant, start: Instant) -> Outcome {
    let secret = match token() { Ok(v) => v, Err(e) => return e };
    diag(start, "token_loaded");
    send(e, local_addr(PORT), &secret, deadline)
}
fn relay(exe: &str, e: &Observation, start: Instant, deadline: Instant) -> Outcome {
    let body = match serde_json::to_vec(e) { Ok(b) if b.len() <= MAX_EVENT => b, _ => return Outcome::Invalid };
    let cutoff = deadline.min(start + RELAY_BUDGET);
    if remaining(cutoff).is_err() { return Outcome::Timeout; }
    let mut child = match Command::new(exe).arg("relay").stdin(Stdio::piped())
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(c) => c, Err(_) => return Outcome::RelayStart,
    };
    diag(start, "relay_spawned");
    if child.stdin.take().map(|mut s| s.write_all(&body).is_ok()) != Some(true) {
        let _ = child.kill(); let _ = child.wait(); return Outcome::RelayFailed;
    }
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return Outcome::from_code(s.code()),
            Ok(None) => {},
            Err(_) => { let _ = child.kill(); let _ = child.wait(); return Outcome::RelayFailed; },
        }
        if remaining(cutoff).is_err() {
            let _ = child.kill(); let _ = child.wait(); return Outcome::Timeout;
        }
        thread::sleep(Duration::from_millis(5));
    }
}
fn finish(stop_json: bool, code: i32) -> ! {
    if FINISHED.swap(true, Ordering::SeqCst) { loop { thread::park(); } }
    if stop_json { let _ = io::stdout().write_all(b"{}\n"); let _ = io::stdout().flush(); }
    process::exit(code)
}
fn hook_or_relay(args: &[String], is_relay: bool) -> ! {
    let start = Instant::now();
    let deadline = start + BUDGET;
    let stop_json = !is_relay && args.get(1).map(String::as_str) == Some("codex") && args.get(2).map(String::as_str) == Some("Stop");
    thread::spawn(move || {
        thread::sleep(deadline.saturating_duration_since(Instant::now()));
        // Do not let optional diagnostics block the hard watchdog.
        finish(stop_json, if is_relay { Outcome::Timeout.code() } else { 0 });
    });
    diag(start, if is_relay { "relay_started" } else { "hook_started" });
    let outcome = (|| {
        let bytes = read_bounded(io::stdin().lock(), if is_relay { MAX_EVENT } else { MAX_RAW })?;
        diag(start, "stdin_read");
        let e = if is_relay {
            serde_json::from_slice::<Observation>(&bytes).map_err(|_| Outcome::Invalid)?
        } else {
            let raw: Value = serde_json::from_slice(&bytes).map_err(|_| Outcome::Input)?;
            let source = args.get(1).ok_or(Outcome::Invalid)?;
            let event = args.get(2).ok_or(Outcome::Invalid)?;
            let host = args.get(3).ok_or(Outcome::Invalid)?;
            sanitize(source, event, host, &raw)?
        };
        if !e.valid() { return Err(Outcome::Invalid); }
        diag(start, "event_validated");
        Ok(if !is_relay {
            if let Some(exe) = args.get(4) { relay(exe, &e, start, deadline) }
            else { deliver(&e, deadline, start) }
        } else { deliver(&e, deadline, start) })
    })().unwrap_or_else(|e| e);
    diag(start, outcome.name());
    finish(stop_json, if is_relay { outcome.code() } else { 0 })
}
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("hook") => hook_or_relay(&args, false),
        Some("relay") => hook_or_relay(&args, true),
        Some("init") => { if init().is_err() { eprintln!("Token setup failed."); process::exit(1); } },
        Some("serve") => {
            let secret = match token() { Ok(v) => v, Err(_) => { eprintln!("Token unavailable."); process::exit(1); } };
            let listener = match TcpListener::bind(local_addr(PORT)) {
                Ok(l) => l, Err(_) => { eprintln!("Loopback bind failed."); process::exit(1); }
            };
            let hang = args.get(1).map(String::as_str) == Some("--hang");
            println!("Experimental collector listening on 127.0.0.1:{PORT}");
            let secret: Arc<str> = Arc::from(secret);
            let active = Arc::new(AtomicUsize::new(0));
            for conn in listener.incoming() {
                if let Ok(s) = conn {
                    let _ = dispatch_connection(s, PORT, Arc::clone(&secret), hang, &active, |e| {
                        if let Ok(line) = serde_json::to_string(&e) { println!("{line}"); }
                    });
                }
            }
        },
        _ => { eprintln!("Usage: agent-monitor-spike init | serve [--hang] | hook SOURCE EVENT HOST [WINDOWS_EXE] | relay"); process::exit(2); },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn sample() -> Observation {
        sanitize("codex", "Stop", "native", &json!({
            "hook_event_name":"Stop", "session_id":"s1", "turn_id":"t1",
            "stop_hook_active":true, "prompt":"secret", "cwd":"/private",
            "tool_input":{"command":"secret"}
        })).unwrap()
    }
    #[test]
    fn allowlist_and_provisional() {
        let e = sample(); assert!(e.valid()); assert_eq!(e.confidence, "provisional");
        assert_eq!(e.stop_hook_active, Some(true));
        let text = serde_json::to_string(&e).unwrap();
        assert!(!text.contains("secret") && !text.contains("private"));
        let mut v = serde_json::to_value(e).unwrap(); v["prompt"] = json!("secret");
        assert!(serde_json::from_value::<Observation>(v).is_err());
        assert!(sanitize("claude","Stop","native",&json!({"session_id":"s","hook_event_name":"SessionStart"})).is_err());
    }
    #[test]
    fn no_confirmed_states() {
        let mut e = sample(); e.confidence = "confirmed".into(); assert!(!e.valid());
        let n = sanitize("claude","Notification","wsl",&json!({
            "hook_event_name":"Notification","session_id":"s","notification_type":"permission_prompt"
        })).unwrap();
        assert_eq!(n.confidence,"provisional");
        assert!(n.valid());
    }
    #[test]
    fn input_limit() {
        assert_eq!(read_bounded(&b"abcd"[..], 3), Err(Outcome::Input));
        assert_eq!(read_bounded(&b"abc"[..], 3).unwrap(), b"abc");
    }
    #[test]
    fn request_checks() {
        let secret = "a".repeat(64);
        let req = format!("POST /observe HTTP/1.1\r\nHost: 127.0.0.1:8787\r\nAuthorization: Bearer {secret}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n");
        assert_eq!(request_head(&req,8787,&secret),Ok((100,204)));
        assert_eq!(request_head(&req,8787,"wrong"),Ok((100,401)));
        assert_eq!(request_head(&req.replace("127.0.0.1","example.com"),8787,&secret),Ok((100,403)));
        assert_eq!(request_head(&req.replace("Content-Length:","Origin: https://example.com\r\nContent-Length:"),8787,&secret),Ok((100,403)));
        assert_eq!(request_head(&req.replace("application/json","text/plain"),8787,&secret),Ok((100,415)));
        assert_eq!(request_head(&req.replace("100","5000"),8787,&secret),Err(400));
        assert_eq!(request_head(&req.replace("Content-Length:","Content-Length: 1\r\nContent-Length:"),8787,&secret),Err(400));
        assert_eq!(request_head(&req.replace("Content-Length:","Transfer-Encoding: chunked\r\nContent-Length:"),8787,&secret),Err(403));
    }
    #[test]
    fn direct_delivery_and_rejection() {
        for (client, expected) in [("secret",Outcome::Ack),("wrong",Outcome::Http)] {
            let l = TcpListener::bind(local_addr(0)).unwrap();
            let addr = l.local_addr().unwrap();
            let server = thread::spawn(move || {
                let (s,_) = l.accept().unwrap();
                serve_one(s,addr.port(),"secret",false,Instant::now()+CONNECTION_BUDGET,|e| {
                    assert_eq!(expected,Outcome::Ack,"rejected requests must not emit observations");
                    assert!(e.valid());
                });
            });
            assert_eq!(send(&sample(),addr,client,Instant::now()+Duration::from_secs(2)),expected);
            server.join().unwrap();
        }
    }
    #[test]
    fn hanging_receiver_times_out() {
        let l = TcpListener::bind(local_addr(0)).unwrap();
        let addr = l.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (s,_) = l.accept().unwrap();
            serve_one(s,addr.port(),"secret",true,Instant::now()+CONNECTION_BUDGET, |_| panic!("hang must not acknowledge"));
        });
        assert_eq!(send(&sample(),addr,"secret",Instant::now()+Duration::from_millis(150)),Outcome::Timeout);
        server.join().unwrap();
    }
    #[test]
    fn rejected_bounded_body_returns_exact_status_without_observation() {
        for (token, origin, content_type, expected) in [
            ("wrong", "", "application/json", "401"),
            ("secret", "Origin: https://example.com\r\n", "application/json", "403"),
            ("secret", "", "text/plain", "415"),
        ] {
            let l = TcpListener::bind(local_addr(0)).unwrap();
            let addr = l.local_addr().unwrap();
            let server = thread::spawn(move || {
                let (s, _) = l.accept().unwrap();
                serve_one(s, addr.port(), "secret", false, Instant::now()+CONNECTION_BUDGET,
                    |_| panic!("rejected requests must not emit observations"));
            });
            let mut client = TcpStream::connect(addr).unwrap();
            client.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
            let body = b"not an observation";
            write!(client, "POST /observe HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\n{origin}Content-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",body.len()).unwrap();
            // Separate header and body writes to exercise the original close-before-body path.
            client.write_all(body).unwrap();
            client.shutdown(Shutdown::Write).unwrap();
            let response = read_headers(&mut client,Instant::now()+Duration::from_secs(2)).unwrap();
            assert_eq!(response.lines().next().and_then(|v| v.split_whitespace().nth(1)),Some(expected));
            server.join().unwrap();
        }
    }
    #[test]
    fn slow_connection_does_not_block_independent_hook() {
        let l = TcpListener::bind(local_addr(0)).unwrap();
        let addr = l.local_addr().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let server_active = Arc::clone(&active);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (observed_tx, observed_rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (first, _) = l.accept().unwrap();
            let slow = dispatch_connection(first,addr.port(),Arc::from("secret"),false,&server_active,
                |_| panic!("incomplete request emitted an observation")).unwrap();
            ready_tx.send(()).unwrap();
            let (second, _) = l.accept().unwrap();
            let fast = dispatch_connection(second,addr.port(),Arc::from("secret"),false,&server_active,
                move |e| { observed_tx.send(e.observation_id).unwrap(); }).unwrap();
            fast.join().unwrap();
            // The first connection is intentionally held open until the fast request finishes.
            assert!(!slow.is_finished(), "slow request should still be pending");
            slow.join().unwrap();
        });
        let mut incomplete = TcpStream::connect(addr).unwrap();
        incomplete.write_all(b"POST /observe HTTP/1.1\r\nHost: ").unwrap();
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let e = sample();
        let result = send(&e,addr,"secret",Instant::now()+BUDGET);
        assert_eq!(result,Outcome::Ack);
        assert_eq!(observed_rx.recv_timeout(Duration::from_secs(1)).unwrap(),e.observation_id);
        // Keep incomplete alive until the server checks that the fast request did not wait.
        server.join().unwrap();
        drop(incomplete);
        assert_eq!(active.load(Ordering::Acquire),0);
    }
    #[test]
    fn connection_limit_rejects_excess_and_releases_slots() {
        let active = Arc::new(AtomicUsize::new(0));
        let mut permits: Vec<_> = (0..MAX_CONNECTIONS)
            .map(|_| ConnectionPermit::acquire(&active,MAX_CONNECTIONS).unwrap()).collect();
        assert_eq!(active.load(Ordering::Acquire),MAX_CONNECTIONS);
        assert!(ConnectionPermit::acquire(&active,MAX_CONNECTIONS).is_none());
        drop(permits.pop());
        let next = ConnectionPermit::acquire(&active,MAX_CONNECTIONS).unwrap();
        assert_eq!(active.load(Ordering::Acquire),MAX_CONNECTIONS);
        drop(next); drop(permits);
        assert_eq!(active.load(Ordering::Acquire),0);
    }

}
