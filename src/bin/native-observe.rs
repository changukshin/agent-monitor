use serde_json::{json, Value};
use std::{
    env,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{ChildStdin, ChildStdout, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const LIMIT: usize = 1024 * 1024;
type Result<T> = std::result::Result<T, &'static str>;

fn session_id(value: &Value) -> Result<&str> {
    let id = value.as_str().ok_or("invalid_id")?;
    let valid = id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        });
    if valid {
        Ok(id)
    } else {
        Err("invalid_id")
    }
}

fn label<'a>(value: &'a Value, allowed: &[&str]) -> &'a str {
    value
        .as_str()
        .filter(|v| allowed.contains(v))
        .unwrap_or("unknown")
}

fn snapshot(provider: &str, rows: &Value, cwd: &Path) -> Result<Value> {
    let mut observations = Vec::new();
    for row in rows.as_array().ok_or("invalid_list")? {
        let path = row["cwd"].as_str().ok_or("missing_cwd")?;
        if Path::new(path) != cwd {
            continue;
        }
        let record = if provider == "claude" {
            let mut record = json!({
                "sessionId": session_id(&row["sessionId"]).ok(),
                "kind": label(&row["kind"], &["interactive", "background"]),
                "status": label(&row["status"], &["idle", "busy", "waiting"]),
                "state": label(&row["state"], &["working", "idle", "waiting", "blocked", "completed", "done", "failed", "stopped"])
            });
            if record["kind"] == "background" {
                if let Some(id) = row["id"]
                    .as_str()
                    .filter(|id| id.len() == 8 && id.bytes().all(|b| b.is_ascii_hexdigit()))
                {
                    record["backgroundId"] = json!(id);
                }
            }
            if record["status"] == "waiting" {
                record["waitingFor"] = json!(label(
                    &row["waitingFor"],
                    &[
                        "permission prompt",
                        "input needed",
                        "sandbox request",
                        "worker request",
                        "dialog open"
                    ]
                ));
            }
            record
        } else {
            let status = &row["status"];
            let flags: Vec<_> = status["activeFlags"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| label(v, &["waitingOnApproval", "waitingOnUserInput"]))
                .collect();
            json!({"sessionId":session_id(&row["id"])?,
                "status":label(&status["type"], &["notLoaded","idle","active","systemError"]),
                "flags":flags})
        };
        observations.push(record);
    }
    Ok(json!({"provider":provider,"sessions":observations}))
}

fn response(reader: &mut impl BufRead, id: u8, remaining: &mut usize) -> Result<Value> {
    loop {
        let mut line = Vec::new();
        let n = reader
            .by_ref()
            .take(*remaining as u64 + 1)
            .read_until(b'\n', &mut line)
            .map_err(|_| "read_error")?;
        if n == 0 {
            return Err("unavailable");
        }
        *remaining = remaining.checked_sub(n).ok_or("output_limit")?;
        let message: Value = serde_json::from_slice(&line).map_err(|_| "invalid_json")?;
        if message.get("method").is_none() && message["id"] == id {
            return message.get("result").cloned().ok_or("rpc_error");
        }
    }
}

fn send(pipe: &mut ChildStdin, message: Value) -> Result<()> {
    writeln!(pipe, "{message}")
        .and_then(|_| pipe.flush())
        .map_err(|_| "unavailable")
}

fn inspect(
    provider: &str,
    mut input: ChildStdin,
    output: ChildStdout,
    cwd: &Path,
) -> Result<Value> {
    if provider == "claude" {
        drop(input);
        let mut data = Vec::new();
        output
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut data)
            .map_err(|_| "read_error")?;
        if data.len() > LIMIT {
            return Err("output_limit");
        }
        return snapshot(
            provider,
            &serde_json::from_slice(&data).map_err(|_| "invalid_json")?,
            cwd,
        );
    }
    if provider == "codex-managed" {
        return managed_sample(input, output, cwd);
    }
    let mut reader = BufReader::new(output);
    let mut remaining = LIMIT;
    send(
        &mut input,
        json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"observation-spike","version":"0.1"}}}),
    )?;
    response(&mut reader, 1, &mut remaining)?;
    send(&mut input, json!({"method":"initialized"}))?;
    send(
        &mut input,
        json!({"id":2,"method":"thread/list","params":{"cwd":cwd,"limit":32,"useStateDbOnly":true}}),
    )?;
    let result = response(&mut reader, 2, &mut remaining)?;
    codex_page(&result, cwd)
}

// Query only threads loaded by the explicitly selected owner. No thread
// creation, subscription, resume or approval response is part of this client.
struct ProxyPipe(ChildStdout, ChildStdin);

impl Read for ProxyPipe {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for ProxyPipe {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.1.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.1.flush()
    }
}

fn managed_sample(input: ChildStdin, output: ChildStdout, cwd: &Path) -> Result<Value> {
    use tungstenite::{client::client_with_config, protocol::WebSocketConfig, Message};
    let config = WebSocketConfig::default()
        .max_message_size(Some(LIMIT))
        .max_frame_size(Some(LIMIT));
    // The HTTP upgrade runs inside the proxy pipe, not over local TCP.
    let (mut socket, _) =
        client_with_config("ws://localhost/", ProxyPipe(output, input), Some(config))
            .map_err(|_| "owner_unavailable")?;
    let mut request_id = 0u8;
    let mut budget = LIMIT;
    let mut query = |socket: &mut tungstenite::WebSocket<ProxyPipe>,
                     method: &str,
                     params: Value|
     -> Result<Value> {
        request_id += 1;
        socket
            .send(Message::text(
                json!({"id":request_id,"method":method,"params":params}).to_string(),
            ))
            .map_err(|_| "owner_unavailable")?;
        loop {
            let msg = socket.read().map_err(|_| "owner_unavailable")?;
            budget = budget.checked_sub(msg.len()).ok_or("output_limit")?;
            match msg {
                Message::Text(text) => {
                    let reply: Value = serde_json::from_str(&text).map_err(|_| "invalid_json")?;
                    if reply.get("method").is_none() && reply["id"] == request_id {
                        return reply.get("result").cloned().ok_or("rpc_error");
                    }
                }
                Message::Close(_) => return Err("owner_unavailable"),
                _ => continue,
            }
        }
    };
    query(
        &mut socket,
        "initialize",
        json!({"clientInfo":{"name":"observation-spike","version":"0.1"},
        "capabilities":{"experimentalApi":true}}),
    )?;
    socket
        .send(Message::text(r#"{"method":"initialized"}"#))
        .map_err(|_| "owner_unavailable")?;
    let page = query(&mut socket, "thread/loaded/list", json!({"limit":8}))?;
    let threads = page["data"].as_array().ok_or("invalid_list")?;
    if threads.len() > 8 {
        return Err("invalid_list");
    }
    let mut sessions = Vec::new();
    for thread in threads {
        let id = session_id(thread)?;
        let result = query(
            &mut socket,
            "thread/read",
            json!({"threadId":id,"includeTurns":false}),
        )?;
        if result["thread"]["id"] != id {
            return Err("identity_mismatch");
        }
        if result["thread"]["ephemeral"] == true {
            continue;
        }
        let sample = snapshot("codex", &json!([result["thread"]]), cwd)?;
        if let Some(mut record) = sample["sessions"][0].as_object().cloned() {
            let turns = query(
                &mut socket,
                "thread/turns/list",
                json!({"threadId":id,"limit":1,
                "sortDirection":"desc","itemsView":"notLoaded"}),
            );
            match turns {
                Ok(page) => {
                    record.insert("latestTurn".into(), latest_turn(&page)?);
                }
                // The owner refuses history queries before the first message.
                // Omit the field on RPC failure; do not invent an empty turn.
                Err("rpc_error") => {}
                Err(other) => return Err(other),
            }
            sessions.push(record);
        }
    }
    Ok(
        json!({"provider":"codex","mode":"managed","sessions":sessions,
              "partial":!page["nextCursor"].is_null()}),
    )
}

fn latest_turn(response: &Value) -> Result<Value> {
    let rows = response["data"].as_array().ok_or("invalid_list")?;
    match rows.as_slice() {
        [] => Ok(Value::Null),
        [turn] => Ok(json!({"id":session_id(&turn["id"])?,
            "status":label(&turn["status"], &["inProgress","completed","interrupted","failed"])})),
        _ => Err("invalid_list"),
    }
}

fn codex_page(page: &Value, cwd: &Path) -> Result<Value> {
    let mut sample = snapshot("codex", &page["data"], cwd)?;
    sample["partial"] = json!(!page["nextCursor"].is_null());
    Ok(sample)
}

fn run() -> Result<Value> {
    let args: Vec<_> = env::args().skip(1).collect();
    let (provider, binary, directory, socket) = match args.as_slice() {
        [provider, binary, directory] => (provider, binary, directory, None),
        [provider, binary, directory, socket]
            if provider == "codex-managed" && Path::new(socket).is_absolute() =>
        {
            (provider, binary, directory, Some(socket))
        }
        _ => return Err("usage"),
    };
    if !Path::new(binary).is_absolute() || !Path::new(directory).is_absolute() {
        return Err("absolute_path_required");
    }
    let args: Vec<&str> = match provider.as_str() {
        "claude" => vec!["agents", "--json", "--all", "--cwd", directory],
        "codex" => vec!["app-server", "proxy"],
        "codex-managed" => vec![
            "app-server",
            "proxy",
            "--sock",
            socket.ok_or("explicit_socket_required")?,
        ],
        _ => return Err("usage"),
    };
    let listing = provider == "claude";
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "unavailable")?;
    let input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let provider = provider.clone();
    let directory = directory.clone();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(inspect(&provider, input, output, Path::new(&directory)));
    });
    let result = rx
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .unwrap_or(Err("timeout"));
    // Claude's listing must exit successfully. The Codex proxy is a persistent
    // connection: close our client once the correlated list response arrives.
    if listing && result.is_ok() {
        while Instant::now() < deadline {
            match child.try_wait().map_err(|_| "process_error")? {
                Some(status) if status.success() => return result,
                Some(_) => return Err("listing_failed"),
                None => thread::sleep(Duration::from_millis(5)),
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    if listing && result.is_ok() {
        return Err("timeout");
    }
    result
}

fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(reason) => {
            eprintln!("{reason}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn listing_drops_private_fields_and_preserves_unknowns() {
        let rows = json!([{"cwd":"/trial","kind":"interactive","sessionId":"aaaaaaaa-1234-1234-abcd-111111111111",
            "name":"private","status":"private"},{"cwd":"/elsewhere","name":"private"}]);
        let result = snapshot("claude", &rows, Path::new("/trial")).unwrap();
        assert_eq!(result["sessions"][0]["status"], "unknown");
        assert_eq!(result["sessions"].as_array().unwrap().len(), 1);
        assert!(!result.to_string().contains("private"));
    }
    #[test]
    fn samples_keep_provider_facts_separate_from_turn_outcome() {
        let id = "aaaaaaaa-1234-1234-abcd-111111111111";
        let rows = json!([{"cwd":"/trial","id":id,"preview":"private",
            "status":{"type":"active","activeFlags":["waitingOnUserInput","private"]}}]);
        let sample = snapshot("codex", &rows, Path::new("/trial")).unwrap();
        assert_eq!(
            sample["sessions"][0]["flags"],
            json!(["waitingOnUserInput", "unknown"])
        );
        assert!(!sample.to_string().contains("private"));
        let rows = json!([{"cwd":"/trial","sessionId":id,"kind":"background","state":"blocked","status":"idle"}]);
        let sample = snapshot("claude", &rows, Path::new("/trial")).unwrap();
        assert_eq!(sample["sessions"][0]["state"], "blocked");
        assert_eq!(sample["sessions"][0]["status"], "idle");
    }

    #[test]
    fn claude_optional_metadata_is_scoped_and_sanitized() {
        let reasons = [
            "permission prompt",
            "input needed",
            "sandbox request",
            "worker request",
            "dialog open",
        ];
        for reason in reasons {
            let rows = json!([
                {"cwd":"/trial","kind":"interactive","status":"waiting","waitingFor":reason},
                {"cwd":"/trial","kind":"background","id":"abc01234","state":"done","waitingFor":"private"},
                {"cwd":"/trial","kind":"background","id":"private","sessionId":"private","status":"waiting","waitingFor":"private"}
            ]);
            let sample = snapshot("claude", &rows, Path::new("/trial")).unwrap();
            let rows = &sample["sessions"];
            assert_eq!(rows[0]["waitingFor"], reason);
            assert!(rows[0]["sessionId"].is_null());
            assert_eq!(rows[1]["backgroundId"], "abc01234");
            assert_eq!(rows[1]["state"], "done");
            assert!(rows[1].get("waitingFor").is_none());
            assert_eq!(rows[2]["waitingFor"], "unknown");
            assert!(!sample.to_string().contains("private"));
        }
    }

    #[test]
    fn truncated_page_keeps_rows_without_disclosing_cursor() {
        for (cursor, partial) in [(json!("private"), true), (Value::Null, false)] {
            let page = json!({"nextCursor":cursor,"data":[{"cwd":"/trial",
                "id":"aaaaaaaa-1234-1234-abcd-111111111111","status":{"type":"idle"}}]});
            let sample = codex_page(&page, Path::new("/trial")).unwrap();
            assert_eq!(sample["partial"], partial);
            assert_eq!(sample["sessions"].as_array().unwrap().len(), 1);
            assert!(!sample.to_string().contains("private"));
        }
    }

    #[test]
    fn turn_metadata_drops_content_and_requires_valid_identity() {
        let id = "aaaaaaaa-1234-1234-abcd-111111111111";
        assert_eq!(latest_turn(&json!({"data":[]})).unwrap(), Value::Null);
        for status in ["completed", "interrupted", "failed", "inProgress"] {
            let result =
                latest_turn(&json!({"data":[{"id":id,"status":status,"items":["private"]}]}))
                    .unwrap();
            assert_eq!(result, json!({"id":id,"status":status}));
        }
        assert!(latest_turn(&json!({"data":[{"id":"private"}]})).is_err());
    }

    #[test]
    fn rpc_requires_matching_response_within_budget() {
        let data = b"{\"id\":1,\"method\":\"server/request\"}\n{\"id\":1,\"result\":true}\n";
        let mut remaining = LIMIT;
        assert_eq!(response(&mut &data[..], 1, &mut remaining).unwrap(), true);
        assert_eq!(response(&mut &data[..], 1, &mut 4), Err("output_limit"));
    }
}
