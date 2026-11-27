use super::*;
use std::{fs, net::TcpListener};
use tungstenite::handshake::server::{Request, Response};
#[test]
fn local_handshake_ping_and_invalid_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let dir = std::env::temp_dir().join(format!("dash-wire-{}", addr.port()));
    fs::create_dir(&dir).unwrap();
    let secret = "a".repeat(64);
    fs::write(dir.join("dashboard.token"), &secret).unwrap();
    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut ws = tungstenite::accept_hdr(tcp, |req: &Request, res: Response| {
            assert_eq!(req.uri().path(), "/v1/sessions");
            assert_eq!(req.headers()["host"], addr.to_string());
            assert_eq!(req.headers()["authorization"], format!("Bearer {secret}"));
            assert!(!req.headers().contains_key("origin"));
            Ok(res)
        })
        .unwrap();
        ws.send(Message::Ping(vec![1].into())).unwrap();
        assert_eq!(ws.read().unwrap(), Message::Pong(vec![1].into()));
        ws.send(Message::text(serde_json::json!({"schema":1,"instance":"a".repeat(64),"runtime":"windows","revision":1,"type":"snapshot","sampledAtMs":now(),"sources":[],"alerts":[]}).to_string())).unwrap();
        ws.send(Message::text("x".repeat(1024 * 1024 + 1))).ok();
    });
    let state = Mutex::new(Shared::default());
    let mut received = false;
    assert!(listen(addr, &dir, &state, &sync_channel(1).0, &mut received).is_err());
    assert!(received);
    server.join().unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stalled_and_failing_delivery_does_not_block_websocket_revisions() {
    check_delivery_isolation(Diagnostic::Error("settings query", 0x80070490u32 as i32));
}
#[test]
fn disabled_os_policy_does_not_block_websocket_revisions() {
    check_delivery_isolation(Diagnostic::Setting(3));
}
fn check_delivery_isolation(result: Diagnostic) {
    use serde_json::json;
    use std::sync::mpsc::channel;
    const SID: &str = "10000000-0000-0000-0000-000000000001";
    const TURN: &str = "20000000-0000-0000-0000-000000000002";
    let addr_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = addr_listener.local_addr().unwrap();
    let dir = std::env::temp_dir().join(format!("dash-delay-{}", addr.port()));
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("dashboard.token"), "d".repeat(64)).unwrap();
    let state = Arc::new(Mutex::new(Shared {
        enabled: true,
        ..Default::default()
    }));
    let (entered, started) = channel();
    let (release, gate) = channel();
    let sender = delivery(state.clone(), move |_, current| {
        entered.send(()).unwrap();
        gate.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(
            !current(),
            "cleared notification became obsolete while Windows was busy"
        );
        Some(result)
    });
    let client_state = state.clone();
    let client_dir = dir.clone();
    let client =
        thread::spawn(move || listen(addr, &client_dir, &client_state, &sender, &mut false));
    let mut ws = tungstenite::accept(addr_listener.accept().unwrap().0).unwrap();
    let send = |ws: &mut tungstenite::WebSocket<TcpStream>, rev, activity| {
        let time = now();
        let alerts = if activity == "waiting_input" {
            vec![json!({"kind":"input_wait","source":0,"session":SID,"turn":TURN})]
        } else {
            vec![]
        };
        ws.send(Message::text(json!({"schema":1,"runtime":"windows","type":if rev==1{"snapshot"}else{"update"},"revision":rev,"instance":"a".repeat(64),"sampledAtMs":time,"alerts":alerts,"sources":[{"mode":"codex_managed","version":"0.167.0","project":"Test","owner":"owner","sequence":rev,"coverage":"complete","sessions":{(SID):{"activity":activity,"latest_turn":{"kind":"known","value":{"id":TURN,"status":"active"}},"visibility":"present","confidence":"confirmed","observed_ms":time,"changed_ms":time,"sequence":rev}}}]}).to_string())).unwrap();
    };
    send(&mut ws, 1, "running");
    send(&mut ws, 2, "waiting_input");
    started.recv_timeout(Duration::from_secs(3)).unwrap();
    send(&mut ws, 3, "running");
    let wait = |condition: &dyn Fn(&Shared) -> bool| {
        let limit = Instant::now() + Duration::from_secs(2);
        while !condition(&state.lock().unwrap()) {
            assert!(Instant::now() < limit);
            thread::sleep(Duration::from_millis(5));
        }
    };
    wait(&|s| s.feed.frame.as_ref().is_some_and(|f| f.revision == 3));
    release.send(()).unwrap();
    wait(&|s| s.notification == Some(result));
    send(&mut ws, 4, "running");
    wait(&|s| s.feed.frame.as_ref().is_some_and(|f| f.revision == 4));
    ws.close(None).unwrap();
    assert!(client.join().unwrap().is_err());
    fs::remove_dir_all(dir).unwrap();
}
