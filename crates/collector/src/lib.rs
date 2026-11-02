//! A single per-user collector serves independent local hosts over a private socket.
mod events;
mod workers;
pub use events::record_lifecycle;
use sq_contract::*;
use sq_core::*;
use sq_store::{CollectorLock, Store};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, RwLock,
    },
    thread,
    time::Duration,
};

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
pub fn state_root() -> PathBuf {
    std::env::var_os("STATUS_QUO_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| sq_sources::expand("~/.local/state"))
                .join("status-quo")
        })
}
pub struct Service {
    root: PathBuf,
    store: Arc<Store>,
    snapshot: Arc<RwLock<Snapshot>>,
    stop: Arc<AtomicBool>,
    _lock: CollectorLock,
}
impl Service {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
        let lock = CollectorLock::acquire(&root)?;
        let store = Arc::new(Store::new(root.clone())?);
        let mut snapshot = if root.join("snapshot.json").exists() {
            sq_store::read_json(&root.join("snapshot.json"))?
        } else {
            Snapshot::default()
        };
        for health in snapshot.health.values_mut() {
            health.state = SourceState::Stale;
            health.message = "Collector restarted; verifying saved observations".into();
        }
        snapshot.revision = snapshot.revision.saturating_add(1);
        Ok(Self {
            root,
            store,
            snapshot: Arc::new(RwLock::new(snapshot)),
            stop: Arc::new(AtomicBool::new(false)),
            _lock: lock,
        })
    }
    pub fn socket_path(&self) -> PathBuf {
        self.root.join("collector.sock")
    }
    pub fn stop_handle(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }
    pub fn serve(self, live: bool) -> Result<(), String> {
        let path = self.socket_path();
        if path.exists() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        let listener =
            UnixListener::bind(&path).map_err(|e| format!("Bind collector socket: {e}"))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let workers = if live {
            workers::start(self.store.clone(), self.snapshot.clone(), self.stop.clone())
        } else {
            vec![]
        };
        let clients = Arc::new(AtomicUsize::new(0));
        while !self.stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => {
                    if clients.load(Ordering::Acquire) >= 8 {
                        drop(stream);
                        continue;
                    }
                    clients.fetch_add(1, Ordering::AcqRel);
                    let clients = clients.clone();
                    let store = self.store.clone();
                    let snapshot = self.snapshot.clone();
                    thread::spawn(move || {
                        let _ = handle(stream, &store, &snapshot);
                        clients.fetch_sub(1, Ordering::AcqRel);
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20))
                }
                Err(e) => return Err(format!("Accept collector client: {e}")),
            }
        }
        for worker in workers {
            let _ = worker.join();
        }
        let _ = fs::remove_file(path);
        Ok(())
    }
}
fn handle(
    mut stream: UnixStream,
    store: &Store,
    snapshot: &RwLock<Snapshot>,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    let bytes = read_frame(&mut stream)?;
    let body = match decode_request(&bytes) {
        Err(error) => Reply::Error(error),
        Ok(request) => match request.body {
            Command::Ping => Reply::Pong,
            Command::Snapshot { after } => {
                let snapshot = snapshot.read().map_err(|_| "Snapshot unavailable")?;
                if after == Some(snapshot.revision) {
                    Reply::Unchanged {
                        revision: snapshot.revision,
                    }
                } else {
                    Reply::Snapshot(Box::new(snapshot.clone()))
                }
            }
            Command::Configuration => match store.configuration() {
                Ok(saved) => Reply::Configuration {
                    revision: saved.revision,
                    config: Box::new(saved.config),
                },
                Err(error) => Reply::Error(error),
            },
            Command::Configure { revision, action } => match store.configure(revision, action) {
                Ok(saved) => Reply::Configuration {
                    revision: saved.revision,
                    config: Box::new(saved.config),
                },
                Err(error) => Reply::Error(error),
            },
        },
    };
    write_frame(
        &mut stream,
        &serde_json::to_vec(&Response {
            version: PROTOCOL_VERSION,
            body,
        })
        .map_err(|e| e.to_string())?,
    )
}
fn read_frame(stream: &mut UnixStream) -> Result<Vec<u8>, String> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).map_err(|e| e.to_string())?;
    let size = u32::from_be_bytes(header) as usize;
    if size > MAX_FRAME {
        return Err("Collector frame exceeds limit".into());
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}
fn write_frame(stream: &mut UnixStream, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_FRAME {
        return Err("Collector response exceeds limit".into());
    }
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| stream.write_all(bytes))
        .map_err(|e| e.to_string())
}
#[derive(Clone)]
pub struct Client {
    path: PathBuf,
}
impl Client {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn default_path() -> Self {
        Self::new(state_root().join("collector.sock"))
    }
    pub fn request(&self, body: Command) -> Result<Reply, String> {
        let mut stream =
            UnixStream::connect(&self.path).map_err(|_| "Collector unavailable; reconnecting")?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        write_frame(
            &mut stream,
            &serde_json::to_vec(&Request {
                version: PROTOCOL_VERSION,
                body,
            })
            .map_err(|e| e.to_string())?,
        )?;
        let response: Response = serde_json::from_slice(&read_frame(&mut stream)?)
            .map_err(|_| "Invalid collector response")?;
        if response.version != PROTOCOL_VERSION {
            return Err("Collector version incompatible".into());
        }
        Ok(response.body)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
/// Revision changes only with visible data, so idle hosts don't deserialize unchanged feeds.
pub(crate) fn update(snapshot: &RwLock<Snapshot>, mutate: impl FnOnce(&mut Snapshot)) {
    if let Ok(mut current) = snapshot.write() {
        let before = current.clone();
        mutate(&mut current);
        if *current != before {
            current.revision = before.revision.saturating_add(1);
        }
    }
}
