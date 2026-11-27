use crate::view::{now, Feed, Frame};
use std::{
    fs::File,
    io::Read,
    net::{SocketAddr, TcpStream},
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tungstenite::{
    client::client_with_config, client::IntoClientRequest, protocol::WebSocketConfig, Message,
};
#[derive(Default)]
pub struct Shared {
    pub feed: Feed,
    pub enabled: bool,
    pub notification: Option<bool>,
}
fn token(dir: &Path) -> Result<String, ()> {
    let mut secret = String::new();
    File::open(dir.join("dashboard.token"))
        .map_err(|_| ())?
        .take(65)
        .read_to_string(&mut secret)
        .map_err(|_| ())?;
    if secret.len() != 64 || !secret.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(());
    }
    Ok(secret)
}
fn listen(
    addr: SocketAddr,
    dir: &Path,
    state: &Mutex<Shared>,
    notify: fn(&str) -> bool,
    received: &mut bool,
) -> Result<(), ()> {
    let mut req = format!("ws://{addr}/v1/sessions")
        .into_client_request()
        .map_err(|_| ())?;
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {}", token(dir)?).parse().map_err(|_| ())?,
    );
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|_| ())?;
    tcp.set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| ())?;
    tcp.set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| ())?;
    let config = WebSocketConfig::default()
        .max_frame_size(Some(1024 * 1024))
        .max_message_size(Some(1024 * 1024));
    let (mut ws, _) = client_with_config(req, tcp, Some(config)).map_err(|_| ())?;
    loop {
        match ws.read().map_err(|_| ())? {
            Message::Text(raw) => {
                let frame: Frame = serde_json::from_str(&raw).map_err(|_| ())?;
                let notices = {
                    let mut s = state.lock().unwrap();
                    let enabled = s.enabled;
                    s.feed.accept(frame, now(), enabled)?
                };
                *received = true;
                deliver(state, notices, notify);
            }
            Message::Ping(_) => ws.flush().map_err(|_| ())?,
            Message::Pong(_) => (),
            _ => return Err(()),
        }
    }
}
fn deliver(state: &Mutex<Shared>, notices: Vec<&str>, notify: fn(&str) -> bool) {
    for notice in notices {
        if !state.lock().unwrap().enabled {
            break;
        }
        let result = notify(notice);
        state.lock().unwrap().notification = Some(result);
        if !result {
            break;
        }
    }
}
pub fn start(
    port: u16,
    dir: std::path::PathBuf,
    state: Arc<Mutex<Shared>>,
    notify: fn(&str) -> bool,
) {
    thread::spawn(move || {
        let mut delay = 1;
        loop {
            let mut received = false;
            let _ = listen(
                ([127, 0, 0, 1], port).into(),
                &dir,
                &state,
                notify,
                &mut received,
            );
            state.lock().unwrap().feed.frame = None;
            if received {
                delay = 1
            }
            thread::sleep(Duration::from_secs(delay));
            delay = (delay * 2).min(8);
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_is_bounded_and_reread_after_rotation() {
        let path = std::env::temp_dir().join(format!("dashboard-token-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        for value in [
            "a".repeat(64),
            "b".repeat(64),
            "f".repeat(65),
            "g".repeat(64),
        ] {
            std::fs::write(path.join("dashboard.token"), &value).unwrap();
            assert_eq!(
                token(&path).ok(),
                (value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit()))
                    .then_some(value)
            );
        }
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    #[test]
    fn denied_toast_keeps_subscriber_state_and_disabled_skips_os_call() {
        let s = Mutex::new(Shared::default());
        deliver(&s, vec!["Input needed"], |_| panic!("disabled"));
        s.lock().unwrap().enabled = true;
        deliver(&s, vec!["Input needed"], |_| false);
        assert_eq!(s.lock().unwrap().notification, Some(false));
        deliver(&s, vec!["Turn completed"], |_| true);
        assert_eq!(s.lock().unwrap().notification, Some(true));
    }
}

#[cfg(test)]
mod wire_tests;
