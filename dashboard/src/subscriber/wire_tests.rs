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
    assert!(listen(addr, &dir, &state, |_| panic!("no alerts"), &mut received).is_err());
    assert!(received);
    server.join().unwrap();
    fs::remove_dir_all(dir).unwrap();
}
