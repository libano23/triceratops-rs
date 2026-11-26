//! The window's side of the MCP connection: requests arrive on socket threads, wait in a
//! queue until the GUI thread calls [`Host::serve`] between frames, and are answered
//! from there. Captures are answered once a frame has rendered them.

use std::io;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::app::App;
use crate::mcp::handler::{busy_json, finish_run, handle, take_capture, Outcome, RunState};
use crate::mcp::protocol::{Request, Response};
use crate::mcp::socket::{socket_dir, Listener};

type Incoming = (Request, Sender<Response>);

pub struct Host {
    /// How long a capture may wait for a rendered frame (a minimized window renders none).
    pub capture_deadline: Duration,
    listener: Listener,
    queue: Receiver<Incoming>,
    /// (capture id, request id, reply channel, when asked) waiting for a rendered frame.
    pending: Vec<(u64, u64, Sender<Response>, Instant)>,
    /// run_commands waiting for their job: (state, request id, reply, last progress frame).
    runs: Vec<(RunState, u64, Sender<Response>, Instant)>,
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
        Ok(Self { listener, queue, pending: Vec::new(), runs: Vec::new(), capture_deadline: Duration::from_secs(5) })
    }

    #[cfg(test)]
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
            if req.method == crate::mcp::protocol::TICK {
                // a socket thread waiting for a job: just come round and poll it (below)
                continue;
            }
            any = true;
            match handle(app, &req.method, &req.params) {
                Outcome::Reply(r) => {
                    let _ = reply.send(match r {
                        Ok(v) => Response::ok(req.id, v),
                        Err(e) => Response::err(req.id, e),
                    });
                }
                Outcome::Deferred(capture) => self.pending.push((capture, req.id, reply, Instant::now())),
                Outcome::Run(state) => self.runs.push((state, req.id, reply, Instant::now())),
            }
        }
        any |= app.poll_job();
        any |= app.locust.poll();
        self.finish_runs(app);
        any
    }

    /// Answer the runs whose job has ended; tell the others how far it got, once a second.
    pub fn finish_runs(&mut self, app: &mut App) {
        self.runs.retain_mut(|(state, id, reply, last)| match finish_run(app, state) {
            Some(v) => {
                let _ = reply.send(Response::ok(*id, v));
                false
            }
            None => {
                if last.elapsed() >= Duration::from_secs(1) {
                    *last = Instant::now();
                    // a bridge that has gone away: keep the run, the job still ends normally
                    let _ = reply.send(Response::progress(*id, busy_json(app)));
                }
                true
            }
        });
    }

    /// Answer the captures a frame has finished, and fail those no frame rendered in time
    /// (a minimized or hidden window), withdrawing them from the queue.
    pub fn finish_captures(&mut self, app: &mut App) {
        let deadline = self.capture_deadline;
        self.pending.retain(|(capture, id, reply, asked)| match take_capture(app, *capture) {
            Some(r) => {
                let _ = reply.send(match r {
                    Ok(v) => Response::ok(*id, v),
                    Err(e) => Response::err(*id, e),
                });
                false
            }
            None if asked.elapsed() > deadline => {
                app.capture_requests.retain(|c| c.memory != Some(*capture));
                let _ = reply.send(Response::err(*id, "the Triceratops window is minimized or hidden, so the view could not be captured; it needs to be visible"));
                false
            }
            None => true,
        });
        // captures nobody waits for any more
        let pending: Vec<u64> = self.pending.iter().map(|p| p.0).collect();
        app.finished_captures.retain(|(c, _)| pending.contains(c));
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
            let c = Client::connect(&name).unwrap();
            c.call("run_commands", json!({ "lines": ["Sphere", "0,0,0", "5"] }), Duration::from_secs(10)).unwrap()
        });
        pump(&mut host, &mut app, |a| a.doc.objects().count() == 1);
        let r = caller.join().unwrap();
        assert_eq!(r["added"].as_array().unwrap().len(), 1);
    }

    /// A run that starts a job is answered when the job ends; meanwhile progress frames
    /// keep the caller informed and `cancel` (another call on the same connection) stops it.
    #[test]
    fn run_waits_for_job_with_progress_and_cancel() {
        let mut app = App::new();
        crate::jobs::testing::heavy_spheres(&mut app);
        let mut host = Host::start_in(&temp_dir("job"), || {}).unwrap();
        let name = host.name().to_string();
        // connecting needs the host to answer hello, so it happens off this thread
        let connecting = std::thread::spawn(move || Client::connect(&name).unwrap());
        pump(&mut host, &mut app, |_| connecting.is_finished());
        let client = std::sync::Arc::new(connecting.join().unwrap());
        let progress = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let caller = {
            let (c, p) = (client.clone(), progress.clone());
            std::thread::spawn(move || {
                c.call_with_progress("run_commands", json!({ "lines": ["BooleanUnion", "#1,#2", ""] }), Duration::from_secs(10), &mut |_| {
                    p.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                })
            })
        };
        pump(&mut host, &mut app, |a| a.job_status().is_some());
        let seen = progress.clone();
        pump(&mut host, &mut app, move |_| seen.load(std::sync::atomic::Ordering::SeqCst) > 0);
        let canceller = {
            let c = client.clone();
            std::thread::spawn(move || c.call("cancel", json!({}), Duration::from_secs(10)))
        };
        pump(&mut host, &mut app, |a| {
            a.poll_job();
            a.job_status().is_none()
        });
        pump(&mut host, &mut app, |_| true);
        assert_eq!(canceller.join().unwrap().unwrap()["cancelled"], true);
        let r = caller.join().unwrap().unwrap();
        assert!(r["log"].to_string().contains("BooleanUnion cancelled"), "{r}");
    }

    #[test]
    fn capture_is_answered_after_a_frame() {
        let mut app = App::new();
        let mut host = Host::start_in(&temp_dir("capture"), || {}).unwrap();
        let name = host.name().to_string();
        let caller = std::thread::spawn(move || {
            let c = Client::connect(&name).unwrap();
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
            let c = Client::connect(&name).unwrap();
            c.call("get_state", json!({}), Duration::from_secs(10)).unwrap()
        });
        pump(&mut host, &mut app, |_| woken.load(std::sync::atomic::Ordering::SeqCst) >= 2);
        caller.join().unwrap();
        assert!(woken.load(std::sync::atomic::Ordering::SeqCst) >= 2, "hello and get_state");
    }

    /// Review: a capture that never renders (minimized window) froze the connection.
    #[test]
    fn unrendered_capture_fails_after_its_deadline() {
        let mut app = App::new();
        let mut host = Host::start_in(&temp_dir("deadline"), || {}).unwrap();
        host.capture_deadline = Duration::from_millis(200);
        let name = host.name().to_string();
        let caller = std::thread::spawn(move || {
            let c = Client::connect(&name).unwrap();
            c.call("capture_view", json!({}), Duration::from_secs(10))
        });
        pump(&mut host, &mut app, |a| !a.capture_requests.is_empty());
        // no frame ever renders it
        std::thread::sleep(Duration::from_millis(300));
        pump(&mut host, &mut app, |_| true);
        let e = caller.join().unwrap().unwrap_err();
        assert!(e.contains("minimized or hidden"), "{e}");
        assert!(!host.waiting_for_frame(), "no endless redraws");
        assert!(app.capture_requests.iter().all(|c| c.memory.is_none()), "the queued capture is withdrawn");
    }
}
