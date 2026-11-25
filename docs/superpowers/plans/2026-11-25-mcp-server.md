# MCP Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let MCP clients (Claude, Codex, …) model in the open Triceratops window through `triceratops mcp`, a stdio MCP bridge that drives the running app over a local socket.

**Architecture:** The app starts a local-socket listener; requests are handed to the GUI thread through a channel plus a winit user event, executed against `App` between frames by a UI-free handler, and answered as JSON. The same executable run as `triceratops mcp` is an `rmcp` stdio server whose tools forward to that socket, launching the app if none is running.

**Tech Stack:** Rust 2021, winit 0.30 (`EventLoop::with_user_event`), `rmcp` 3.x (features `server`, `transport-io`, `macros`, `schemars`), `tokio` (rt-multi-thread, macros, io-std), `interprocess` 2.x, `serde_json`, `schemars`.

**Spec:** `docs/superpowers/specs/2026-11-25-mcp-server-design.md`

## Global Constraints

- Local only: Unix socket `$XDG_RUNTIME_DIR/triceratops/<pid>.sock` (fallback: `std::env::temp_dir()/triceratops-<uid>/<pid>.sock`), mode 0600; Windows named pipe `triceratops-<user>-<pid>`. Never bind a network interface.
- Internal protocol: newline-delimited JSON, `{"id":u64,"method":str,"params":obj}` → `{"id":u64,"result":obj}` | `{"id":u64,"error":str}`; first exchange `hello` with `{"version": env!("CARGO_PKG_VERSION")}`; mismatched versions refuse with a message naming both.
- Bridge: `triceratops mcp`; most recently started live window wins; `TRICERATOPS_SOCKET` overrides; if none, spawn `std::env::current_exe()` (no args) and wait up to 20 s.
- Request timeout 60 s (bridge side), reply `"timed out after 60 s; the command is still running in Triceratops (Esc cancels it)"`.
- Assistant calls never open modal UI: Loft uses defaults; file pickers fail with `"<Command> needs a file path when run by an assistant"`; message boxes become log lines.
- Options switch `allow_assistants` (on by default) in `options.json`; when off, no socket is created. Status bar shows `Assistant connected` while a bridge is attached.
- Every crate addition must build on Linux, Windows (MSVC) and macOS; CI stays `clippy -D warnings` + tests on all three.
- Version for the release: `0.2.0`.

## Review Focus

1. The user has a command half-typed (or running) when a request arrives → the request fails with `"the user is running a command in Triceratops"`; nothing is fed into the user's command. (Task 5)
2. A `run_commands` call ends inside a command still waiting for input → the reply carries that prompt and its options; the next `run_commands` continues it; the user can Esc it. (Task 2)
3. The Triceratops window is closed while the bridge is attached → the current call reports `"Triceratops was closed"`; the next call reconnects or relaunches. (Task 6)
4. Bad input from the assistant (unknown command, malformed point, `#999` missing object) → the reply contains the app's own log lines (`Unknown command: …`, `No object matches #999`) and success, never a protocol error or a stuck prompt. (Task 2)
5. A stale socket from a crashed app, or two windows open → stale entries are removed, the newest live one is chosen. (Task 4)

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/app/src/app.rs` (modify) | `run_script_lines`, assistant mode (dialog guards, Loft defaults, message box → log), capture to memory, `allow_assistants` option, `assistant_connected` flag |
| `crates/app/src/mcp/mod.rs` (create) | module root, re-exports |
| `crates/app/src/mcp/protocol.rs` (create) | `Request`, `Response`, method names, `PROTOCOL_HELLO` |
| `crates/app/src/mcp/handler.rs` (create) | UI-free execution of each method against `&mut App` |
| `crates/app/src/mcp/socket.rs` (create) | socket paths, registry of live windows, app-side listener thread, bridge-side client |
| `crates/app/src/mcp/bridge.rs` (create) | `rmcp` stdio server: tools, instructions, launching the app |
| `crates/app/src/main.rs` (modify) | `mcp` subcommand; user events; start listener; serve queued requests |
| `crates/app/src/ui/status.rs`, `crates/app/src/ui/panels.rs` (modify) | indicator; Options switch |
| `README.md`, `site/index.html` (modify) | set-up instructions |
| `.github/workflows/ci.yml` (modify) | MCP handshake smoke test on all OSes |

---

### Task 1: Script runner and assistant mode in `App`

**Files:**
- Modify: `crates/app/src/app.rs` (`run_command_file` at ~4056, `process_requests` LoftDialog/Message arms ~2964-2967, each `rfd::FileDialog` call site in `app.rs`)
- Test: `crates/app/src/app.rs` `#[cfg(test)] mod assistant_tests`

**Interfaces:**
- Produces:
  - `pub fn run_script_lines(&mut self, lines: &[String])` — the loop now inside `run_command_file` (`""`/`_enter` = Enter, `//` comments skipped); `run_command_file` calls it.
  - `pub assistant_mode: bool` field on `App` (false by default).
  - `fn file_dialog_blocked(&mut self, command: &str) -> bool` — when `assistant_mode`, logs `"{command} needs a file path when run by an assistant"` and returns true.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn script_lines_build_and_boolean() {
    let mut app = App::new();
    app.run_script_lines(&lines(&["Box", "0,0,0", "10,10,0", "5", "Cylinder", "5,5,-1", "2", "7", "BooleanDifference", "#1", "#2"]));
    assert_eq!(app.doc.objects().count(), 1);
    assert!(!app.runner.is_active());
}
#[test] fn assistant_loft_uses_defaults_without_dialog() {
    // two InterpCrv curves, then Loft "#1,#2" with assistant_mode = true
    // assert app.loft_dialog.is_none() and a surface/polysurface was added
}
#[test] fn assistant_open_without_path_fails_instead_of_dialog() {
    // assistant_mode = true; run "Open" → log contains "Open needs a file path when run by an assistant"; no panic, runner idle
}
#[test] fn assistant_message_box_goes_to_log() {
    // push AppRequest::Message("hi") with assistant_mode → message_box is None, log ends with "hi"
}
```
(`lines(&[&str]) -> Vec<String>` is a test helper.)

- [ ] **Step 2: Run, expect FAIL** — `cargo test -p triceratops assistant_tests` → `run_script_lines` / `assistant_mode` not found.
- [ ] **Step 3: Implement** — move the loop; in the `LoftDialog` arm, when `assistant_mode`, build the `LoftDialog` with defaults and call `finish_loft(true)` immediately; in the `Message` arm, log instead of `message_box`; guard every `rfd::FileDialog` in `app.rs` with `if self.file_dialog_blocked("<CommandName>") { return; }` (or the arm's equivalent early exit).
- [ ] **Step 4: Run** — `cargo test -p triceratops` → all pass; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] **Step 5: Commit** — `git commit -m "App: script runner and assistant mode without modal dialogs"`

---

### Task 2: Protocol types and the UI-free handler

**Files:**
- Create: `crates/app/src/mcp/mod.rs`, `crates/app/src/mcp/protocol.rs`, `crates/app/src/mcp/handler.rs`
- Modify: `crates/app/src/main.rs` (`mod mcp;`)
- Test: `crates/app/src/mcp/handler.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `App::run_script_lines`, `App::assistant_mode` (Task 1); `commands::lookup(&str, usize)`; `Document::{objects, object_bbox, area_of, volume_of, layers}`.
- Produces:
  - `protocol::Request { id: u64, method: String, params: serde_json::Value }`, `protocol::Response { id: u64, result: Option<Value>, error: Option<String> }` (serde, newline-delimited).
  - `pub const METHODS: &[&str] = &["hello", "run_commands", "get_state", "list_objects", "object_info", "find_commands", "capture_view", "undo", "redo"]`.
  - `handler::Outcome { Reply(Result<Value, String>), Deferred(u64) }` (Deferred = waiting on a capture id, Task 3).
  - `pub fn handle(app: &mut App, method: &str, params: &Value) -> Outcome`.
- Reply shapes (JSON):
  - `run_commands {lines: [str]}` → `{log: [str], added: [u64], changed: [u64], deleted: [u64], prompt: null | {message, kind, options: [{name, value?}], default?}}`. `changed` = ids present before and after whose `serde_json` of geometry+attributes hash differs. Sets `assistant_mode` for the duration.
  - `get_state {}` → `{prompt, selected: [u64], active_viewport: str, units: str, layers: [{name, visible, locked, current}]}`
  - `list_objects {type?, layer?, ids?, limit? = 200}` → `{objects: [{id, type, layer, name, bbox: [[x,y,z],[x,y,z]], closed}], total}`
  - `object_info {id}` → `{id, type, layer, name, bbox, area?, volume?, centroid?}` or error `"no object #<id>"`
  - `find_commands {query, limit? = 20}` → `{commands: [{name, aliases, description}]}`
  - `undo {steps? = 1}` / `redo` → `{log: [str]}`

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn run_commands_reports_added_and_log() {
    let r = reply(handle(&mut App::new(), "run_commands", &json!({"lines": ["Sphere", "0,0,0", "5"]})));
    assert_eq!(r["added"].as_array().unwrap().len(), 1);
    assert!(r["prompt"].is_null());
}
#[test] fn run_commands_returns_pending_prompt_and_continues() {
    let mut app = App::new();
    let r = reply(handle(&mut app, "run_commands", &json!({"lines": ["Box", "0,0,0"]})));
    assert!(r["prompt"]["message"].as_str().unwrap().len() > 0);
    let r = reply(handle(&mut app, "run_commands", &json!({"lines": ["10,10,0", "5"]})));
    assert_eq!(r["added"].as_array().unwrap().len(), 1);
}
#[test] fn bad_input_is_logged_not_an_error() {   // Review Focus 4
    let r = reply(handle(&mut App::new(), "run_commands", &json!({"lines": ["Frobnicate", "Move", "#999"]})));
    let log = r["log"].to_string();
    assert!(log.contains("Unknown command: Frobnicate") && log.contains("No object matches #999"));
}
#[test] fn changed_and_deleted_are_detected() { /* Box; Move #1 0,0,0 0,0,5 → changed [1]; Delete #1 → deleted [1] */ }
#[test] fn list_objects_and_object_info() { /* Box 10×10×5: type "polysurface", closed true, volume ≈ 500 */ }
#[test] fn find_commands_matches_descriptions() { /* query "fillet" → contains "FilletEdge" */ }
#[test] fn undo_removes_last_object() { /* Sphere; undo → objects 0 */ }
```

- [ ] **Step 2: Run, expect FAIL** — `cargo test -p triceratops mcp::handler` → module missing.
- [ ] **Step 3: Implement** `protocol.rs` and `handler.rs`; `type` strings come from `Geometry::type_name()`.
- [ ] **Step 4: Run** — tests pass; clippy clean.
- [ ] **Step 5: Commit** — `git commit -m "MCP: protocol types and UI-free request handler"`

---

### Task 3: `capture_view` into memory

**Files:**
- Modify: `crates/app/src/app.rs` (`CaptureRequest` ~456, `finish_capture` ~3503), `crates/app/src/mcp/handler.rs`
- Test: `crates/app/src/mcp/handler.rs`

**Interfaces:**
- Produces:
  - `CaptureRequest.memory: Option<u64>` (capture id).
  - `App::finished_captures: Vec<(u64, Result<Vec<u8>, String>)>` — PNG bytes (resized to the requested `width`×`height` keeping aspect if given, default 1280 wide) filled by `finish_capture`.
  - `capture_view {viewport?: str (title, e.g. "Perspective"), width?, height?}` → `Outcome::Deferred(id)`; unknown viewport → `Reply(Err("no viewport named <x>; viewports: …"))`.
  - `pub fn take_capture(app: &mut App, id: u64) -> Option<Result<Value, String>>` → `{png_base64, width, height}`.

- [ ] **Step 1: Failing tests** — `capture_view` returns `Deferred`, pushes a `CaptureRequest` with `memory: Some(id)` and makes the named viewport active; calling `app.finish_capture(&req, 4, 2, vec![255; 32])` then `take_capture(&mut app, id)` yields a PNG whose decoded size is 4×2 (no resize requested in this test).
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** — `finish_capture` branches on `memory` before the file/clipboard path; encode with `image::codecs::png::PngEncoder`.
- [ ] **Step 4: Run, pass; clippy clean.**
- [ ] **Step 5: Commit** — `git commit -m "MCP: capture a viewport into memory"`

---

### Task 4: Local sockets: registry, listener, client

**Files:**
- Create: `crates/app/src/mcp/socket.rs`
- Modify: `crates/app/Cargo.toml` (`interprocess = "2"`)
- Test: `crates/app/src/mcp/socket.rs`

**Interfaces:**
- Consumes: `protocol::{Request, Response}` (Task 2).
- Produces:
  - `pub fn socket_dir() -> PathBuf`; `pub fn own_socket_name() -> String` (Unix: path in `socket_dir()`; Windows: pipe name).
  - `pub fn live_sockets() -> Vec<(u32 /*pid*/, String, SystemTime)>` newest first (by the socket file's modification time on Unix; on Windows by a small registry file `socket_dir()/<pid>.json` written at start). An entry is stale when connecting to it is refused; stale entries are deleted.
  - `pub struct Listener` with `pub fn start(on_request: impl Fn(Request, Sender<Response>) + Send + 'static) -> io::Result<Listener>`; one thread per connection; `Drop` removes the socket file. `pub fn connected_clients(&self) -> usize`.
  - `pub struct Client` with `pub fn connect(name: &str) -> io::Result<Client>` and `pub fn call(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String>`; performs `hello` on connect and refuses a version mismatch.

- [ ] **Step 1: Failing tests**

```rust
#[test] fn roundtrip_echo() { /* Listener echoing params; Client::call("run_commands", {"x":1}) == {"x":1} */ }
#[test] fn version_mismatch_is_refused() { /* listener answers hello with "0.0.0" → connect error mentions both versions */ }
#[test] fn stale_socket_is_skipped() {   // Review Focus 5
    // create a socket file with no listener in a temp socket_dir (override via TRICERATOPS_SOCKET_DIR for tests),
    // start a real Listener; live_sockets() returns only the live one and the stale file is gone
}
#[test] fn newest_live_window_first() { /* two listeners; the second started appears first */ }
```
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** with `interprocess::local_socket` (`GenericFilePath` on Unix, `GenericNamespaced` on Windows); set 0600 on Unix after bind. Support `TRICERATOPS_SOCKET_DIR` (tests) besides `TRICERATOPS_SOCKET` (explicit target).
- [ ] **Step 4: Run on Linux; push branch so CI runs Windows/macOS.**
- [ ] **Step 5: Commit** — `git commit -m "MCP: local-socket listener and client"`

---

### Task 5: Wire the listener into the running app

**Files:**
- Modify: `crates/app/src/main.rs`, `crates/app/src/app.rs` (options), `crates/app/src/ui/status.rs`, `crates/app/src/ui/panels.rs`
- Test: `crates/app/src/mcp/handler.rs` (guard), manual run

**Interfaces:**
- Consumes: `Listener::start`, `handler::{handle, take_capture, Outcome}`.
- Produces:
  - `enum UserEvent { Mcp }`; `EventLoop::<UserEvent>::with_user_event()`; `ApplicationHandler<UserEvent>::user_event` drains an `mpsc::Receiver<(Request, Sender<Response>)>`, calls `handle`, sends replies, keeps `Deferred` ones until `take_capture` returns `Some` after a frame, then `window.request_redraw()`.
  - `pub fn busy_with_user(app: &App) -> bool` in `handler.rs`: `!app.cmd_text.trim().is_empty() || (app.runner.is_active() && !app.assistant_owns_command)`; `App::assistant_owns_command: bool` set when a `run_commands` call leaves a prompt pending, cleared when the runner goes idle.
  - `App::allow_assistants: bool` persisted in options (default true); `App::assistant_connected: bool` mirrored from `Listener::connected_clients() > 0`.

- [ ] **Step 1: Failing test** (Review Focus 1)

```rust
#[test] fn request_refused_while_user_types() {
    let mut app = App::new();
    app.cmd_text = "Cir".into();
    assert_eq!(err(handle(&mut app, "run_commands", &json!({"lines": ["Sphere"]}))), "the user is running a command in Triceratops");
    assert_eq!(app.cmd_text, "Cir");
}
```
(`handle` checks `busy_with_user` for `run_commands`, `undo`, `redo`.)
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** the guard, the event-loop wiring, the listener start in `resumed` when `allow_assistants`, the status-bar label `Assistant connected`, and the Options → General checkbox `Allow AI assistants (MCP)`.
- [ ] **Step 4: Verify** — tests pass; run the app, then from a second shell `socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/triceratops/<pid>.sock` with a hello line and `{"id":1,"method":"run_commands","params":{"lines":["Sphere","0,0,0","5"]}}`: the sphere appears in the window and the reply lists it.
- [ ] **Step 5: Commit** — `git commit -m "App: serve assistant requests from the running window"`

---

### Task 6: The `triceratops mcp` bridge

**Files:**
- Create: `crates/app/src/mcp/bridge.rs`
- Modify: `crates/app/src/main.rs` (dispatch `mcp` before creating the event loop), `crates/app/Cargo.toml` (`rmcp`, `tokio`, `schemars`)
- Test: `crates/app/src/mcp/bridge.rs` (in-process, against a test `Listener`)

**Interfaces:**
- Consumes: `socket::{live_sockets, Client}`; method names and reply shapes from Task 2–3.
- Produces:
  - `pub fn run() -> anyhow::Result<()>` — tokio runtime, `rmcp` stdio server.
  - Tools (names, schemas via `schemars`, descriptions): `run_commands{lines}`, `get_state{}`, `list_objects{type?,layer?,ids?,limit?}`, `object_info{id}`, `find_commands{query,limit?}`, `capture_view{viewport?,width?,height?}` (returns MCP image content `image/png`), `undo{steps?}`, `redo{steps?}`.
  - Server instructions: the README "Scripts" rules, units, "check your work with list_objects and capture_view", "a pending prompt means the command waits for more input".
  - Connection policy: connect lazily on first call (`TRICERATOPS_SOCKET` or newest live socket); if none, spawn `current_exe()` and poll for 20 s; on a broken connection reply `"Triceratops was closed"` and drop the client so the next call reconnects/relaunches; 60 s per-call timeout.
- [ ] **Step 1: Failing tests** — with a test `Listener` backed by `handler::handle` on a real `App`: `tools/list` has the 8 tools; `run_commands` builds a sphere; after the listener is dropped the next call errors with `"Triceratops was closed"` and the following call reconnects to a new listener (Review Focus 3).
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** with `rmcp` (`#[tool_router]`/`#[tool]`, `ServerHandler::get_info` carrying the instructions, `serve(rmcp::transport::stdio())`); keep blocking socket I/O in `spawn_blocking`.
- [ ] **Step 4: Verify** — `cargo test -p triceratops mcp::bridge`; then `printf` an `initialize` + `tools/list` JSON-RPC pair into `target/release/triceratops mcp` and see 8 tools.
- [ ] **Step 5: Commit** — `git commit -m "MCP: stdio bridge (triceratops mcp)"`

---

### Task 7: Docs, CI smoke test, version

**Files:**
- Modify: `README.md` (new section "Model with an AI assistant (MCP)"), `site/index.html` (feature card + short section), `.github/workflows/ci.yml`, `Cargo.toml` (`version = "0.2.0"`)

- [ ] **Step 1: CI smoke test** — new step on all three OSes after tests: build, then run a small script (bash / pwsh) that pipes `initialize`, `notifications/initialized`, `tools/list` into `target/debug/triceratops mcp` with `TRICERATOPS_SOCKET=none-for-ci` so it does not launch a window, and asserts the reply lists `run_commands`. This also proves the Windows GUI-subsystem binary reads stdin; if it does not, add `[[bin]] triceratops-mcp` (console subsystem) calling `mcp::bridge::run` and document that path for Windows.
- [ ] **Step 2: README/site** — set-up for Claude Code (`claude mcp add triceratops -- <exe> mcp`), Claude Desktop (`claude_desktop_config.json` snippet), Codex (`codex mcp add triceratops -- <exe> mcp` and `config.toml` snippet); per-OS executable paths (AppImage path, `C:\…\triceratops.exe`, `/Applications/Triceratops.app/Contents/MacOS/triceratops`); a three-line example prompt.
- [ ] **Step 3: Verify** — CI green on the branch.
- [ ] **Step 4: Commit** — `git commit -m "MCP: docs, CI smoke test, version 0.2.0"`

---

### Task 8: Acceptance — model something through MCP

- [ ] **Step 1:** Start Triceratops (release build) in a private Xwayland display as in `site/tools/shoot.sh`, with `DISPLAY` pointed there so the bridge finds its socket.
- [ ] **Step 2:** As an MCP client (Claude Code: `claude mcp add triceratops -- target/release/triceratops mcp`, or an `rmcp`/JSON-RPC client script if the session cannot load new servers), model from the prompt *"a mounting bracket: 60×40×8 mm base plate with two 6 mm through holes 40 mm apart, and a 40×8×30 mm upright flange on one long edge, fillet-free"* using `find_commands`, `run_commands`, `list_objects`, `capture_view`.
- [ ] **Step 3:** Check: one closed solid; volume within 2 % of 60·40·8 + 40·8·30 − 2·π·3²·8 = 28,347 mm³; a capture shows the holes.
- [ ] **Step 4:** Save screenshots of the window after each stage and share them with the user; note anything the assistant found hard (missing tool, confusing reply) as follow-ups.
