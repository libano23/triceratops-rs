//! The window's side of the MCP connection: requests arrive on socket threads, wait in a
//! queue until the GUI thread calls [`Host::serve`] between frames, and are answered
//! from there. Captures are answered once a frame has rendered them.

use std::io;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::app::App;
use crate::mcp::handler::{handle, take_capture, Outcome};
use crate::mcp::protocol::{Request, Response};
use crate::mcp::socket::{socket_dir, Listener};

type Incoming = (Request, Sender<Response>);

pub struct Host {
    listener: Listener,
    queue: Receiver<Incoming>,
    /// (capture id, request id, reply channel) waiting for a rendered frame.
    pending: Vec<(u64, u64, Sender<Response>)>,
}

impl Host {
    /// Listen for bridges; `wake` is called from socket threads whenever a request is
    /// queued, so the event loop can come round and [`serve`](Self::serve) it.
    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        Self::start_in(&socket_dir(), wake)
    }

    pub fn start_in(dir: &Path, wake: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        let (tx, queue) = mpsc::channel::<Incoming>();
        let tx = std::sync::Mutex::new(tx);
        let listener = Listener::start_in(dir, move |req, reply| {
            if tx.lock().map(|t| t.send((req, reply))).is_ok() {
                wake();
            }
        })?;
        Ok(Self { listener, queue, pending: Vec::new() })
    }

    pub fn name(&self) -> &str {
        self.listener.name()
    }

    /// A bridge is attached.
    pub fn connected(&self) -> bool {
        self.listener.connected_clients() > 0
    }

    /// A capture was requested and the next frame must render it.
    pub fn waiting_for_frame(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Run every queued request. Returns true when anything ran (the window should redraw).
    pub fn serve(&mut self, app: &mut App) -> bool {
        let mut any = false;
        while let Ok((req, reply)) = self.queue.try_recv() {
            any = true;
            match handle(app, &req.method, &req.params) {
                Outcome::Reply(r) => {
                    let _ = reply.send(match r {
                        Ok(v) => Response::ok(req.id, v),
                        Err(e) => Response::err(req.id, e),
                    });
                }
                Outcome::Deferred(capture) => self.pending.push((capture, req.id, reply)),
            }
        }
        any
    }

    /// Answer the captures a frame has finished.
    pub fn finish_captures(&mut self, app: &mut App) {
        self.pending.retain(|(capture, id, reply)| match take_capture(app, *capture) {
            Some(r) => {
                let _ = reply.send(match r {
                    Ok(v) => Response::ok(*id, v),
                    Err(e) => Response::err(*id, e),
                });
                false
            }
            None => true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::mcp::socket::Client;
    use serde_json::json;
    use std::time::Duration;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tri-host-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// Serve the host from this thread until `done` says so, as the GUI thread would.
    fn pump(host: &mut Host, app: &mut App, mut done: impl FnMut(&mut App) -> bool) {
        for _ in 0..500 {
            host.serve(app);
            host.finish_captures(app);
            if done(app) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out pumping the host");
    }

    #[test]
    fn requests_run_on_the_serving_thread() {
        let mut app = App::new();
        let mut host = Host::start_in(&temp_dir("run"), || {}).unwrap();
        let name = host.name().to_string();
        let caller = std::thread::spawn(move || {
            let mut c = Client::connect(&name).unwrap();
            c.call("run_commands", json!({ "lines": ["Sphere", "0,0,0", "5"] }), Duration::from_secs(10)).unwrap()
        });
        pump(&mut host, &mut app, |a| a.doc.objects().count() == 1);
        let r = caller.join().unwrap();
        assert_eq!(r["added"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn capture_is_answered_after_a_frame() {
        let mut app = App::new();
        let mut host = Host::start_in(&temp_dir("capture"), || {}).unwrap();
        let name = host.name().to_string();
        let caller = std::thread::spawn(move || {
            let mut c = Client::connect(&name).unwrap();
            c.call("capture_view", json!({}), Duration::from_secs(10)).unwrap()
        });
        pump(&mut host, &mut app, |a| !a.capture_requests.is_empty());
        assert!(host.waiting_for_frame());
        // what the event loop does after rendering
        let req = app.capture_requests.remove(0);
        app.finish_capture(&req, 8, 4, vec![200; 8 * 4 * 4]);
        pump(&mut host, &mut app, |_| true);
        assert!(!host.waiting_for_frame());
        let r = caller.join().unwrap();
        assert_eq!(r["width"], 8);
    }

    #[test]
    fn wake_is_called_for_each_request() {
        let woken = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let w = woken.clone();
        let mut app = App::new();
        let mut host = Host::start_in(&temp_dir("wake"), move || {
            w.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
        .unwrap();
        let name = host.name().to_string();
        let caller = std::thread::spawn(move || {
            let mut c = Client::connect(&name).unwrap();
            c.call("get_state", json!({}), Duration::from_secs(10)).unwrap()
        });
        pump(&mut host, &mut app, |_| woken.load(std::sync::atomic::Ordering::SeqCst) >= 2);
        caller.join().unwrap();
        assert!(woken.load(std::sync::atomic::Ordering::SeqCst) >= 2, "hello and get_state");
    }
}
