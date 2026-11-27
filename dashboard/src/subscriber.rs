use crate::view::{now, Feed, Frame, Notice};
use std::{
    fs::File,
    io::Read,
    net::{SocketAddr, TcpStream},
    path::Path,
    sync::{
        mpsc::{sync_channel, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{
    client::client_with_config, client::IntoClientRequest, protocol::WebSocketConfig, Message,
};
#[derive(Default)]
pub struct Shared {
    pub feed: Feed,
    pub enabled: bool,
    pub notification: Option<Diagnostic>,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Diagnostic {
    Submitted,
    SubmittedBeforeSettings,
    Error(&'static str, i32),
    Setting(i32),
}
impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Submitted => write!(f, "Notification submitted"),
            Self::SubmittedBeforeSettings => {
                write!(f, "Notification submitted (initial settings 0x80070490)")
            }
            Self::Error(stage, hr) => write!(f, "Notification: {stage} (0x{:08X})", *hr as u32),
            Self::Setting(value) => write!(
                f,
                "Notification setting: {} ({value})",
                match value {
                    1 => "disabled for application",
                    2 => "disabled for user",
                    3 => "disabled by group policy",
                    4 => "disabled by manifest",
                    _ => "unknown",
                }
            ),
        }
    }
}
// Windows creates the per-app settings record on the first ordinary Show.
pub fn notification_settings(result: Result<i32, i32>) -> Result<Diagnostic, Diagnostic> {
    match result {
        Ok(0) => Ok(Diagnostic::Submitted),
        Err(hr) if hr == 0x80070490u32 as i32 => Ok(Diagnostic::SubmittedBeforeSettings),
        Ok(value) => Err(Diagnostic::Setting(value)),
        Err(hr) => Err(Diagnostic::Error("settings query", hr)),
    }
}
struct Job {
    notice: Notice,
    generation: u64,
    queued: Instant,
}
fn delivery(
    state: Arc<Mutex<Shared>>,
    sink: impl Fn(&str, &dyn Fn() -> bool) -> Option<Diagnostic> + Send + 'static,
) -> SyncSender<Job> {
    let (tx, rx) = sync_channel::<Job>(8);
    thread::spawn(move || {
        for job in rx {
            let current = || {
                let s = state.lock().unwrap();
                s.enabled
                    && job.generation == s.generation
                    && job.queued.elapsed() < Duration::from_secs(5)
                    && s.feed
                        .frame
                        .as_ref()
                        .is_some_and(|f| job.notice.current(f, now()))
            };
            if current() {
                if let Some(result) = sink(job.notice.label, &current) {
                    let mut s = state.lock().unwrap();
                    if s.enabled && s.generation == job.generation {
                        s.notification = Some(result);
                    }
                }
            }
        }
    });
    tx
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
    queue: &SyncSender<Job>,
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
                let mut s = state.lock().unwrap();
                let enabled = s.enabled;
                let reset = frame.kind == "snapshot"
                    && s.feed.frame.as_ref().is_none_or(|old| {
                        old.instance != frame.instance || old.revision < frame.revision
                    });
                let notices = s.feed.accept(frame, now(), enabled)?;
                if reset {
                    s.generation += 1;
                }
                for notice in notices {
                    let _ = queue.try_send(Job {
                        notice,
                        generation: s.generation,
                        queued: Instant::now(),
                    });
                }
                *received = true;
            }
            Message::Ping(_) => ws.flush().map_err(|_| ())?,
            Message::Pong(_) => (),
            _ => return Err(()),
        }
    }
}
pub fn start(
    port: u16,
    dir: std::path::PathBuf,
    state: Arc<Mutex<Shared>>,
    notify: fn(&str, &dyn Fn() -> bool) -> Option<Diagnostic>,
) {
    let queue = delivery(state.clone(), notify);
    thread::spawn(move || {
        let mut delay = 1;
        loop {
            let mut received = false;
            let _ = listen(
                ([127, 0, 0, 1], port).into(),
                &dir,
                &state,
                &queue,
                &mut received,
            );
            {
                let mut s = state.lock().unwrap();
                s.feed.frame = None;
                s.generation += 1;
            }
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
mod wire_tests;

#[cfg(test)]
mod settings_tests {
    use super::*;
    #[test]
    fn only_absent_settings_allow_first_native_submission() {
        assert_eq!(notification_settings(Ok(0)), Ok(Diagnostic::Submitted));
        assert_eq!(
            notification_settings(Err(0x80070490u32 as i32)),
            Ok(Diagnostic::SubmittedBeforeSettings)
        );
        for denied in 1..=4 {
            assert_eq!(
                notification_settings(Ok(denied)),
                Err(Diagnostic::Setting(denied))
            );
        }
        assert_eq!(
            notification_settings(Err(-1)),
            Err(Diagnostic::Error("settings query", -1))
        );
    }
}
