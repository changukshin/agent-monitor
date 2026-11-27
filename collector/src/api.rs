use crate::{model::Store, Result};
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tungstenite::{
    accept_hdr_with_config,
    handshake::server::{ErrorResponse, Request, Response},
    http::{HeaderMap, StatusCode},
    protocol::WebSocketConfig,
    Message,
};

pub fn random_key() -> Result<String> {
    let mut b = [0; 32];
    OsRng
        .try_fill_bytes(&mut b)
        .map_err(|_| "random_unavailable")?;
    Ok(b.iter().map(|v| format!("{v:02x}")).collect())
}
pub struct Credential {
    pub secret: String,
    path: PathBuf,
    _lock: File,
}
impl Credential {
    pub fn create(dir: &Path) -> Result<Self> {
        crate::platform::directory(dir)?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0).custom_flags(0x00200000);
        }
        let lock = options
            .open(dir.join("collector.lock"))
            .map_err(|_| "collector_lock")?;
        let path = dir.join("dashboard.token");
        let secret = random_key()?;
        let temp = dir.join(format!(".token-{}", random_key()?));
        let mut f = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|_| "token_create")?;
        if crate::platform::private_file(&f).is_err() {
            drop(f);
            let _ = fs::remove_file(&temp);
            return Err("acl_not_private");
        }
        let saved = f.write_all(secret.as_bytes()).and_then(|_| f.sync_all());
        drop(f);
        if saved.is_err() || fs::rename(&temp, &path).is_err() {
            let _ = fs::remove_file(&temp);
            return Err("token_install");
        }
        Ok(Self {
            secret,
            path,
            _lock: lock,
        })
    }
}
impl Drop for Credential {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
struct Timed {
    socket: TcpStream,
    end: Instant,
}
impl Timed {
    fn left(&self) -> std::io::Result<Duration> {
        self.end
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(std::io::ErrorKind::TimedOut.into())
    }
}
impl Read for Timed {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.socket.set_read_timeout(Some(self.left()?))?;
        self.socket.read(b)
    }
}
impl Write for Timed {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.socket.set_write_timeout(Some(self.left()?))?;
        self.socket.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.socket.flush()
    }
}
fn single<'a>(h: &'a HeaderMap, key: &str) -> Option<&'a [u8]> {
    let mut v = h.get_all(key).iter();
    let first = v.next()?.as_bytes();
    v.next().is_none().then_some(first)
}
fn authorize(req: &Request, addr: SocketAddr, secret: &str) -> bool {
    let h = req.headers();
    req.uri().path_and_query().map(|s| s.as_str()) == Some("/v1/sessions")
        && req.uri().authority().is_none()
        && single(h, "host") == Some(addr.to_string().as_bytes())
        && !h.contains_key("origin")
        && single(h, "authorization")
            .and_then(|s| s.strip_prefix(b"Bearer "))
            .is_some_and(|s| bool::from(s.ct_eq(secret.as_bytes())))
}
pub fn serve(
    socket: TcpStream,
    addr: SocketAddr,
    secret: &str,
    instance: &str,
    store: Arc<Mutex<Store>>,
    stop: Arc<AtomicBool>,
) {
    let stream = Timed {
        socket,
        end: Instant::now() + Duration::from_secs(2),
    };
    let callback = |req: &Request, res: Response| -> std::result::Result<Response, ErrorResponse> {
        if authorize(req, addr, secret) {
            Ok(res)
        } else {
            let mut r = ErrorResponse::new(None);
            *r.status_mut() = StatusCode::FORBIDDEN;
            Err(r)
        }
    };
    let cfg = WebSocketConfig::default()
        .max_frame_size(Some(1024))
        .max_message_size(Some(1024));
    let Ok(mut ws) = accept_hdr_with_config(stream, callback, Some(cfg)) else {
        return;
    };
    let mut previous = None;
    while !stop.load(Ordering::Relaxed) {
        let frame = {
            let s = store.lock().unwrap();
            let rev = s.revision;
            if previous == Some(rev) {
                None
            } else {
                let consecutive = previous.and_then(|p: u64| p.checked_add(1)) == Some(rev);
                let alerts = if consecutive {
                    s.alerts.as_slice()
                } else {
                    &[]
                };
                let value = json!({"schema":1,"instance":instance,"runtime":"windows","revision":rev,"sampledAtMs":crate::wall(),
                    "type":if consecutive{"update"}else{"snapshot"},"sources":s.sources,"alerts":alerts});
                previous = Some(rev);
                Some(value.to_string())
            }
        };
        ws.get_mut().end = Instant::now() + Duration::from_millis(800);
        // Heartbeats also detect disconnected readers when all sources are down.
        let message = frame
            .map(Message::text)
            .unwrap_or_else(|| Message::Ping(Default::default()));
        if ws.send(message).is_err() {
            break;
        }
        // Drain control frames or close on application input. Do not buffer
        // unsolicited messages or allow the client to invoke any action.
        let mut byte = [0];
        let tcp = &ws.get_ref().socket;
        if tcp.set_nonblocking(true).is_err() {
            break;
        }
        let available = tcp.peek(&mut byte);
        if tcp.set_nonblocking(false).is_err() {
            break;
        }
        match available {
            Ok(0) => break,
            Ok(_) => {
                if !matches!(ws.read(), Ok(Message::Pong(_) | Message::Ping(_))) {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            _ => break,
        }
        thread::sleep(Duration::from_millis(250));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundary() {
        let addr = "127.0.0.1:9870".parse().unwrap();
        let mut r = Request::builder()
            .uri("/v1/sessions")
            .header("host", "127.0.0.1:9870")
            .header("authorization", "Bearer k")
            .body(())
            .unwrap();
        assert!(authorize(&r, addr, "k"));
        assert!(!authorize(&r, addr, "bad"));
        r.headers_mut().insert("origin", "null".parse().unwrap());
        assert!(!authorize(&r, addr, "k"));
        r.headers_mut().remove("origin");
        r.headers_mut().append("host", "localhost".parse().unwrap());
        assert!(!authorize(&r, addr, "k"));
    }
}

#[cfg(test)]
mod subscription_tests;
