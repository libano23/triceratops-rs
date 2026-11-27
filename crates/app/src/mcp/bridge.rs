//! `triceratops mcp`: an MCP server on stdin/stdout that forwards each tool call to the
//! running Triceratops window over the local socket, starting a window when none runs.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ErrorData, Implementation, ProgressNotificationParam, ServerCapabilities, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, RoleServer, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::socket::{live_sockets_in, socket_dir, Client};

/// How long a tool call may go without any word from the window (a long job keeps sending
/// progress, so it may take as long as it needs).
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long to wait for a window started by the bridge to appear.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(20);

const INSTRUCTIONS: &str = "Triceratops is a NURBS 3D modeller. You drive the window the user has open; they watch \
every change and can undo it.\n\
\n\
Two ways to model, and the user chooses: direct commands (run_commands) make fixed geometry; a Locust definition \
(locust_set_graph) makes a live, parametric model the user can adjust with sliders in Locust, Triceratops' node \
editor. Model the way the user asks. If they want something adjustable or parametric, use Locust; if they have not \
said and it matters, ask them briefly. The two mix: locust_bake turns a definition's result into ordinary objects.\n\
\n\
Model with run_commands: each line is what a person would type on Triceratops' command line.\n\
- A command name starts a command (Box, Cylinder, Sphere, Line, Polyline, InterpCrv, ExtrudeCrv, Revolve, Loft, \
BooleanUnion, BooleanDifference, Move, Copy, Rotate, Mirror, FilletEdge, Delete, Undo, ...). Use find_commands to \
discover more by what they do.\n\
- Then answer its prompts in order: points as x,y,z (document units, usually millimetres), numbers, option names, \
or objects as #id, #id1,#id2 or last (the objects the last command made). #id@x,y,z picks an object at a point. \
Every command you start begins with nothing selected, so always name the objects it works on.\n\
- An empty string \"\" presses Enter: it finishes point lists (Polyline, InterpCrv) and accepts defaults. \
\"_Cancel\" presses Esc: it abandons the command that is waiting.\n\
- The reply lists the log, the ids added, changed and deleted, and the prompt still waiting, if any. A waiting \
prompt means the command wants more input: continue it with another run_commands call, or send \"_Cancel\".\n\
\n\
Check your work with list_objects, object_info (volume, area) and capture_view, which returns an image of a \
viewport (Perspective, Top, Front, Right). Commands that need a file (Open, Save, Export) are not available. \
If the user is typing or running a command of their own, requests are refused: wait and try again.\n\
\n\
Locust: find components with locust_components (inputs, outputs, defaults), send a whole definition with \
locust_set_graph, read the report (errors per node, panels, a summary of what is previewed or baked), adjust sliders \
by name with locust_set_values, and look at the preview with capture_view. A definition is JSON: \
{\"nodes\": [{\"type\": \"Number\", \"nickname\": \"Radius\", \"values\": {\"Value\": 6}}, {\"type\": \"Circle\"}, \
{\"type\": \"Extrude\", \"values\": {\"Distance\": 12}}, {\"type\": \"Preview\"}, {\"type\": \"Bake\"}], \
\"connections\": [{\"from\": \"Radius\", \"to\": \"Circle.Radius\"}, {\"from\": \"Circle\", \"to\": \"Extrude.Curve\"}, \
{\"from\": \"Extrude\", \"to\": \"Preview.Geometry\"}, {\"from\": \"Extrude\", \"to\": \"Bake.Geometry\"}]}. \
Nodes are named by nickname (or type when unique), ports by name; \"from\": \"Node\" means its first output. \
Unwired inputs take \"values\" by input name. Positions are optional. Give sliders (Number, Integer) nicknames \
so they are easy to set later.\n\
\n\
Long operations (Booleans above all) run in the background while the window stays responsive: run_commands \
answers when they end and sends progress notifications meanwhile; get_state shows what is running under \"busy\", \
and cancel stops it (the user can also press Esc). While one runs, other requests that change the model are refused.";

/// Which window to talk to.
pub enum Target {
    /// A socket named explicitly (`TRICERATOPS_SOCKET`).
    Socket(String),
    /// The newest live window registered in `dir`; if there is none and `launch` is set,
    /// start that executable and wait for it.
    Discover { dir: PathBuf, launch: Option<PathBuf> },
}

#[derive(Clone)]
pub struct Bridge {
    target: Arc<Target>,
    conn: Arc<Mutex<Option<Arc<Client>>>>,
    /// A window has answered in this session: once it is gone (closed, or assistants
    /// switched off), never start another one on our own.
    connected_once: Arc<std::sync::atomic::AtomicBool>,
    #[allow(dead_code)] // read by the code #[tool_handler] generates
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RunCommandsParams {
    /// Command-line entries, one per element, run in order. "" presses Enter.
    pub lines: Vec<String>,
}

#[derive(Default, Deserialize, JsonSchema)]
pub struct ListObjectsParams {
    /// Only objects whose type contains this text, e.g. "curve", "surface", "polysurface", "mesh", "point".
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Only objects on this layer.
    pub layer: Option<String>,
    /// Only these object ids.
    pub ids: Option<Vec<u64>>,
    /// At most this many objects (default 200); "total" still counts all matches.
    pub limit: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ObjectInfoParams {
    /// The object id (as in #12).
    pub id: u64,
}

#[derive(Deserialize, JsonSchema)]
pub struct FindCommandsParams {
    /// Words to look for in command names and descriptions, e.g. "fillet edge" or "sweep".
    pub query: String,
    /// At most this many commands (default 20).
    pub limit: Option<u64>,
}

#[derive(Default, Deserialize, JsonSchema)]
pub struct CaptureViewParams {
    /// Viewport title: "Perspective", "Top", "Front", "Right" (default: the active one).
    pub viewport: Option<String>,
    /// Maximum width in pixels (default 1280).
    pub width: Option<u32>,
    /// Maximum height in pixels (default 1280).
    pub height: Option<u32>,
}

#[derive(Default, Deserialize, JsonSchema)]
pub struct LocustComponentsParams {
    /// Words that must all appear in the name, category or description.
    pub query: Option<String>,
    /// Only this category (Params, Math, Sets, Vector, Curve, Surface, Solid, Mesh, Transform, Analysis, Output, ...).
    pub category: Option<String>,
    /// At most this many (default 40).
    pub limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct LocustSetGraphParams {
    /// The definition: {"nodes": [{"type", "nickname"?, "values"?, "x"?, "y"?}], "connections": [{"from": "Node[.Output]", "to": "Node.Input"}], "groups"?, "comments"?}.
    pub graph: Value,
}

#[derive(Deserialize, JsonSchema)]
pub struct LocustSetValuesParams {
    /// Input values by "Node" (its first input, e.g. a Number slider) or "Node.Input": {"Radius": 3, "Extrude.Cap": false}; also "Node.Expression" (an Expression's formula) and "Node.code" (a Script's code).
    pub values: serde_json::Map<String, Value>,
}

#[derive(Default, Deserialize, JsonSchema)]
pub struct StepsParams {
    /// How many steps (default 1).
    pub steps: Option<u32>,
}

impl Bridge {
    pub fn new(target: Target) -> Self {
        Self { target: Arc::new(target), conn: Arc::new(Mutex::new(None)), connected_once: Default::default(), tool_router: Self::tool_router() }
    }

    fn connect(&self) -> Result<Client, String> {
        match &*self.target {
            Target::Socket(name) => Client::connect(name).map_err(|e| format!("cannot reach Triceratops at {name}: {e}")),
            Target::Discover { dir, launch: exe } => {
                let live = live_sockets_in(dir);
                if !live.is_empty() {
                    // a window is there: use it, or say why not (another version, busy),
                    // but never start a second one beside it
                    let mut first_error = None;
                    for name in &live {
                        match Client::connect(name) {
                            Ok(c) => return Ok(c),
                            Err(e) => {
                                first_error.get_or_insert(e.to_string());
                            }
                        }
                    }
                    return Err(format!("Triceratops is running but cannot be used: {}", first_error.unwrap_or_default()));
                }
                if self.connected_once.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err("Triceratops is not running any more (it was closed, or AI assistants were switched off in Options); start it again to continue".into());
                }
                let Some(exe) = exe else { return Err("Triceratops is not running; start it and try again".into()) };
                launch(exe).map_err(|e| format!("cannot start Triceratops ({}): {e}", exe.display()))?;
                let deadline = Instant::now() + LAUNCH_TIMEOUT;
                while Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(250));
                    if let Some(c) = live_sockets_in(dir).iter().find_map(|n| Client::connect(n).ok()) {
                        return Ok(c);
                    }
                }
                Err("started Triceratops but its window did not come up within 20 s".into())
            }
        }
    }

    /// The connection to the window, connecting (or reconnecting) first when needed. The
    /// lock is held only for this: calls on the connection run concurrently.
    fn client(&self) -> Result<Arc<Client>, String> {
        let mut conn = self.conn.lock().map_err(|_| "internal error: connection lock poisoned".to_string())?;
        if conn.is_none() {
            *conn = Some(Arc::new(self.connect()?));
            self.connected_once.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        conn.clone().ok_or_else(|| "not connected".into())
    }

    /// One request to the window; progress frames of a long one go to `on_progress`.
    fn call_blocking(&self, method: &str, params: Value, on_progress: &mut dyn FnMut(&Value)) -> Result<Value, String> {
        let client = self.client()?;
        let result = client.call_with_progress(method, params, CALL_TIMEOUT, on_progress);
        if matches!(&result, Err(e) if e == "Triceratops was closed") {
            // the next call reconnects, or starts a new window
            if let Ok(mut conn) = self.conn.lock() {
                if conn.as_ref().is_some_and(|c| Arc::ptr_eq(c, &client)) {
                    *conn = None;
                }
            }
        }
        result
    }

    async fn call(&self, method: &'static str, params: Value) -> Result<Value, String> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.call_blocking(method, params, &mut |_| {})).await.map_err(|e| format!("internal error: {e}"))?
    }

    /// `call` for requests that may start a long job: its progress becomes MCP progress
    /// notifications (when the client asked for them), and a client that cancels the tool
    /// call cancels the job in the window.
    async fn call_with_progress(&self, method: &'static str, params: Value, ctx: Option<&RequestContext<RoleServer>>) -> Result<Value, String> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
        let me = self.clone();
        let mut work = tokio::task::spawn_blocking(move || me.call_blocking(method, params, &mut |p| drop(tx.send(p.clone()))));
        let token = ctx.and_then(|c| c.meta.get_progress_token());
        let mut frames = 0.0;
        let mut cancel_sent = false;
        loop {
            tokio::select! {
                r = &mut work => return r.map_err(|e| format!("internal error: {e}"))?,
                Some(p) = rx.recv() => {
                    if let (Some(token), Some(ctx)) = (&token, ctx) {
                        frames += 1.0;
                        let message = progress_message(&p);
                        let _ = ctx.peer.notify_progress(ProgressNotificationParam::new(token.clone(), frames).with_message(message)).await;
                    }
                }
                _ = async { match ctx { Some(c) => c.ct.cancelled().await, None => std::future::pending().await } }, if !cancel_sent => {
                    cancel_sent = true;
                    let _ = self.call("cancel", json!({})).await;
                }
            }
        }
    }

    async fn reply_with_progress(&self, method: &'static str, params: Value, ctx: Option<&RequestContext<RoleServer>>) -> Result<CallToolResult, ErrorData> {
        Ok(match self.call_with_progress(method, params, ctx).await {
            Ok(v) => CallToolResult::success(vec![ContentBlock::text(serde_json::to_string_pretty(&v).unwrap_or_default())]),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e)]),
        })
    }

    /// A JSON reply as text, or the app's error as a tool error the assistant can read.
    async fn run_lines(&self, lines: Vec<String>, ctx: Option<&RequestContext<RoleServer>>) -> Result<CallToolResult, ErrorData> {
        self.reply_with_progress("run_commands", json!({ "lines": lines }), ctx).await
    }

    async fn reply(&self, method: &'static str, params: Value) -> Result<CallToolResult, ErrorData> {
        Ok(match self.call(method, params).await {
            Ok(v) => CallToolResult::success(vec![ContentBlock::text(serde_json::to_string_pretty(&v).unwrap_or_default())]),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e)]),
        })
    }
}

#[tool_router]
impl Bridge {
    #[tool(description = "Run Triceratops command-line entries in the open window, as if typed. Returns the command log, the ids of objects added, changed and deleted, and the prompt still waiting for input (null when the command finished).")]
    async fn run_commands(&self, Parameters(p): Parameters<RunCommandsParams>, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        self.run_lines(p.lines, Some(&ctx)).await
    }

    #[tool(description = "Search Locust's components (the node editor's building blocks) by words and category: name, description, inputs with kinds and defaults, outputs.")]
    async fn locust_components(&self, Parameters(p): Parameters<LocustComponentsParams>) -> Result<CallToolResult, ErrorData> {
        self.reply("locust_components", json!({ "query": p.query, "category": p.category, "limit": p.limit })).await
    }

    #[tool(description = "The current Locust definition as JSON (nodes with uid, type, nickname and values; connections by uid and port index).")]
    async fn locust_get_graph(&self) -> Result<CallToolResult, ErrorData> {
        self.reply("locust_get_graph", json!({})).await
    }

    #[tool(description = "Replace the Locust definition with this one (the user can undo it in Locust), open the editor, solve it and report: errors and warnings per node, panel contents, and a summary (kinds, bounding box, volume or area) of what goes to Preview and Bake. A definition with a mistake is refused with the reason and the canvas stays as it was.")]
    async fn locust_set_graph(&self, Parameters(p): Parameters<LocustSetGraphParams>, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        self.reply_with_progress("locust_set_graph", json!({ "graph": p.graph }), Some(&ctx)).await
    }

    #[tool(description = "Set Locust inputs by name, like moving sliders, then solve and report as locust_set_graph does.")]
    async fn locust_set_values(&self, Parameters(p): Parameters<LocustSetValuesParams>, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        self.reply_with_progress("locust_set_values", json!({ "values": p.values }), Some(&ctx)).await
    }

    #[tool(description = "Solve the Locust definition again (after the model changed) and report.")]
    async fn locust_solve(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        self.reply_with_progress("locust_solve", json!({}), Some(&ctx)).await
    }

    #[tool(description = "Bake: add what the definition sends to its Bake components to the model as ordinary objects (one undo step). Returns the new object ids.")]
    async fn locust_bake(&self) -> Result<CallToolResult, ErrorData> {
        self.reply("locust_bake", json!({})).await
    }

    #[tool(description = "Stop the long operation the window is running for you (a Boolean, a Locust solve), like pressing Esc. Returns whether anything was running.")]
    async fn cancel(&self) -> Result<CallToolResult, ErrorData> {
        self.reply("cancel", json!({})).await
    }

    #[tool(description = "The current prompt and its options, selected object ids, viewports, units and layers.")]
    async fn get_state(&self) -> Result<CallToolResult, ErrorData> {
        self.reply("get_state", json!({})).await
    }

    #[tool(description = "List objects in the model with id, type, layer, name, bounding box and whether they are closed. Filter by type text, layer or ids.")]
    async fn list_objects(&self, Parameters(p): Parameters<ListObjectsParams>) -> Result<CallToolResult, ErrorData> {
        self.reply("list_objects", json!({ "type": p.kind, "layer": p.layer, "ids": p.ids, "limit": p.limit })).await
    }

    #[tool(description = "Details of one object: type, layer, bounding box, and area, volume and centroid where they apply.")]
    async fn object_info(&self, Parameters(p): Parameters<ObjectInfoParams>) -> Result<CallToolResult, ErrorData> {
        self.reply("object_info", json!({ "id": p.id })).await
    }

    #[tool(description = "Search Triceratops' commands by words in their names and descriptions.")]
    async fn find_commands(&self, Parameters(p): Parameters<FindCommandsParams>) -> Result<CallToolResult, ErrorData> {
        self.reply("find_commands", json!({ "query": p.query, "limit": p.limit })).await
    }

    #[tool(description = "Render a viewport of the open window to a PNG image, to look at the model.")]
    async fn capture_view(&self, Parameters(p): Parameters<CaptureViewParams>) -> Result<CallToolResult, ErrorData> {
        match self.call("capture_view", json!({ "viewport": p.viewport, "width": p.width, "height": p.height })).await {
            Ok(v) => Ok(CallToolResult::success(vec![
                ContentBlock::image(v["png_base64"].as_str().unwrap_or_default().to_string(), "image/png"),
                ContentBlock::text(format!("{}×{} px", v["width"], v["height"])),
            ])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e)])),
        }
    }

    #[tool(description = "Undo the last changes to the model, like Ctrl+Z.")]
    async fn undo(&self, Parameters(p): Parameters<StepsParams>) -> Result<CallToolResult, ErrorData> {
        self.reply("undo", json!({ "steps": p.steps })).await
    }

    #[tool(description = "Redo changes that were undone.")]
    async fn redo(&self, Parameters(p): Parameters<StepsParams>) -> Result<CallToolResult, ErrorData> {
        self.reply("redo", json!({ "steps": p.steps })).await
    }
}

#[tool_handler]
impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        let mut info = Implementation::new("triceratops", crate::mcp::protocol::VERSION);
        info.title = Some("Triceratops".into());
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(info).with_instructions(INSTRUCTIONS)
    }
}

/// "BooleanUnion — 12 s · mesh Boolean · 1.3 M" from a progress frame.
fn progress_message(p: &Value) -> String {
    let mut s = format!("{} — {} s", p["label"].as_str().unwrap_or("working"), p["elapsed_s"].as_u64().unwrap_or(0));
    if let Some(stage) = p["stage"].as_str().filter(|s| !s.is_empty()) {
        s.push_str(&format!(" · {stage}"));
    }
    if let Some(n) = p["count"].as_u64().filter(|&n| n > 0) {
        s.push_str(&format!(" · {}", crate::jobs::count_text(n)));
    }
    s
}

/// Entry point of `triceratops mcp`.
pub fn run() -> anyhow::Result<()> {
    let target = match std::env::var("TRICERATOPS_SOCKET") {
        Ok(name) if !name.is_empty() => Target::Socket(name),
        _ => Target::Discover { dir: socket_dir(), launch: std::env::current_exe().ok().map(|exe| launch_path(exe, std::env::var_os("APPIMAGE"))) },
    };
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async move {
        let service = Bridge::new(target).serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        anyhow::Ok(())
    })
}

/// The program to start for a new window. Inside an AppImage, `current_exe` lies in a
/// mount that disappears when the bridge exits, so start the AppImage itself.
fn launch_path(exe: PathBuf, appimage: Option<std::ffi::OsString>) -> PathBuf {
    match appimage {
        Some(a) if !a.is_empty() => PathBuf::from(a),
        _ => exe,
    }
}

/// Start a window that outlives the bridge: no shared stdio (the MCP pipes stay ours) and
/// its own process group, so the assistant client closing does not take it down.
fn launch(exe: &std::path::Path) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(exe);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    launch_cmd(cmd).map(|_| ())
}

/// Spawn `cmd`; returns its process id. A thread waits for it, so when the window exits
/// before the bridge it is reaped instead of lingering as a zombie.
fn launch_cmd(mut cmd: std::process::Command) -> std::io::Result<u32> {
    let mut child = cmd.spawn()?;
    let id = child.id();
    std::thread::Builder::new().name("window-reaper".into()).spawn(move || {
        let _ = child.wait();
    })?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::mcp::host::Host;
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::{CallToolResult, ContentBlock};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tri-bridge-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// A window: a Host served by its own thread, as the event loop would. Stops when dropped.
    struct Window {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Window {
        fn open(dir: &std::path::Path) -> Window {
            let stop = Arc::new(AtomicBool::new(false));
            let (s, dir) = (stop.clone(), dir.to_path_buf());
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || {
                let mut app = App::new();
                let mut host = Host::start_in(&dir, || {}).unwrap();
                ready_tx.send(()).unwrap();
                while !s.load(Ordering::SeqCst) {
                    host.serve(&mut app);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            });
            ready_rx.recv().unwrap();
            Window { stop, thread: Some(thread) }
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn text(r: &CallToolResult) -> String {
        r.content.iter().filter_map(|c| if let ContentBlock::Text(t) = c { Some(t.text.clone()) } else { None }).collect::<Vec<_>>().join("\n")
    }

    /// The window the bridge started is reaped when it exits: no `<defunct>` left behind.
    #[cfg(target_os = "linux")]
    #[test]
    fn launch_reaps_child() {
        let pid = launch_cmd(std::process::Command::new("true")).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok().and_then(|s| s.rsplit(')').next().and_then(|r| r.split_whitespace().next()).map(str::to_string));
        assert_ne!(state.as_deref(), Some("Z"), "child {pid} is a zombie");
    }

    #[test]
    fn lists_the_tools() {
        let b = Bridge::new(Target::Discover { dir: temp_dir("list"), launch: None });
        let mut names: Vec<String> = b.tool_router.list_all().into_iter().map(|t| t.name.to_string()).collect();
        names.sort();
        assert_eq!(names, ["cancel", "capture_view", "find_commands", "get_state", "list_objects", "locust_bake", "locust_components", "locust_get_graph", "locust_set_graph", "locust_set_values", "locust_solve", "object_info", "redo", "run_commands", "undo"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_commands_through_a_window() {
        let dir = temp_dir("run");
        let _w = Window::open(&dir);
        let b = Bridge::new(Target::Discover { dir, launch: None });
        let r = b.run_lines(vec!["Sphere".into(), "0,0,0".into(), "5".into()], None).await.unwrap();
        assert_ne!(r.is_error, Some(true), "{}", text(&r));
        assert!(text(&r).contains("\"added\""), "{}", text(&r));
        let l = b.list_objects(Parameters(ListObjectsParams::default())).await.unwrap();
        // with exact solids (feature kernel-solidcore) a sphere is a solid polysurface
        let want = if cfg!(feature = "kernel-solidcore") { "closed solid polysurface" } else { "closed surface" };
        assert!(text(&l).contains(want), "{}", text(&l));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn closed_window_is_reported_then_a_new_one_is_found() {
        let dir = temp_dir("closed");
        let b = Bridge::new(Target::Discover { dir: dir.clone(), launch: None });
        let w = Window::open(&dir);
        assert_ne!(b.get_state().await.unwrap().is_error, Some(true));
        drop(w);
        let r = b.get_state().await.unwrap();
        assert_eq!(r.is_error, Some(true));
        assert!(text(&r).contains("Triceratops was closed"), "{}", text(&r));
        let _w2 = Window::open(&dir);
        assert_ne!(b.get_state().await.unwrap().is_error, Some(true), "reconnects to the new window");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_window_and_no_launch_is_explained() {
        let b = Bridge::new(Target::Discover { dir: temp_dir("none"), launch: None });
        let r = b.get_state().await.unwrap();
        assert_eq!(r.is_error, Some(true));
        assert!(text(&r).contains("Triceratops is not running"), "{}", text(&r));
    }

    /// Review: a window that refuses (version mismatch) made the bridge start another one.
    #[tokio::test(flavor = "multi_thread")]
    async fn refusing_window_is_reported_not_replaced() {
        let dir = temp_dir("mismatch");
        let _l = crate::mcp::socket::Listener::start_in(&dir, |req: crate::mcp::protocol::Request, tx: std::sync::mpsc::Sender<crate::mcp::protocol::Response>| {
            let _ = tx.send(crate::mcp::protocol::Response::ok(req.id, serde_json::json!({ "version": "0.0.1" })));
        })
        .unwrap();
        let b = Bridge::new(Target::Discover { dir, launch: Some(PathBuf::from("/nonexistent/triceratops")) });
        let r = b.get_state().await.unwrap();
        assert_eq!(r.is_error, Some(true));
        let t = text(&r);
        assert!(t.contains("0.0.1") && !t.contains("cannot start"), "{t}");
    }

    /// Review: after the window closed (or assistants were switched off) the bridge
    /// started a new window.
    #[tokio::test(flavor = "multi_thread")]
    async fn no_relaunch_after_the_window_went_away() {
        let dir = temp_dir("norelaunch");
        let b = Bridge::new(Target::Discover { dir: dir.clone(), launch: Some(PathBuf::from("/nonexistent/triceratops")) });
        let w = Window::open(&dir);
        assert_ne!(b.get_state().await.unwrap().is_error, Some(true));
        drop(w);
        let _ = b.get_state().await.unwrap();
        let t = text(&b.get_state().await.unwrap());
        assert!(!t.contains("cannot start"), "must not launch: {t}");
        assert!(t.contains("not running"), "{t}");
    }

    #[test]
    fn launches_the_appimage_rather_than_its_mount() {
        let exe = PathBuf::from("/tmp/.mount_abc/usr/bin/triceratops");
        assert_eq!(launch_path(exe.clone(), Some("/home/u/Triceratops.AppImage".into())), PathBuf::from("/home/u/Triceratops.AppImage"));
        assert_eq!(launch_path(exe.clone(), None), exe);
    }
}
