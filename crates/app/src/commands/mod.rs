//! Triceratops-style interactive command system.
//!
//! A command is a small state machine: it is started, returns a prompt (with
//! optional command-line options), receives inputs and eventually finishes.
//! Prompt wording and option names follow Triceratops as documented in
//! docs/ui-reference.md.

pub mod create;
pub mod curve2;
pub mod curve_edit;
pub mod edit;
pub mod misc;
pub mod misc2;
pub mod misc3;
pub mod misc4;
pub mod misc5;
pub mod misc6;
pub mod misc7;
pub mod misc8;
pub mod history_cmds;
pub mod parametric_cmds;
pub mod annotate;
pub mod blocks;
pub mod catalog;
pub mod cplane_cmds;
pub mod curves3;
pub mod surfaces3;
pub mod ui_cmds;
pub mod mesh3;
pub mod layouts;

pub use curves3::plane_curve_points as curves3_plane_points;
pub mod registry;
pub mod render_cmds;
pub mod simple;
pub mod solid_edit;
pub mod mesh2;
pub mod mesh_cmds;
pub mod surface2;
pub mod surface_edit;
pub mod transform;

#[cfg(test)]
mod flow_tests;

use std::collections::HashMap;

use glam::DVec3;
use tri_doc::{Document, ObjectId};
use tri_geom::{Brep, NurbsCurve, NurbsSurface, Plane};

/// A mesh Boolean for the commands: "nothing came out" is an empty mesh, as it always was;
/// a Boolean over budget or cancelled is an error whose text goes to the log.
pub fn mesh_bool(a: &tri_geom::Mesh, b: &tri_geom::Mesh, op: tri_geom::BooleanOp) -> Result<tri_geom::Mesh, tri_geom::BooleanError> {
    match tri_geom::mesh_boolean(a, b, op) {
        Err(tri_geom::BooleanError::Empty) => Ok(tri_geom::Mesh::default()),
        r => r,
    }
}

pub use registry::{all_commands, command_info, find_command, is_implemented, lookup};
#[allow(unused_imports)]
pub use registry::CommandInfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjFilter {
    Any,
    Curve,
    /// Surfaces and polysurfaces.
    Surface,
    Polysurface,
    Point,
    Mesh,
}

impl ObjFilter {
    pub fn accepts(&self, g: &tri_doc::Geometry) -> bool {
        use tri_doc::Geometry::*;
        match self {
            ObjFilter::Any => true,
            ObjFilter::Curve => matches!(g, Curve(_)),
            ObjFilter::Surface => matches!(g, Surface(_) | Brep(_)),
            ObjFilter::Polysurface => matches!(g, Brep(_)),
            ObjFilter::Point => matches!(g, Point(_)),
            ObjFilter::Mesh => matches!(g, Mesh(_)),
        }
    }
}

/// A command-line option shown in parentheses after the prompt.
#[derive(Clone, Debug, PartialEq)]
pub struct Opt {
    pub name: String,
    pub value: Option<String>,
}

pub fn opt(name: &str) -> Opt {
    Opt { name: name.to_string(), value: None }
}

pub fn optv(name: &str, value: impl ToString) -> Opt {
    Opt { name: name.to_string(), value: Some(value.to_string()) }
}

pub fn yesno(v: bool) -> &'static str {
    if v {
        "Yes"
    } else {
        "No"
    }
}

impl Opt {
    pub fn label(&self) -> String {
        match &self.value {
            Some(v) => format!("{}={}", self.name, v),
            None => self.name.clone(),
        }
    }

    /// Letters a user may type as a shortcut: the upper-case letters and digits of the name.
    pub fn shortcut(&self) -> String {
        self.name.chars().filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit()).collect::<String>().to_ascii_lowercase()
    }
}

/// Find the option matching typed text (exact name, shortcut letters, or a unique prefix).
pub fn match_option<'a>(options: &'a [Opt], typed: &str) -> Option<&'a Opt> {
    let t = typed.trim().to_ascii_lowercase();
    if t.is_empty() {
        return None;
    }
    if let Some(o) = options.iter().find(|o| o.name.eq_ignore_ascii_case(&t)) {
        return Some(o);
    }
    if let Some(o) = options.iter().find(|o| o.shortcut() == t) {
        return Some(o);
    }
    let prefix: Vec<&Opt> = options.iter().filter(|o| o.name.to_ascii_lowercase().starts_with(&t)).collect();
    if prefix.len() == 1 {
        return Some(prefix[0]);
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub enum PromptKind {
    Point,
    /// A point, or a typed number (angle, factor, distance) delivered as `Input::Number`.
    PointOrNumber,
    Objects { filter: ObjFilter, min: usize },
    /// Pick one object with the mouse; the input carries the picked location.
    ObjectPick { filter: ObjFilter },
    Number,
    Text,
    /// Only options can be chosen (Enter accepts the defaults).
    Options,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub kind: PromptKind,
    pub msg: String,
    pub options: Vec<Opt>,
    pub default: Option<String>,
}

impl Prompt {
    pub fn new(kind: PromptKind, msg: impl Into<String>) -> Self {
        Self { kind, msg: msg.into(), options: Vec::new(), default: None }
    }
    pub fn point(msg: impl Into<String>) -> Self {
        Self::new(PromptKind::Point, msg)
    }
    pub fn point_or_number(msg: impl Into<String>) -> Self {
        Self::new(PromptKind::PointOrNumber, msg)
    }
    pub fn objects(msg: impl Into<String>, filter: ObjFilter, min: usize) -> Self {
        Self::new(PromptKind::Objects { filter, min }, msg)
    }
    pub fn pick(msg: impl Into<String>, filter: ObjFilter) -> Self {
        Self::new(PromptKind::ObjectPick { filter }, msg)
    }
    pub fn number(msg: impl Into<String>) -> Self {
        Self::new(PromptKind::Number, msg)
    }
    pub fn text(msg: impl Into<String>) -> Self {
        Self::new(PromptKind::Text, msg)
    }
    pub fn options(msg: impl Into<String>) -> Self {
        Self::new(PromptKind::Options, msg)
    }
    pub fn opts(mut self, options: impl IntoIterator<Item = Opt>) -> Self {
        self.options = options.into_iter().collect();
        self
    }
    pub fn default(mut self, d: impl ToString) -> Self {
        self.default = Some(d.to_string());
        self
    }
    pub fn default_num(mut self, d: f64) -> Self {
        self.default = Some(fmt_num(d));
        self
    }
    pub fn step(self) -> Step {
        Step::Prompt(self)
    }

    /// Text as displayed on the command line: `msg <default> ( A  B=Yes )`.
    pub fn display(&self) -> String {
        let mut s = self.msg.clone();
        if let Some(d) = &self.default {
            s.push_str(&format!(" <{d}>"));
        }
        if !self.options.is_empty() {
            s.push_str(" ( ");
            s.push_str(&self.options.iter().map(|o| o.label()).collect::<Vec<_>>().join("  "));
            s.push_str(" )");
        }
        s
    }

    pub fn wants_point(&self) -> bool {
        matches!(self.kind, PromptKind::Point | PromptKind::PointOrNumber)
    }

    pub fn wants_objects(&self) -> bool {
        matches!(self.kind, PromptKind::Objects { .. })
    }

    pub fn wants_pick(&self) -> bool {
        matches!(self.kind, PromptKind::ObjectPick { .. })
    }
}

impl From<Prompt> for Step {
    fn from(p: Prompt) -> Self {
        Step::Prompt(p)
    }
}

pub fn fmt_num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Point(DVec3),
    Objects(Vec<ObjectId>),
    /// An object picked with the mouse and the location of the pick on it.
    Pick(ObjectId, DVec3),
    Number(f64),
    Text(String),
    /// A command-line option was clicked or typed.
    Option(String),
    /// Enter pressed with no value.
    Empty,
    Cancel,
}

pub enum Step {
    Prompt(Prompt),
    Done,
    Cancelled,
    /// Long work started on a worker thread (`CmdCtx::work`); the command waits for it.
    Work(Job),
}

type WorkResult = Box<dyn std::any::Any + Send>;
type ApplyFn = Box<dyn for<'c> FnOnce(WorkResult, &mut CmdCtx<'c>) -> Step>;

/// A computation running on a worker thread for a command, and what to do with its result.
pub struct Job {
    pub label: String,
    pub token: tri_geom::job::Token,
    pub started: std::time::Instant,
    /// Why it was cancelled ("cancelled", "cancelled by the user"), once it was.
    pub cancelled: Option<String>,
    /// The command changed the document (in an undo group) before starting the job: a
    /// cancelled job undoes that group, so nothing is left half done.
    pub undo_on_cancel: bool,
    handle: std::thread::JoinHandle<WorkResult>,
    apply: ApplyFn,
}

impl Job {
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

/// Requests that commands send to the application (things outside the document).
#[derive(Clone, Debug, PartialEq)]
pub enum AppRequest {
    /// Open the Locust node editor.
    Locust,
    ZoomExtents { all: bool, selected: bool },
    ZoomWindow,
    SetView(String),
    Layout(String),
    MaxViewport,
    SetDisplayMode(String),
    ToggleSnap(String),
    FileNew,
    FileOpen,
    FileSave,
    FileSaveAs,
    Import,
    Export,
    Quit,
    Help,
    ShowPanel(String),
    CommandHistory,
    Options(String),
    DocumentProperties(String),
    UndoView,
    RedoView,
    LoftDialog(Vec<ObjectId>),
    Message(String),
    PointsOn(Vec<ObjectId>),
    PointsOff,
    CopyToClipboard { cut: bool },
    Paste,
    NamedView { name: String, restore: bool },
    ZoomWindowPoints(DVec3, DVec3),
    PlanView,
    NextViewport(bool),
    /// Set the active viewport's construction plane (None = previous).
    SetCPlane(Option<Plane>),
    CPlaneToView,
    /// Camera operation on the active viewport: RotateView (degrees), Pan (fraction of view), Zoom (factor in dx).
    CameraOp { op: String, dx: f64, dy: f64 },
    /// SaveACopy (dialog, keeps the document path) or IncrementalSave (numbered copy next to the file).
    SaveCopy { incremental: bool },
    /// Reload the document from its file.
    Revert,
    /// Toggle a UI element: "right_sidebar", "viewport_tabs".
    ToggleUi(String),
    /// Register a planar surface object as a clipping plane.
    AddClippingPlane(ObjectId),
    /// Enable/disable (or flip) clipping planes; empty ids = all.
    SetClippingPlanes { ids: Vec<ObjectId>, enabled: bool, flip: bool },
    /// Read a text file and run each line as a command-line entry.
    RunCommandFile(String),
    /// Generic option: "ortho_angle", "fov", "lens", "snap", "ortho", "planar", "active_viewport", "gumball", "osnap", "draw_order" ...
    SetOption { key: String, value: String },
    /// Export objects translated so that `origin` becomes the world origin.
    ExportWithOrigin { ids: Vec<ObjectId>, origin: DVec3 },
    /// Grip (control point) selection operations: SelControlPoint, InvertPt, SelU, SelV, SelUV, NextU, PrevU, NextV, PrevV, AddNextU, ..., SelConnected, HidePt, ShowPt.
    GripOp(String),
    /// Restrict picking to an object kind (None = all).
    SelectionFilter(Option<String>),
    /// Window operations: "minimize", "fullscreen", "maximize".
    WindowOp(String),
    /// Save a backup copy of the document in the options folder.
    Autosave,
    /// Export (true) or import (false) the options file through a dialog.
    OptionsFile { export: bool },
    /// Repeat the last command.
    RepeatLast,
    /// Write each block definition to its own .3dm file (folder dialog).
    ExportBlocks,
    /// Check a .3dm file chosen with a dialog and report its contents.
    Audit3dmFile,
    /// Save the document as a template in the options folder.
    SaveTemplate,
    /// Capture the active viewport to a PNG file (dialog when the path is empty; "screen:" prefix captures the whole window).
    ViewCapture { path: String, clipboard: bool },
    /// Render every named view to PNG files in a folder.
    BatchRenderNamedViews(String),
    /// Sun light: Some((azimuth, altitude)) in degrees, or None to turn it off.
    Sun(Option<(f64, f64)>),
    /// Ground plane at a height, or None to turn it off.
    GroundPlane(Option<f64>),
    /// Turntable animation: number of frames and an optional folder to record PNG frames into.
    Turntable { frames: usize, record: Option<String> },
    /// Reload texture files from disk.
    RefreshTextures,
    /// Gradient background for the active viewport (top, bottom) or None.
    Gradient(Option<([u8; 3], [u8; 3])>),
    /// Point the active camera from one point towards another.
    LookAt { from: DVec3, to: DVec3 },
    /// Replay history records (children in the list, or all when empty).
    HistoryUpdate(Vec<ObjectId>),
    /// Viewport management: "new", "close", "split_h", "split_v", "float", "to_top", "to_back", "swap".
    ViewportOp(String),
    /// Animation setup: kind = "turntable" | "path" | "fly" | "sun_day" | "sun_season".
    AnimSetup { kind: String, frames: usize, path: Vec<DVec3>, targets: Vec<DVec3>, record: Option<String> },
    /// Animation control: "play", "record", "first", "last", "next", "prev", "frame" (with index).
    AnimControl(String, i64),
    /// Add a guide (two points), remove the guide nearest a point (one point) or clear all (none).
    Guide(Option<DVec3>, Option<DVec3>),
    /// Move the camera target to the selected objects.
    LookAtTarget,
    /// Gumball options: "relocate" (points), "alignment"/"scale_mode"/"auto_reset" (value), "reset".
    GumballOp { op: String, points: Vec<DVec3>, value: String },
    /// Layer states: "save", "restore", "delete", "list", "book_next", "book_previous", "book_first", "book_all".
    LayerState { op: String, name: String },
    /// Named selections: "save", "restore", "delete", "list".
    NamedSelection { op: String, name: String },
    /// Construction plane operations: "copy_all", "sync", "auto_align", "one_view", "mplane_attach:<id>", "mplane_detach", "mplane_refresh".
    CPlaneOp(String),
    /// Read (save = false) or write (save = true) the viewport cameras as JSON.
    ViewportsFile { path: String, save: bool },
    /// Worksession: "attach" (path), "detach", "list".
    Worksession { op: String, path: String },
    /// Run command lines as if typed.
    RunLines(Vec<String>),
    /// Run a python script and feed its output to the command line.
    RunPython(String),
    /// Export objects to a file chosen by extension (.3dm, .dxf, .obj).
    ExportSelected { ids: Vec<ObjectId>, path: String },
    /// Save a named construction plane.
    NamedCPlane { name: String, plane: Plane },
    /// Save a named view looking along a section plane.
    NamedSectionView { name: String, plane: Plane },
    /// Page layout operations (see `App::layout_op`).
    LayoutOp { op: String, name: String },
    /// Theme operations (see `App::theme_op`).
    ThemeOp(String),
    /// Window layout files: "save", "load", "reset", "reset_all" (empty path opens a dialog).
    LayoutFile { op: String, path: String },
}

pub struct CmdCtx<'a> {
    pub doc: &'a mut Document,
    pub log: &'a mut Vec<String>,
    pub cplane: Plane,
    pub view_dir: DVec3,
    /// Camera location of the active viewport.
    pub view_eye: DVec3,
    pub requests: &'a mut Vec<AppRequest>,
    /// Values remembered between runs (last radius, option states, ...).
    pub memory: &'a mut HashMap<String, String>,
}

impl CmdCtx<'_> {
    /// Run `compute` on a worker thread (cancellable, with progress) and, when it ends, hand
    /// its result to `apply` on the GUI thread, where the document may be changed. The
    /// command waits meanwhile; Esc cancels it and leaves the document as it was.
    pub fn work<T, F, A>(&mut self, label: impl Into<String>, compute: F, apply: A) -> Step
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
        A: FnOnce(T, &mut CmdCtx) -> Step + 'static,
    {
        let label = label.into();
        let token = tri_geom::job::Token::new();
        let worker_token = token.clone();
        let handle = std::thread::Builder::new()
            .name(format!("job: {label}"))
            .spawn(move || {
                let _guard = tri_geom::job::install(worker_token);
                Box::new(compute()) as WorkResult
            })
            .expect("spawn job thread");
        let apply: ApplyFn = Box::new(move |r: WorkResult, ctx: &mut CmdCtx| match r.downcast::<T>() {
            Ok(v) => apply(*v, ctx),
            Err(_) => Step::Cancelled,
        });
        Step::Work(Job { label, token, started: std::time::Instant::now(), cancelled: None, undo_on_cancel: false, handle, apply })
    }

    /// `work` for a computation that can fail: an error is logged as "<label> failed: <e>."
    /// and the command ends; a result goes to `apply`.
    pub fn try_work<T, F, A>(&mut self, label: &'static str, compute: F, apply: A) -> Step
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
        A: FnOnce(T, &mut CmdCtx) -> Step + 'static,
    {
        self.work(label, compute, move |r, ctx| match r {
            Ok(v) => apply(v, ctx),
            Err(e) => {
                ctx.log(format!("{label} failed: {e}."));
                Step::Cancelled
            }
        })
    }

    /// `work` for a command that already changed the document in an undo group: if the job
    /// is cancelled, that group is undone.
    pub fn work_after_edit<T, F, A>(&mut self, label: impl Into<String>, compute: F, apply: A) -> Step
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
        A: FnOnce(T, &mut CmdCtx) -> Step + 'static,
    {
        match self.work(label, compute, apply) {
            Step::Work(mut j) => {
                j.undo_on_cancel = true;
                Step::Work(j)
            }
            other => other,
        }
    }

    pub fn log(&mut self, msg: impl Into<String>) {
        self.log.push(msg.into());
    }
    pub fn mem_num(&self, key: &str, default: f64) -> f64 {
        self.memory.get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }
    pub fn mem_set(&mut self, key: &str, v: impl ToString) {
        self.memory.insert(key.to_string(), v.to_string());
    }
    pub fn mem_bool(&self, key: &str, default: bool) -> bool {
        self.memory.get(key).map_or(default, |v| v == "Yes" || v == "true")
    }
}

/// Temporary geometry drawn while a command is running (feedback colour).
#[derive(Clone, Debug)]
pub enum Preview {
    Line(DVec3, DVec3),
    Polyline(Vec<DVec3>),
    Curve(NurbsCurve),
    Surface(NurbsSurface),
    Brep(Brep),
    Point(DVec3),
}

pub trait Command {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step;
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step;
    /// Dynamic feedback given the current cursor position (already snapped).
    fn preview(&self, _cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        Vec::new()
    }
    /// Reference point for relative coordinates, ortho, distance input and the status bar delta.
    fn base_point(&self) -> Option<DVec3> {
        None
    }
}

/// Drives the active command, keeps the current prompt and echoes prompts to history.
#[derive(Default)]
pub struct CommandRunner {
    active: Option<(String, Box<dyn Command>)>,
    prompt: Option<Prompt>,
    pub last_command: Option<String>,
    /// Text typed for the current prompt (echoed to history with the prompt).
    answer: String,
    /// Inputs fed to the active command (for history recording).
    recorded: Vec<Input>,
    /// The last command that finished successfully and the inputs it received.
    pub finished: Option<(String, Vec<Input>)>,
    /// Work the active command is waiting for.
    job: Option<Job>,
}

impl CommandRunner {
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// The job the active command waits for.
    pub fn job(&self) -> Option<&Job> {
        self.job.as_ref()
    }

    /// Ask the running job to stop (`reason` goes to the log when it has). False when no job runs.
    pub fn cancel_job(&mut self, reason: &str) -> bool {
        let Some(j) = self.job.as_mut() else { return false };
        j.token.cancel();
        j.cancelled.get_or_insert_with(|| reason.to_string());
        true
    }

    /// When the job has ended, apply its result (or the cancellation). True when it ended.
    pub fn poll_job(&mut self, ctx: &mut CmdCtx) -> bool {
        if !self.job.as_ref().is_some_and(|j| j.is_finished()) {
            return false;
        }
        self.finish_job(ctx);
        true
    }

    /// Block until the job ends and apply it (history replay, which is not interactive).
    pub fn wait_job(&mut self, ctx: &mut CmdCtx) {
        while self.job.is_some() {
            if !self.poll_job(ctx) {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }

    fn finish_job(&mut self, ctx: &mut CmdCtx) {
        let Some(job) = self.job.take() else { return };
        let Job { label, cancelled, handle, apply, undo_on_cancel, .. } = job;
        let result = handle.join();
        match (cancelled, result) {
            (Some(reason), _) => {
                ctx.log(format!("{label} {reason}"));
                if undo_on_cancel {
                    ctx.doc.undo();
                }
                self.cancel(ctx);
            }
            (None, Err(_)) => {
                ctx.log(format!("{label} failed (internal error)."));
                self.cancel(ctx);
            }
            (None, Ok(r)) => {
                let step = apply(r, ctx);
                self.finish_step(step, ctx);
            }
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.active.as_ref().map(|(n, _)| n.as_str())
    }

    pub fn prompt(&self) -> Option<&Prompt> {
        if self.active.is_some() {
            self.prompt.as_ref()
        } else {
            None
        }
    }

    pub fn base_point(&self) -> Option<DVec3> {
        self.active.as_ref().and_then(|(_, c)| c.base_point())
    }

    pub fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        self.active.as_ref().map_or_else(Vec::new, |(_, c)| c.preview(cursor, doc))
    }

    fn echo_prompt(&mut self, ctx: &mut CmdCtx) {
        if let Some(p) = self.prompt.take() {
            let a = std::mem::take(&mut self.answer);
            ctx.log(format!("{}: {}", p.display(), a));
        }
    }

    /// Start a command by canonical name (as typed or from a button; `typed` is echoed).
    pub fn start(&mut self, name: &str, typed: &str, ctx: &mut CmdCtx, preselected: &[ObjectId]) {
        if let Some(j) = &self.job {
            ctx.log(format!("{} is running — wait for it or press Esc to cancel it.", j.label));
            return;
        }
        if self.active.is_some() {
            self.cancel(ctx);
        }
        ctx.log(format!("Command: {typed}"));
        let Some(mut cmd) = registry::create_command(name) else {
            ctx.log(format!("The {name} command is not implemented in triceratops yet."));
            return;
        };
        self.last_command = Some(name.to_string());
        self.recorded.clear();
        self.finished = None;
        let step = cmd.start(ctx);
        self.active = Some((name.to_string(), cmd));
        self.apply_step(step, ctx);
        // preselection feeds a first objects prompt
        if let Some(Prompt { kind: PromptKind::Objects { filter, min }, .. }) = self.prompt.clone() {
            let sel: Vec<ObjectId> = preselected
                .iter()
                .copied()
                .filter(|id| ctx.doc.get(*id).is_some_and(|o| filter.accepts(&o.geometry)))
                .collect();
            if !sel.is_empty() && sel.len() >= min {
                self.feed(Input::Objects(sel), ctx);
            }
        }
    }

    pub fn feed(&mut self, input: Input, ctx: &mut CmdCtx) {
        let Some((_, cmd)) = self.active.as_mut() else {
            return;
        };
        if matches!(input, Input::Cancel) {
            self.cancel(ctx);
            return;
        }
        if self.job.is_some() {
            // the command is waiting for its work: only Esc (above) reaches it
            return;
        }
        self.recorded.push(input.clone());
        let step = cmd.input(input, ctx);
        self.finish_step(step, ctx);
    }

    fn finish_step(&mut self, step: Step, ctx: &mut CmdCtx) {
        let done = matches!(step, Step::Done);
        let name = self.active.as_ref().map(|(n, _)| n.clone());
        self.apply_step(step, ctx);
        if done {
            if let Some(n) = name {
                self.finished = Some((n, std::mem::take(&mut self.recorded)));
            }
        }
    }

    /// Feed an option; if a value was typed as `Name=Value` and the command
    /// answers with a number/text prompt, feed the value straight away.
    pub fn feed_option(&mut self, name: &str, value: Option<&str>, ctx: &mut CmdCtx) {
        self.answer = match value {
            Some(v) => format!("{name}={v}"),
            None => name.to_string(),
        };
        self.feed(Input::Option(name.to_string()), ctx);
        if let (Some(v), Some(p)) = (value, self.prompt.clone()) {
            match p.kind {
                PromptKind::Number | PromptKind::PointOrNumber => {
                    if let Some(n) = crate::input::parse_number(v) {
                        self.answer = v.to_string();
                        self.feed(Input::Number(n), ctx);
                    }
                }
                PromptKind::Text => {
                    self.answer = v.to_string();
                    self.feed(Input::Text(v.to_string()), ctx);
                }
                PromptKind::Options => {
                    if let Some(o) = match_option(&p.options, v) {
                        let n = o.name.clone();
                        self.answer = v.to_string();
                        self.feed(Input::Option(n), ctx);
                    }
                }
                _ => {}
            }
        }
    }

    pub fn set_answer(&mut self, text: &str) {
        self.answer = text.to_string();
    }

    pub fn cancel(&mut self, ctx: &mut CmdCtx) {
        self.cancel_with(ctx, "cancelled");
    }

    /// Cancel the active command; a running job is asked to stop and the command ends when
    /// it has (`poll_job`), logging `<label> <reason>`.
    pub fn cancel_with(&mut self, ctx: &mut CmdCtx, reason: &str) {
        if self.cancel_job(reason) {
            return;
        }
        if let Some((_, mut cmd)) = self.active.take() {
            let _ = cmd.input(Input::Cancel, ctx);
            self.echo_prompt(ctx);
            ctx.log("Command: ");
        }
        self.prompt = None;
        self.answer.clear();
    }

    fn apply_step(&mut self, step: Step, ctx: &mut CmdCtx) {
        self.echo_prompt(ctx);
        match step {
            Step::Prompt(p) => self.prompt = Some(p),
            Step::Done => {
                self.active = None;
                self.prompt = None;
            }
            Step::Cancelled => {
                self.active = None;
                self.prompt = None;
            }
            Step::Work(job) => {
                self.prompt = None;
                self.job = Some(job);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_matching() {
        let opts = vec![opt("BothSides"), optv("Copy", "Yes"), opt("3Point"), opt("PersistentClose")];
        assert_eq!(match_option(&opts, "copy").unwrap().name, "Copy");
        assert_eq!(match_option(&opts, "bs").unwrap().name, "BothSides");
        assert_eq!(match_option(&opts, "3p").unwrap().name, "3Point");
        assert_eq!(match_option(&opts, "pers").unwrap().name, "PersistentClose");
        assert!(match_option(&opts, "zz").is_none());
    }

    #[test]
    fn prompt_display() {
        let p = Prompt::point("Radius").default_num(1.0).opts([opt("Diameter"), optv("Copy", "No")]);
        assert_eq!(p.display(), "Radius <1> ( Diameter  Copy=No )");
    }
}
