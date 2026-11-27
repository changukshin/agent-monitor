mod api;
mod model;
mod provider;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(not(windows))]
mod platform {
    use std::path::Path;
    pub fn directory(_: &Path) -> super::Result<()> {
        Err("unsupported_runtime")
    }
    pub fn socket(_: &Path) -> super::Result<String> {
        Err("unsupported_runtime")
    }
    pub fn executable(_: &Path) -> super::Result<String> {
        Err("unsupported_runtime")
    }
    pub fn process_binary(_: u32) -> super::Result<String> {
        Err("unsupported_runtime")
    }
    pub fn private_file(_: &std::fs::File) -> super::Result<()> {
        Err("unsupported_runtime")
    }
}
use model::{Mode, Store};
use serde::Deserialize;
use std::{
    collections::HashSet,
    env,
    fs::File,
    io::Read,
    net::{Ipv4Addr, TcpListener},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
type Result<T> = std::result::Result<T, &'static str>;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigSource {
    mode: Mode,
    project: String,
    binary: PathBuf,
    cwd: PathBuf,
    socket: Option<PathBuf>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    port: u16,
    private_directory: PathBuf,
    sources: Vec<ConfigSource>,
}
impl Config {
    fn validate(&self) -> Result<()> {
        if self.port == 0
            || self.sources.is_empty()
            || self.sources.len() > 8
            || !self.private_directory.is_absolute()
        {
            return Err("invalid_config");
        }
        let mut scopes = HashSet::new();
        for s in &self.sources {
            if !s.binary.is_absolute()
                || !s.cwd.is_absolute()
                || !s.cwd.is_dir()
                || !s.binary.is_file()
                || s.project.is_empty()
                || s.project.len() > 64
                || s.project.chars().any(char::is_control)
                || (s.mode == Mode::ClaudePassive) != s.socket.is_none()
                || s.socket.as_ref().is_some_and(|p| !p.is_absolute())
                || !scopes.insert((s.mode as u8, &s.cwd, &s.socket))
            {
                return Err("invalid_config");
            }
        }
        Ok(())
    }
}
fn wall() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn wait(stop: &AtomicBool, delay: Duration) {
    let end = Instant::now() + delay;
    while !stop.load(Ordering::Relaxed) && Instant::now() < end {
        thread::sleep(
            end.saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100)),
        );
    }
}
fn run() -> Result<()> {
    if !cfg!(windows) {
        return Err("unsupported_runtime");
    }
    let mut args = env::args_os().skip(1);
    let file = args
        .next()
        .ok_or("usage: agent-monitor-collector CONFIG.json")?;
    if args.next().is_some() {
        return Err("invalid_config");
    }
    let mut bytes = vec![];
    File::open(file)
        .map_err(|_| "config_unavailable")?
        .take(32769)
        .read_to_end(&mut bytes)
        .map_err(|_| "config_unavailable")?;
    if bytes.len() > 32768 {
        return Err("config_too_large");
    }
    let config: Config = serde_json::from_slice(&bytes).map_err(|_| "invalid_config")?;
    config.validate()?;
    let listener =
        TcpListener::bind((Ipv4Addr::LOCALHOST, config.port)).map_err(|_| "bind_failed")?;
    listener.set_nonblocking(true).map_err(|_| "bind_failed")?;
    let addr = listener.local_addr().map_err(|_| "bind_failed")?;
    let credential = api::Credential::create(&config.private_directory)?;
    let instance = api::random_key()?;
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed))
        .map_err(|_| "signal_setup")?;
    let store = Arc::new(Mutex::new(Store::new(
        config.sources.iter().map(|s| (s.mode, s.project.clone())),
    )));
    let clock = Instant::now();
    let mut workers = Vec::new();
    for (idx, source) in config.sources.into_iter().enumerate() {
        let store = store.clone();
        let stop = stop.clone();
        workers.push(thread::spawn(move || {
            let interval = Duration::from_secs(if source.mode == Mode::ClaudePassive {
                3
            } else {
                1
            });
            let mut failures = 0;
            let mut seq = 0;
            while !stop.load(Ordering::Relaxed) {
                let start = clock.elapsed().as_millis() as u64;
                let observed = wall();
                let (version, result) = provider::poll(&source, stop.clone());
                failures = if result.is_err() {
                    (failures + 1).min(5)
                } else {
                    0
                };
                seq += 1;
                let cost = clock.elapsed().as_millis() as u64 - start;
                store
                    .lock()
                    .unwrap()
                    .apply(idx, seq, start, observed, cost, version, result);
                wait(
                    &stop,
                    (interval * (1 << failures)).min(Duration::from_secs(30)),
                );
            }
        }));
    }
    let mut clients: Vec<thread::JoinHandle<()>> = vec![];
    while !stop.load(Ordering::Relaxed) {
        store
            .lock()
            .unwrap()
            .expire(clock.elapsed().as_millis() as u64, wall());
        clients.retain(|t| !t.is_finished());
        match listener.accept() {
            Ok((tcp, peer)) if peer.ip().is_loopback() && clients.len() < 8 => {
                let (store, stop, secret, id) = (
                    store.clone(),
                    stop.clone(),
                    credential.secret.clone(),
                    instance.clone(),
                );
                clients.push(thread::spawn(move || {
                    api::serve(tcp, addr, &secret, &id, store, stop)
                }));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    for t in workers.into_iter().chain(clients) {
        let _ = t.join();
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1)
    }
}
