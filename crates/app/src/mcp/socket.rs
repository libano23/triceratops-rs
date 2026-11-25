//! The local socket between `triceratops mcp` and running windows.
//!
//! Each window listens on a Unix socket (Linux, macOS) or a named pipe (Windows) and
//! registers itself with a small JSON file in [`socket_dir`], so the bridge can find the
//! most recently started live window. Nothing listens on a network interface.

use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use interprocess::local_socket::{prelude::*, ListenerNonblockingMode, ListenerOptions, Name, Stream};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::mcp::protocol::{Request, Response, VERSION};

/// Where running windows register: `$XDG_RUNTIME_DIR/triceratops` when there is one,
/// otherwise a per-user folder in the temporary directory.
pub fn socket_dir() -> PathBuf {
    #[cfg(unix)]
    if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d).join("triceratops");
    }
    std::env::temp_dir().join(format!("triceratops-{}", user_name()))
}

fn user_name() -> String {
    let u = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "user".into());
    u.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect()
}

/// Several listeners can live in one process (tests), so the tag adds a sequence number.
static SEQ: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
fn socket_name(dir: &Path, tag: &str) -> String {
    dir.join(format!("{tag}.sock")).to_string_lossy().into_owned()
}

#[cfg(windows)]
fn socket_name(_dir: &Path, tag: &str) -> String {
    format!("triceratops-{}-{tag}", user_name())
}

#[cfg(unix)]
fn to_name(s: &str) -> io::Result<Name<'static>> {
    s.to_string().to_fs_name::<interprocess::local_socket::GenericFilePath>()
}

#[cfg(windows)]
fn to_name(s: &str) -> io::Result<Name<'static>> {
    s.to_string().to_ns_name::<interprocess::local_socket::GenericNamespaced>()
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) {}

/// One registry file per running window.
#[derive(Serialize, Deserialize)]
struct Entry {
    name: String,
    pid: u32,
    started_ms: u128,
    seq: u64,
}

/// The app side: accepts connections and hands each request to `on_request` together
/// with the channel its reply goes back on. Dropping it unregisters the window and closes
/// connections as they next send something.
pub struct Listener {
    name: String,
    entry: PathBuf,
    stop: Arc<AtomicBool>,
    clients: Arc<AtomicUsize>,
}

impl Listener {
    pub fn start_in(dir: &Path, on_request: impl Fn(Request, Sender<Response>) + Send + Sync + 'static) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        restrict(dir, 0o700);
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let tag = format!("{}-{seq}", std::process::id());
        let name = socket_name(dir, &tag);
        #[cfg(unix)]
        let _ = std::fs::remove_file(&name);
        let listener = ListenerOptions::new().name(to_name(&name)?).nonblocking(ListenerNonblockingMode::Accept).create_sync()?;
        #[cfg(unix)]
        restrict(Path::new(&name), 0o600);
        let started_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let entry = dir.join(format!("{tag}.json"));
        std::fs::write(&entry, serde_json::to_vec(&Entry { name: name.clone(), pid: std::process::id(), started_ms, seq })?)?;
        restrict(&entry, 0o600);

        let stop = Arc::new(AtomicBool::new(false));
        let clients = Arc::new(AtomicUsize::new(0));
        let on_request = Arc::new(on_request);
        {
            let (stop, clients) = (stop.clone(), clients.clone());
            std::thread::Builder::new().name("mcp-accept".into()).spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok(stream) => {
                            let (stop, clients, on_request) = (stop.clone(), clients.clone(), on_request.clone());
                            let _ = std::thread::Builder::new().name("mcp-conn".into()).spawn(move || serve(stream, &stop, &clients, &*on_request));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(40)),
                    }
                }
            })?;
        }
        Ok(Self { name, entry, stop, clients })
    }

    /// The socket name a [`Client`] connects to.
    #[cfg(test)]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// How many bridges are attached right now.
    pub fn connected_clients(&self) -> usize {
        self.clients.load(Ordering::SeqCst)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::fs::remove_file(&self.entry);
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.name);
    }
}

fn serve(stream: Stream, stop: &AtomicBool, clients: &AtomicUsize, on_request: &(dyn Fn(Request, Sender<Response>) + Send + Sync)) {
    clients.fetch_add(1, Ordering::SeqCst);
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let reply = match serde_json::from_str::<Request>(line.trim()) {
            // the bridge is going away: closing our end also ends its reader thread
            Ok(req) if req.method == BYE => break,
            Ok(req) => {
                let id = req.id;
                let (tx, rx) = mpsc::channel();
                on_request(req, tx);
                rx.recv().unwrap_or_else(|_| Response::err(id, "Triceratops did not answer"))
            }
            Err(e) => Response::err(0, format!("bad request: {e}")),
        };
        if write_line(&stream, &reply).is_err() {
            break;
        }
    }
    clients.fetch_sub(1, Ordering::SeqCst);
}

fn write_line<T: Serialize>(mut w: impl Write, v: &T) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(v)?;
    bytes.push(b'\n');
    w.write_all(&bytes)?;
    w.flush()
}

/// Windows registered in `dir` that still answer, most recently started first. Entries
/// nobody listens on any more (a crashed window) are deleted.
pub fn live_sockets_in(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut entries: Vec<(PathBuf, Entry)> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Entry>(&b).ok()).map(|e| (p, e)))
        .collect();
    entries.sort_by_key(|(_, e)| std::cmp::Reverse((e.started_ms, e.seq)));
    let mut live = Vec::new();
    for (path, e) in entries {
        if to_name(&e.name).and_then(Stream::connect).is_ok() {
            live.push(e.name);
        } else {
            let _ = std::fs::remove_file(&path);
            #[cfg(unix)]
            let _ = std::fs::remove_file(&e.name);
        }
    }
    live
}

/// Sent by a [`Client`] that is dropped, so the window closes the connection.
const BYE: &str = "bye";

/// The bridge side of one connection to a window.
pub struct Client {
    stream: Arc<Stream>,
    replies: Receiver<Response>,
    next_id: u64,
}

impl Client {
    /// Connect and check that the window runs the same version as this bridge.
    pub fn connect(name: &str) -> io::Result<Client> {
        let stream = Arc::new(Stream::connect(to_name(name)?)?);
        let (tx, replies) = mpsc::channel();
        let incoming = stream.clone();
        std::thread::Builder::new().name("mcp-replies".into()).spawn(move || {
            let mut reader = BufReader::new(&*incoming);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Ok(r) = serde_json::from_str::<Response>(line.trim()) {
                            if tx.send(r).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        })?;
        let mut client = Client { stream, replies, next_id: 1 };
        let hello = client.call("hello", json!({ "version": VERSION }), Duration::from_secs(10)).map_err(io::Error::other)?;
        let theirs = hello["version"].as_str().unwrap_or("unknown");
        if theirs != VERSION {
            return Err(io::Error::other(format!(
                "the running Triceratops is version {theirs} but this `triceratops mcp` is version {VERSION}; start the assistant with the same Triceratops you are running"
            )));
        }
        Ok(client)
    }

    /// Send one request and wait for its reply.
    pub fn call(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        if write_line(&*self.stream, &Request { id, method: method.into(), params }).is_err() {
            return Err("Triceratops was closed".into());
        }
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.replies.recv_timeout(left) {
                Ok(r) if r.id == id => return r.into_result(),
                Ok(_) => continue, // a late reply to an earlier call that timed out
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("timed out after {} s; the command is still running in Triceratops (Esc cancels it)", timeout.as_secs()));
                }
                Err(RecvTimeoutError::Disconnected) => return Err("Triceratops was closed".into()),
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = write_line(&*self.stream, &Request { id: 0, method: BYE.into(), params: Value::Null });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::protocol::{Request, Response};
    use serde_json::json;
    use std::sync::mpsc::Sender;
    use std::time::Duration;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tri-sock-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// Answers hello with `version` and echoes every other request's params.
    fn echo(version: &'static str) -> impl Fn(Request, Sender<Response>) + Send + Sync + 'static {
        move |req, tx| {
            let result = if req.method == "hello" { json!({ "version": version }) } else { req.params };
            let _ = tx.send(Response::ok(req.id, result));
        }
    }

    const T: Duration = Duration::from_secs(5);

    #[test]
    fn roundtrip_echo() {
        let dir = temp_dir("echo");
        let l = Listener::start_in(&dir, echo(crate::mcp::protocol::VERSION)).unwrap();
        let mut c = Client::connect(l.name()).unwrap();
        assert_eq!(c.call("run_commands", json!({ "x": 1 }), T).unwrap(), json!({ "x": 1 }));
        assert_eq!(c.call("get_state", json!({ "y": 2 }), T).unwrap(), json!({ "y": 2 }));
    }

    #[test]
    fn version_mismatch_is_refused() {
        let dir = temp_dir("version");
        let l = Listener::start_in(&dir, echo("0.0.0")).unwrap();
        let err = Client::connect(l.name()).err().expect("refused").to_string();
        assert!(err.contains("0.0.0") && err.contains(crate::mcp::protocol::VERSION), "{err}");
    }

    #[test]
    fn stale_entry_is_removed() {
        let dir = temp_dir("stale");
        let stale = Listener::start_in(&dir, echo(crate::mcp::protocol::VERSION)).unwrap();
        let stale_name = stale.name().to_string();
        // a crashed window: its registry entry stays behind but nobody listens any more
        let entry = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "json")).unwrap();
        let kept = std::fs::read(&entry).unwrap();
        drop(stale);
        std::fs::write(&entry, kept).unwrap();
        let live = Listener::start_in(&dir, echo(crate::mcp::protocol::VERSION)).unwrap();
        let found = live_sockets_in(&dir);
        assert_eq!(found, vec![live.name().to_string()]);
        assert!(!found.contains(&stale_name));
        assert!(!entry.exists(), "stale entry deleted");
    }

    #[test]
    fn newest_live_window_first() {
        let dir = temp_dir("newest");
        let a = Listener::start_in(&dir, echo(crate::mcp::protocol::VERSION)).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let b = Listener::start_in(&dir, echo(crate::mcp::protocol::VERSION)).unwrap();
        assert_eq!(live_sockets_in(&dir), vec![b.name().to_string(), a.name().to_string()]);
    }

    #[test]
    fn closed_window_is_reported() {
        let dir = temp_dir("closed");
        let l = Listener::start_in(&dir, echo(crate::mcp::protocol::VERSION)).unwrap();
        let mut c = Client::connect(l.name()).unwrap();
        drop(l);
        assert_eq!(c.call("get_state", json!({}), T).unwrap_err(), "Triceratops was closed");
    }

    #[test]
    fn slow_reply_times_out() {
        let dir = temp_dir("slow");
        let l = Listener::start_in(&dir, |req: Request, tx: Sender<Response>| {
            if req.method == "hello" {
                let _ = tx.send(Response::ok(req.id, json!({ "version": crate::mcp::protocol::VERSION })));
            } else {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(400));
                    let _ = tx.send(Response::ok(req.id, json!("late")));
                });
            }
        })
        .unwrap();
        let mut c = Client::connect(l.name()).unwrap();
        let e = c.call("run_commands", json!({}), Duration::from_millis(50)).unwrap_err();
        assert!(e.starts_with("timed out"), "{e}");
        // the late reply to the first call is skipped, the next call gets its own
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(c.call("run_commands", json!({}), Duration::from_secs(2)).unwrap(), json!("late"));
    }

    #[test]
    fn counts_connected_clients() {
        let dir = temp_dir("count");
        let l = Listener::start_in(&dir, echo(crate::mcp::protocol::VERSION)).unwrap();
        assert_eq!(l.connected_clients(), 0);
        let c = Client::connect(l.name()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(l.connected_clients(), 1);
        drop(c);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(l.connected_clients(), 0);
    }
}
