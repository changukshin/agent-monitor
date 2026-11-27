use super::*;
use crate::model::Alert;
use std::net::TcpListener;
use tungstenite::client::IntoClientRequest;

#[test]
fn subscriptions_start_fresh_and_skip_missed_alerts() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let store = Arc::new(Mutex::new(Store::new([])));
    let stop = Arc::new(AtomicBool::new(false));
    let s = store.clone();
    let done = stop.clone();
    let worker = thread::spawn(move || {
        serve(
            listener.accept().unwrap().0,
            addr,
            "test",
            "instance",
            s,
            done,
        )
    });
    let mut request = format!("ws://{addr}/v1/sessions")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer test".parse().unwrap());
    let stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    let (mut client, _) = tungstenite::client(request, stream).unwrap();
    let mut next = || loop {
        if let Message::Text(s) = client.read().unwrap() {
            break serde_json::from_str::<serde_json::Value>(&s).unwrap();
        }
    };
    assert_eq!(next()["type"], "snapshot");
    for (rev, expected, n) in [(1, "update", 1), (3, "snapshot", 0)] {
        {
            let mut s = store.lock().unwrap();
            s.revision = rev;
            s.alerts = vec![Alert {
                source: 0,
                session: "s".into(),
                turn: "t".into(),
                kind: "completed",
            }];
        }
        let event = next();
        assert_eq!(event["type"], expected);
        assert_eq!(event["alerts"].as_array().unwrap().len(), n);
    }
    stop.store(true, Ordering::Relaxed);
    worker.join().unwrap();
}
