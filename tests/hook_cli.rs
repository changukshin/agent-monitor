use std::{io::Write, process::{Command, Stdio}, time::{Duration, Instant}};
fn hook(args: &[&str], input: Option<&[u8]>) -> (std::process::Output, Duration) {
    let start=Instant::now();
    let mut child=Command::new(env!("CARGO_BIN_EXE_agent-monitor-spike"))
        .args(args).env_remove("AM_SPIKE_DIAG_FILE")
        .env("AM_SPIKE_TOKEN_FILE",std::env::temp_dir().join(format!("am-nonexistent-token-{}",std::process::id())))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    if let Some(bytes)=input {
        let _=child.stdin.take().unwrap().write_all(bytes);
    }
    // Keep stdin open for the incomplete-input case; let the watchdog terminate.
    let result=if input.is_none() {
        let pipe=child.stdin.take().unwrap();
        let output=child.wait_with_output().unwrap();
        drop(pipe); output
    } else { child.wait_with_output().unwrap() };
    (result,start.elapsed())
}
#[test]
fn neutral_candidate_output_on_errors() {
    for (provider,expected) in [("codex","{}\n"),("claude","")] {
        let (out,elapsed)=hook(&["hook",provider,"Stop","native"],Some(b"{bad"));
        assert!(out.status.success()); assert_eq!(out.stdout,expected.as_bytes()); assert!(out.stderr.is_empty());
        assert!(elapsed<Duration::from_secs(3));
    }
}
#[test]
fn missing_token_and_watchdog() {
    let raw=br#"{"hook_event_name":"Stop","session_id":"s1"}"#;
    for input in [Some(raw.as_slice()),None] {
        let (out,elapsed)=hook(&["hook","codex","Stop","native"],input);
        assert!(out.status.success()); assert_eq!(out.stdout,b"{}\n"); assert!(out.stderr.is_empty());
        assert!(elapsed<Duration::from_secs(3));
    }
}
#[test]
fn missing_relay_is_fail_open() {
    let exe=std::env::temp_dir().join(format!("am-nonexistent-relay-{}",std::process::id()));
    let (out,_)=hook(&["hook","codex","Stop","wsl",exe.to_str().unwrap()],Some(br#"{"hook_event_name":"Stop","session_id":"s1"}"#));
    assert!(out.status.success()); assert_eq!(out.stdout,b"{}\n"); assert!(out.stderr.is_empty());
}
