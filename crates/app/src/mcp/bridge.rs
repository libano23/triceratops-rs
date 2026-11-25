//! `triceratops mcp`: an MCP server on stdin/stdout that forwards each tool call to the
//! running Triceratops window over the local socket, starting a window when none runs.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ErrorData, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{tool, tool_handler, tool_router, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::socket::{live_sockets_in, socket_dir, Client};

/// How long one tool call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long to wait for a window started by the bridge to appear.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(20);

const INSTRUCTIONS: &str = "Triceratops is a NURBS 3D modeller. You drive the window the user has open; they watch \
every change and can undo it.\n\
\n\
Model with run_commands: each line is what a person would type on Triceratops' command line.\n\
- A command name starts a command (Box, Cylinder, Sphere, Line, Polyline, InterpCrv, ExtrudeCrv, Revolve, Loft, \
BooleanUnion, BooleanDifference, Move, Copy, Rotate, Mirror, FilletEdge, Delete, Undo, ...). Use find_commands to \
discover more by what they do.\n\
- Then answer its prompts in order: points as x,y,z (document units, usually millimetres), numbers, option names, \
or objects as #id, #id1,#id2 or last (the objects the last command made). #id@x,y,z picks an object at a point. \
Every command you start begins with nothing selected, so always name the objects it works on.\n\
- An empty string \"\" presses Enter: it finishes point lists (Polyline, InterpCrv) and accepts defaults.\n\
- The reply lists the log, the ids added, changed and deleted, and the prompt still waiting, if any. A waiting \
prompt means the command wants more input: continue it with another run_commands call.\n\
\n\
Check your work with list_objects, object_info (volume, area) and capture_view, which returns an image of a \
viewport (Perspective, Top, Front, Right). Commands that need a file (Open, Save, Export) are not available. \
If the user is typing a command, requests are refused until they finish.";

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
    conn: Arc<Mutex<Option<Client>>>,
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
pub struct StepsParams {
    /// How many steps (default 1).
    pub steps: Option<u32>,
}

impl Bridge {
    pub fn new(target: Target) -> Self {
        Self { target: Arc::new(target), conn: Arc::new(Mutex::new(None)), tool_router: Self::tool_router() }
    }

    fn connect(&self) -> Result<Client, String> {
        match &*self.target {
            Target::Socket(name) => Client::connect(name).map_err(|e| format!("cannot reach Triceratops at {name}: {e}")),
            Target::Discover { dir, launch } => {
                if let Some(c) = live_sockets_in(dir).iter().find_map(|n| Client::connect(n).ok()) {
                    return Ok(c);
                }
                let Some(exe) = launch else { return Err("Triceratops is not running; start it and try again".into()) };
                std::process::Command::new(exe).spawn().map_err(|e| format!("cannot start Triceratops ({}): {e}", exe.display()))?;
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

    /// One request to the window, connecting (or reconnecting) first when needed.
    fn call_blocking(&self, method: &str, params: Value) -> Result<Value, String> {
        let mut conn = self.conn.lock().map_err(|_| "internal error: connection lock poisoned".to_string())?;
        if conn.is_none() {
            *conn = Some(self.connect()?);
        }
        let result = conn.as_mut().map(|c| c.call(method, params, CALL_TIMEOUT)).unwrap_or_else(|| Err("not connected".into()));
        if matches!(&result, Err(e) if e == "Triceratops was closed") {
            *conn = None; // the next call reconnects, or starts a new window
        }
        result
    }

    async fn call(&self, method: &'static str, params: Value) -> Result<Value, String> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.call_blocking(method, params)).await.map_err(|e| format!("internal error: {e}"))?
    }

    /// A JSON reply as text, or the app's error as a tool error the assistant can read.
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
    async fn run_commands(&self, Parameters(p): Parameters<RunCommandsParams>) -> Result<CallToolResult, ErrorData> {
        self.reply("run_commands", json!({ "lines": p.lines })).await
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

/// Entry point of `triceratops mcp`.
pub fn run() -> anyhow::Result<()> {
    let target = match std::env::var("TRICERATOPS_SOCKET") {
        Ok(name) if !name.is_empty() => Target::Socket(name),
        _ => Target::Discover { dir: socket_dir(), launch: std::env::current_exe().ok() },
    };
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async move {
        let service = Bridge::new(target).serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        anyhow::Ok(())
    })
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

    #[test]
    fn lists_the_eight_tools() {
        let b = Bridge::new(Target::Discover { dir: temp_dir("list"), launch: None });
        let mut names: Vec<String> = b.tool_router.list_all().into_iter().map(|t| t.name.to_string()).collect();
        names.sort();
        assert_eq!(names, ["capture_view", "find_commands", "get_state", "list_objects", "object_info", "redo", "run_commands", "undo"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_commands_through_a_window() {
        let dir = temp_dir("run");
        let _w = Window::open(&dir);
        let b = Bridge::new(Target::Discover { dir, launch: None });
        let r = b.run_commands(Parameters(RunCommandsParams { lines: vec!["Sphere".into(), "0,0,0".into(), "5".into()] })).await.unwrap();
        assert_ne!(r.is_error, Some(true), "{}", text(&r));
        assert!(text(&r).contains("\"added\""), "{}", text(&r));
        let l = b.list_objects(Parameters(ListObjectsParams::default())).await.unwrap();
        assert!(text(&l).contains("closed surface"), "{}", text(&l));
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
}
