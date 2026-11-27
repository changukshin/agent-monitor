use crate::{model::*, ConfigSource, Result};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::{Read, Write},
    path::Path,
    process::{ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{client::client_with_config, protocol::WebSocketConfig, Message, WebSocket};
const MAX: usize = 1024 * 1024;
pub type Poll = (Option<String>, Result<Sample>);
pub fn uuid(v: &Value) -> Result<&str> {
    let s = v.as_str().ok_or("invalid_id")?;
    if s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        Ok(s)
    } else {
        Err("invalid_id")
    }
}
fn semver(s: &str) -> Option<String> {
    let parts: Vec<_> = s.split('.').collect();
    (parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit())))
    .then(|| s.to_owned())
}

fn child<T: Send + 'static>(
    mut command: Command,
    stop: &AtomicBool,
    deadline: Instant,
    io: impl FnOnce(ChildStdin, ChildStdout) -> Result<T> + Send + 'static,
    listing: bool,
) -> Result<T> {
    let mut proc = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "unavailable")?;
    #[cfg(windows)]
    let _guard = match crate::windows::job(&proc) {
        Ok(job) => job,
        Err(e) => {
            let _ = proc.kill();
            let _ = proc.wait();
            return Err(e);
        }
    };
    let input = proc.stdin.take().unwrap();
    let output = proc.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let _ = tx.send(io(input, output));
    });
    let result = loop {
        if stop.load(Ordering::Relaxed) {
            break Err("cancelled");
        }
        if Instant::now() >= deadline {
            break Err("timeout");
        }
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Ok(value)) if listing => {
                break loop {
                    match proc.try_wait() {
                        Ok(Some(status)) if status.success() => break Ok(value),
                        Ok(None) if Instant::now() < deadline && !stop.load(Ordering::Relaxed) => {
                            thread::sleep(Duration::from_millis(5))
                        }
                        _ => break Err("listing_failed"),
                    }
                }
            }
            Ok(value) => break value,
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err("worker_error"),
            _ => (),
        }
    };
    let _ = proc.kill();
    let _ = proc.wait();
    #[cfg(windows)]
    drop(_guard);
    let _ = worker.join();
    result
}
fn output(
    bin: &Path,
    args: &[&str],
    stop: &AtomicBool,
    deadline: Instant,
    limit: usize,
) -> Result<Vec<u8>> {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    child(
        cmd,
        stop,
        deadline,
        move |input, out| {
            drop(input);
            let mut bytes = vec![];
            out.take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "read_error")?;
            if bytes.len() > limit {
                return Err("output_limit");
            }
            Ok(bytes)
        },
        true,
    )
}
struct Pipe(ChildStdout, ChildStdin);
impl Read for Pipe {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(b)
    }
}
impl Write for Pipe {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.1.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.1.flush()
    }
}
struct Client {
    ws: WebSocket<Pipe>,
    id: u32,
    left: usize,
}
// Only a well-formed rejection of this request can be isolated to its row.
fn reply(v: Value, id: u32) -> Result<Value> {
    if v["id"] != id {
        return Err("unexpected_response");
    }
    match (v.get("result"), v.get("error")) {
        (Some(result), None) => Ok(result.clone()),
        (None, Some(e)) if e["message"].is_string() => match e["code"].as_i64() {
            Some(-32600 | -32602) => Err("request_rejected"),
            Some(_) => Err("rpc_error"),
            None => Err("invalid_response"),
        },
        _ => Err("invalid_response"),
    }
}
impl Client {
    fn query(&mut self, method: &'static str, params: Value) -> Result<Value> {
        self.id += 1;
        self.ws
            .send(Message::text(
                json!({"id":self.id,"method":method,"params":params}).to_string(),
            ))
            .map_err(|_| "unavailable")?;
        loop {
            let msg = self.ws.read().map_err(|_| "unavailable")?;
            self.left = self
                .left
                .checked_sub(msg.len().max(1))
                .ok_or("output_limit")?;
            match msg {
                Message::Text(s) => {
                    let v: Value = serde_json::from_str(&s).map_err(|_| "invalid_json")?;
                    if v.get("method").is_none() {
                        return reply(v, self.id);
                    }
                }
                Message::Close(_) => return Err("unavailable"),
                _ => (),
            }
        }
    }
    fn thread(&mut self, id: &str) -> Result<Value> {
        let mut r = self.query("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        let t = r.get_mut("thread").ok_or("invalid_thread")?.take();
        if t["id"] != id {
            return Err("identity_mismatch");
        }
        Ok(t)
    }
    fn turn(&mut self, id: &str) -> Result<Turn> {
        let r = match self.query(
            "thread/turns/list",
            json!({"threadId":id,"limit":1,"sortDirection":"desc","itemsView":"notLoaded"}),
        ) {
            Err("request_rejected") => return Ok(Turn::Unavailable),
            r => r?,
        };
        match r["data"].as_array().ok_or("invalid_list")?.as_slice() {
            [] => Ok(Turn::Empty),
            [v] => Ok(Turn::Known {
                id: uuid(&v["id"])?.into(),
                status: match v["status"].as_str() {
                    Some("inProgress") => TurnStatus::Active,
                    Some("completed") => TurnStatus::Completed,
                    Some("interrupted") => TurnStatus::Interrupted,
                    _ => TurnStatus::Unsupported,
                },
            }),
            _ => Err("invalid_list"),
        }
    }
}
fn scoped_thread(result: Result<Value>, sample: &mut Sample) -> Result<Option<Value>> {
    match result {
        Err("request_rejected") => {
            sample.partial = true;
            Ok(None)
        }
        other => other.map(Some),
    }
}
fn availability(cfg: &ConfigSource) -> Result<()> {
    if cfg.binary.is_file() && cfg.cwd.is_dir() {
        Ok(())
    } else {
        Err("unavailable")
    }
}
fn activity(t: &Value) -> Activity {
    let s = &t["status"];
    match s["type"].as_str() {
        Some("idle") => Activity::Idle,
        Some("active") => match s["activeFlags"].as_array().map(Vec::as_slice) {
            Some([]) => Activity::Active,
            Some([flag]) if flag == "waitingOnUserInput" => Activity::Input,
            Some(_) => Activity::Unsupported,
            None => Activity::Unknown,
        },
        _ => Activity::Unknown,
    }
}
fn codex(input: ChildStdin, out: ChildStdout, cfg: &ConfigSource, owner: String) -> Result<Poll> {
    let settings = WebSocketConfig::default()
        .max_message_size(Some(MAX))
        .max_frame_size(Some(MAX));
    let (ws, _) = client_with_config("ws://localhost/", Pipe(out, input), Some(settings))
        .map_err(|_| "unavailable")?;
    let mut c = Client {
        ws,
        id: 0,
        left: MAX,
    };
    let init=c.query("initialize",json!({"clientInfo":{"name":"agent-monitor","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}))?;
    let version = init["userAgent"]
        .as_str()
        .and_then(|s| s.split_once(" (Windows "))
        .and_then(|(s, _)| s.rsplit_once('/'))
        .and_then(|(_, s)| semver(s));
    if version.as_deref() != Some(cfg.mode.version()) {
        return Ok((version, Err("unsupported_version")));
    }
    c.ws.send(Message::text(r#"{"method":"initialized"}"#))
        .map_err(|_| "unavailable")?;
    let managed = cfg.mode == Mode::CodexManaged;
    let page = if managed {
        c.query("thread/loaded/list", json!({"limit":8}))?
    } else {
        c.query(
            "thread/list",
            json!({"cwd":cfg.cwd,"limit":32,"useStateDbOnly":true}),
        )?
    };
    let mut sample = Sample {
        owner,
        partial: page.get("nextCursor") != Some(&Value::Null),
        unidentified: 0,
        rows: vec![],
    };
    let entries = page["data"].as_array().ok_or("invalid_list")?;
    if entries.len() > if managed { 8 } else { 32 } {
        return Err("output_limit");
    }
    let mut seen = HashSet::new();
    for v in entries {
        if !managed && v["cwd"].as_str().is_none() {
            sample.partial = true;
            continue;
        }
        let id = uuid(if managed { v } else { &v["id"] })?;
        if !seen.insert(id) {
            return Err("duplicate_id");
        }
        if !managed {
            if v["cwd"].as_str().map(Path::new) == Some(cfg.cwd.as_path()) {
                sample.rows.push(Row {
                    id: id.into(),
                    evidence: Evidence::Discovery,
                });
            }
            continue;
        }
        let Some(t) = scoped_thread(c.thread(id), &mut sample)? else {
            continue;
        };
        if t["ephemeral"].as_bool().is_none() || t["cwd"].as_str().is_none() {
            sample.partial = true;
            continue;
        }
        if t["ephemeral"] != false || t["cwd"].as_str().map(Path::new) != Some(cfg.cwd.as_path()) {
            continue;
        }
        // Bracket both metadata reads. A single changed value invalidates the pair.
        let a = Reading {
            activity: activity(&t),
            turn: c.turn(id)?,
        };
        let Some(recheck) = scoped_thread(c.thread(id), &mut sample)? else {
            continue;
        };
        if recheck["ephemeral"] != false || recheck["cwd"] != t["cwd"] {
            sample.partial = true;
            continue;
        }
        let b = Reading {
            activity: activity(&recheck),
            turn: c.turn(id)?,
        };
        sample.partial |= a.turn == Turn::Unavailable || b.turn == Turn::Unavailable;
        sample.rows.push(Row {
            id: id.into(),
            evidence: Evidence::Managed(a, b),
        });
    }
    Ok((version, Ok(sample)))
}
pub fn poll(cfg: &ConfigSource, stop: Arc<AtomicBool>) -> Poll {
    let deadline = Instant::now() + Duration::from_secs(6);
    let result = (|| -> Result<Poll> {
        if !cfg!(windows) {
            return Ok((None, Err("unsupported_runtime")));
        }
        availability(cfg)?;
        if cfg.mode != Mode::ClaudePassive {
            let path = cfg.socket.as_ref().ok_or("invalid_config")?;
            let owner = crate::platform::socket(path)?;
            let before = owner.clone();
            let config = cfg.clone();
            let mut cmd = Command::new(&cfg.binary);
            cmd.args(["app-server", "proxy", "--sock"]).arg(path);
            let value = child(
                cmd,
                &stop,
                deadline,
                move |i, o| codex(i, o, &config, owner),
                false,
            )?;
            if crate::platform::socket(path)? != before {
                return Err("owner_changed");
            }
            return Ok(value);
        }
        let identity = crate::platform::executable(&cfg.binary)?;
        let bytes = output(&cfg.binary, &["--version"], &stop, deadline, 256)?;
        let version = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|s| s.trim().strip_suffix(" (Claude Code)"))
            .and_then(semver);
        if version.as_deref() != Some(cfg.mode.version()) {
            return Ok((version, Err("unsupported_version")));
        }
        let bytes = output(
            &cfg.binary,
            &[
                "agents",
                "--json",
                "--all",
                "--cwd",
                cfg.cwd.to_str().ok_or("invalid_cwd")?,
            ],
            &stop,
            deadline,
            MAX,
        )?;
        if crate::platform::executable(&cfg.binary)? != identity {
            return Err("binary_changed");
        }
        let list: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid_json")?;
        let mut sample = Sample {
            owner: identity.clone(),
            partial: false,
            rows: vec![],
            unidentified: 0,
        };
        let mut ids = HashSet::new();
        for v in list.as_array().ok_or("invalid_list")? {
            if v["cwd"].as_str().is_none() {
                sample.partial = true;
                continue;
            }
            if v["cwd"].as_str().map(Path::new) != Some(cfg.cwd.as_path()) {
                continue;
            }
            let Ok(id) = uuid(&v["sessionId"]) else {
                sample.unidentified += 1;
                continue;
            };
            if !ids.insert(id) {
                return Err("duplicate_id");
            }
            if sample.rows.len() >= ROW_LIMIT {
                sample.partial = true;
                break;
            }
            let verified = v["pid"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .and_then(|p| crate::platform::process_binary(p).ok())
                .as_ref()
                == Some(&identity);
            let activity = match (verified, v["status"].as_str()) {
                (true, Some("idle")) => Activity::Idle,
                (true, Some("busy")) => Activity::Active,
                _ => Activity::Unknown,
            };
            sample.rows.push(Row {
                id: id.into(),
                evidence: Evidence::Claude(activity),
            });
        }
        if sample.unidentified > 0 {
            sample.partial = true
        }
        Ok((version, Ok(sample)))
    })();
    result.unwrap_or_else(|e| (None, Err(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explicit_single_wait_or_empty_flags_have_supported_active_semantics() {
        for (flags, want) in [
            (Value::Null, Activity::Unknown),
            (json!([]), Activity::Active),
            (json!(["waitingOnUserInput"]), Activity::Input),
            (
                json!(["waitingOnUserInput", "waitingOnApproval"]),
                Activity::Unsupported,
            ),
        ] {
            assert_eq!(
                activity(&json!({"status":{"type":"active","activeFlags":flags}})),
                want
            );
        }
        assert_eq!(
            activity(&json!({"type":"Stop","tool_result":"failed"})),
            Activity::Unknown
        );
    }
}

#[cfg(test)]
mod isolation_tests {
    use super::*;
    use crate::Config;
    #[test]
    fn available_managed_source_survives_missing_claude_and_claude_recovers() {
        let root = std::env::temp_dir().join(crate::api::random_key().unwrap());
        std::fs::create_dir(&root).unwrap();
        let managed = ConfigSource {
            mode: Mode::CodexManaged,
            project: "project".into(),
            binary: std::env::current_exe().unwrap(),
            cwd: root.clone(),
            socket: Some(root.join("socket")),
        };
        let claude = ConfigSource {
            mode: Mode::ClaudePassive,
            binary: root.join("claude"),
            cwd: root.join("work"),
            socket: None,
            ..managed.clone()
        };
        let cfg = Config {
            port: 32941,
            private_directory: root.clone(),
            sources: vec![managed, claude],
        };
        cfg.validate().unwrap();
        let check = || cfg.sources.iter().map(availability).collect::<Vec<_>>();
        assert_eq!(check(), [Ok(()), Err("unavailable")]);
        std::fs::create_dir(&cfg.sources[1].cwd).unwrap();
        assert_eq!(check(), [Ok(()), Err("unavailable")]);
        std::fs::File::create(&cfg.sources[1].binary).unwrap();
        assert_eq!(check(), [Ok(()), Ok(())]);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn isolated_rejection_keeps_siblings_but_invalid_replies_abort() {
        let mut sample = Sample {
            owner: "owner".into(),
            partial: false,
            unidentified: 0,
            rows: vec![],
        };
        for (id, response) in [
            (1, json!({"id":1,"result":"first"})),
            (
                2,
                json!({"id":2,"error":{"code":-32602,"message":"thread unavailable"}}),
            ),
            (3, json!({"id":3,"result":"third"})),
        ] {
            if let Some(v) = scoped_thread(reply(response, id), &mut sample).unwrap() {
                sample.rows.push(Row {
                    id: v.as_str().unwrap().into(),
                    evidence: Evidence::Discovery,
                });
            }
        }
        assert!(sample.partial);
        assert_eq!(
            sample
                .rows
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "third"]
        );
        for bad in [
            json!({"id":9,"result":{}}),
            json!({"id":1}),
            json!({"id":1,"result":{},"error":{}}),
            json!({"id":1,"error":{"code":-32603,"message":"internal"}}),
        ] {
            assert!(scoped_thread(reply(bad, 1), &mut sample).is_err());
        }
    }
}
