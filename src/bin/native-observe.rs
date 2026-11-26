use serde_json::{json, Value};
use std::{
    env, io::{ BufRead, BufReader, Read, Write}, path::Path,
    process::{ChildStdin, ChildStdout, Command, Stdio}, sync::mpsc, thread,
    time::{Duration, Instant},
};

const LIMIT: usize = 1024 * 1024;
type Result<T> = std::result::Result<T, &'static str>;

fn session_id(value: &Value) -> Result<&str> {
    let id = value.as_str().ok_or("invalid_id")?;
    let valid = id.len() == 36 && id.bytes().enumerate().all(|(i, b)| {
        if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() }
    });
    if valid { Ok(id) } else { Err("invalid_id") }
}

fn label<'a>(value: &'a Value, allowed: &[&str]) -> &'a str {
    value.as_str().filter(|v| allowed.contains(v)).unwrap_or("unknown")
}

fn snapshot(provider: &str, rows: &Value, cwd: &Path) -> Result<Value> {
    let mut observations = Vec::new();
    for row in rows.as_array().ok_or("invalid_list")? {
        let path = row["cwd"].as_str().ok_or("missing_cwd")?;
        if Path::new(path) != cwd { continue; }
        let record = if provider == "claude" {
            json!({"sessionId":session_id(&row["sessionId"])?,
                "kind":label(&row["kind"], &["interactive","background"]),
                "status":label(&row["status"], &["idle","busy","waiting"]),
                "state":label(&row["state"], &["working","idle","waiting","completed","failed","stopped"])})
        } else {
            let status = &row["status"];
            let flags: Vec<_> = status["activeFlags"].as_array().into_iter().flatten()
                .map(|v| label(v, &["waitingOnApproval","waitingOnUserInput"])).collect();
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
        let n = reader.by_ref().take(*remaining as u64 + 1).read_until(b'\n', &mut line)
            .map_err(|_| "read_error")?;
        if n == 0 { return Err("unavailable"); }
        *remaining = remaining.checked_sub(n).ok_or("output_limit")?;
        let message: Value = serde_json::from_slice(&line).map_err(|_| "invalid_json")?;
        if message.get("method").is_none() && message["id"] == id {
            return message.get("result").cloned().ok_or("rpc_error");
        }
    }
}

fn send(pipe: &mut ChildStdin, message: Value) -> Result<()> {
    writeln!(pipe, "{message}").and_then(|_| pipe.flush()).map_err(|_| "unavailable")
}

fn inspect(provider: &str, mut input: ChildStdin, output: ChildStdout, cwd: &Path) -> Result<Value> {
    if provider == "claude" {
        drop(input);
        let mut data = Vec::new();
        output.take(LIMIT as u64 + 1).read_to_end(&mut data).map_err(|_| "read_error")?;
        if data.len() > LIMIT { return Err("output_limit"); }
        return snapshot(provider, &serde_json::from_slice(&data).map_err(|_| "invalid_json")?, cwd);
    }
    let mut reader = BufReader::new(output);
    let mut remaining = LIMIT;
    send(&mut input, json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"observation-spike","version":"0.1"}}}))?;
    response(&mut reader, 1, &mut remaining)?;
    send(&mut input, json!({"method":"initialized"}))?;
    send(&mut input, json!({"id":2,"method":"thread/list","params":{"cwd":cwd,"limit":32,"useStateDbOnly":true}}))?;
    let result = response(&mut reader, 2, &mut remaining)?;
    if !result["nextCursor"].is_null() { return Err("page_limit"); }
    snapshot(provider, &result["data"], cwd)
}

fn run() -> Result<Value> {
    let args: Vec<_> = env::args().skip(1).collect();
    let [provider, binary, directory] = args.as_slice() else { return Err("usage"); };
    if !Path::new(binary).is_absolute() || !Path::new(directory).is_absolute() { return Err("absolute_path_required"); }
    let args: Vec<&str> = match provider.as_str() {
        "claude" => vec!["agents","--json","--all","--cwd",directory],
        "codex" => vec!["app-server","proxy"],
        _ => return Err("usage"),
    };
    let listing = provider == "claude";
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut child = Command::new(binary).args(args).stdin(Stdio::piped()).stdout(Stdio::piped())
        .stderr(Stdio::null()).spawn().map_err(|_| "unavailable")?;
    let input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let provider = provider.clone();
    let directory = directory.clone();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || { let _ = tx.send(inspect(&provider,input,output,Path::new(&directory))); });
    let result = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or(Err("timeout"));
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
    if listing && result.is_ok() { return Err("timeout"); }
    result
}

fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(reason) => { eprintln!("{reason}"); std::process::exit(1); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn listing_drops_private_fields_and_preserves_unknowns() {
        let rows = json!([{"cwd":"/trial","kind":"interactive","sessionId":"aaaaaaaa-1234-1234-abcd-111111111111",
            "name":"private","status":"private"},{"cwd":"/elsewhere","name":"private"}]);
        let result = snapshot("claude",&rows,Path::new("/trial")).unwrap();
        assert_eq!(result["sessions"][0]["status"], "unknown");
        assert_eq!(result["sessions"].as_array().unwrap().len(), 1);
        assert!(!result.to_string().contains("private"));
    }
    #[test]
    fn rpc_requires_matching_response_within_budget() {
        let data = b"{\"id\":1,\"method\":\"server/request\"}\n{\"id\":1,\"result\":true}\n";
        assert_eq!(response(&mut &data[..],1,&mut LIMIT.clone()).unwrap(), true);
        assert_eq!(response(&mut &data[..],1,&mut 4), Err("output_limit"));
    }
}
